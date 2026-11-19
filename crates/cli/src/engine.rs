use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use futures_util::StreamExt;
use http::HeaderMap;
use ocrouter_core::config::{Config, Routing};
use ocrouter_core::events::{EventBus, EventRecord, Level};
use ocrouter_core::keys::KeyFile;
use ocrouter_core::models::{Catalog, Lane};
use ocrouter_core::provider::{upstream_for, Provider};
use ocrouter_core::proxy::decision::{choose_lane, classify, Attempt, LaneChoice, ResponseSignal};
use ocrouter_core::proxy::headers::{build_upstream_headers, generate_session_id};
use ocrouter_core::proxy::routing::{build_upstream_url, strip_lane_prefix};
use ocrouter_core::proxy::usage::{parse_usage, parse_usage_from_sse, ApiShape};
use ocrouter_core::router::allowance::Allowance;
use ocrouter_core::router::circuit_breaker::CircuitBreaker;
use ocrouter_core::router::key_manager::KeyManager;
use ocrouter_core::router::session_affinity::SessionAffinity;
use ocrouter_core::router::types::KeySelection;
use ocrouter_core::router::types::RoutingStrategy;
use ocrouter_core::state::{KeyHealth, OrgUsage, RouterState};

use crate::body::{ByteStream, ProxyBody};

#[derive(Debug, Clone)]
pub struct ProxyRequest {
    pub method: String,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub struct ProxyResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: ProxyBody,
}

/// The transport seam. The real implementation is `http_client::ReqwestClient`;
/// tests drive the pipeline with a scripted fake.
pub trait UpstreamClient {
    fn send(
        &self,
        request: ProxyRequest,
    ) -> impl std::future::Future<Output = Result<ProxyResponse, String>>;
}

#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub method: String,
    pub path: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub struct ProxyOutcome {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: ProxyBody,
}

/// Shared routing state. The control API and the proxy hold clones of the same
/// `Arc`s, so a key added through the API is immediately routable.
#[derive(Clone)]
pub struct Shared {
    pub config: Arc<Config>,
    pub keys: Arc<Mutex<KeyManager>>,
    pub breaker: Arc<Mutex<CircuitBreaker>>,
    pub allowance: Arc<Mutex<Allowance>>,
    /// Swapped wholesale by the background refresh; readers clone the `Arc`.
    pub catalog: Arc<RwLock<Arc<Catalog>>>,
    pub events: EventBus,
    pub affinity: Arc<Mutex<SessionAffinity>>,
    pub strategy: Arc<Mutex<RoutingStrategy>>,
    pub routing: Arc<Mutex<Routing>>,
}

/// One upstream to try. A route step pins a key and the model id to send.
struct Target {
    provider: Provider,
    key: Option<String>,
    model: Option<String>,
}

impl Target {
    fn lane(provider: Provider) -> Self {
        Target {
            provider,
            key: None,
            model: None,
        }
    }
}

impl Shared {
    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog
            .read()
            .map(|c| Arc::clone(&c))
            .unwrap_or_else(|e| Arc::clone(&e.into_inner()))
    }

    pub fn set_catalog(&self, catalog: Catalog) {
        if let Ok(mut slot) = self.catalog.write() {
            *slot = Arc::new(catalog);
        }
    }

    /// Key health and per-org allowance, for `state.json`. Carries no secret.
    pub fn health_snapshot(&self) -> RouterState {
        let keys = self
            .keys
            .lock()
            .map(|keys| {
                keys.all()
                    .iter()
                    .map(|k| KeyHealth {
                        id: k.id.clone(),
                        provider: k.provider,
                        label: k.label.clone(),
                        status: k.status,
                        consecutive_errors: k.consecutive_errors,
                        cooldown_until_ms: k.cooldown_until_ms,
                        tokens_today: 0,
                        cost_accumulated: 0.0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let allowance = self
            .allowance
            .lock()
            .map(|a| {
                a.snapshot()
                    .into_iter()
                    .map(|(org, used, limit)| OrgUsage { org, used, limit })
                    .collect()
            })
            .unwrap_or_default();
        RouterState {
            keys,
            allowance,
            recent_events: Vec::new(),
            saved_at_ms: 0,
        }
    }

    /// Re-apply a saved snapshot so a restart keeps cooldowns and today's
    /// allowance usage. Keys no longer in the pool are ignored.
    pub fn restore(&self, state: &RouterState, now_ms: u64) {
        if let Ok(mut keys) = self.keys.lock() {
            for key in keys.iter_mut() {
                if let Some(saved) = state.keys.iter().find(|h| h.id == key.id) {
                    key.status = saved.status;
                    key.consecutive_errors = saved.consecutive_errors;
                    key.cooldown_until_ms = saved.cooldown_until_ms;
                }
            }
        }
        if let Ok(mut allowance) = self.allowance.lock() {
            for org in &state.allowance {
                allowance.record(&org.org, org.used, state.saved_at_ms);
            }
            allowance.reset_if_new_utc_day(now_ms);
        }
    }
}

pub struct Engine {
    shared: Shared,
    /// Org for EL keys with no resolved org of their own (and keys added later).
    el_default_org: Option<String>,
    /// `key id -> org_id` from `/api/whoami`; the allowance is per org.
    key_orgs: HashMap<String, String>,
}

fn host_of(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn shape_for(path: &str) -> ApiShape {
    let path = path.split('?').next().unwrap_or(path);
    if path.ends_with("/messages") {
        ApiShape::Messages
    } else if path.ends_with("/responses") {
        ApiShape::Responses
    } else {
        ApiShape::Chat
    }
}

fn extract_model(body: &[u8]) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    json.get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string)
}

/// Route steps may name the model differently per upstream (EL's
/// `deepseek-v4.1-flash-1017` is Go's `deepseek-v4.1-flash`); only then is
/// `model` rewritten.
fn with_model(body: &[u8], model: &str) -> Vec<u8> {
    let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.to_vec();
    };
    if json.get("model").and_then(|m| m.as_str()) == Some(model) {
        return body.to_vec();
    }
    if let Some(obj) = json.as_object_mut() {
        obj.insert("model".to_string(), serde_json::Value::from(model));
    }
    serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec())
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false)
}

/// The one permitted body mutation: ask for usage on a streamed
/// chat-completions request.
fn prepare_body(body: &[u8], path: &str) -> Vec<u8> {
    if shape_for(path) != ApiShape::Chat || body.is_empty() {
        return body.to_vec();
    }
    let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.to_vec();
    };
    if json.get("stream").and_then(|s| s.as_bool()) != Some(true) {
        return body.to_vec();
    }
    let mut options = json
        .get("stream_options")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    options.insert("include_usage".to_string(), serde_json::Value::Bool(true));
    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "stream_options".to_string(),
            serde_json::Value::Object(options),
        );
    }
    serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec())
}

async fn collect(body: ProxyBody) -> Vec<u8> {
    match body {
        ProxyBody::Bytes(bytes) => bytes,
        ProxyBody::Stream(mut stream) => {
            let mut out = Vec::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => out.extend_from_slice(&bytes),
                    Err(_) => break,
                }
            }
            out
        }
    }
}

const TEE_TAIL_BYTES: usize = 65_536;

/// Relay a stream unchanged, accumulating a bounded tail so the final SSE usage
/// frame can be recorded against the org's allowance.
fn tee_usage(
    inner: ByteStream,
    allowance: Arc<Mutex<Allowance>>,
    org: Option<String>,
    now_ms: u64,
) -> ByteStream {
    let state = (inner, Vec::<u8>::new(), allowance, org);
    Box::pin(futures_util::stream::unfold(
        state,
        move |(mut inner, mut acc, allowance, org)| async move {
            match inner.next().await {
                Some(Ok(chunk)) => {
                    acc.extend_from_slice(&chunk);
                    if acc.len() > TEE_TAIL_BYTES {
                        acc.drain(..acc.len() - TEE_TAIL_BYTES);
                    }
                    Some((Ok(chunk), (inner, acc, allowance, org)))
                }
                Some(Err(error)) => Some((Err(error), (inner, acc, allowance, org))),
                None => {
                    let tail = String::from_utf8_lossy(&acc);
                    if let (Some(org), Some(usage)) = (org.as_deref(), parse_usage_from_sse(&tail))
                    {
                        if let Ok(mut allowance) = allowance.lock() {
                            allowance.record(org, usage.total(), now_ms);
                        }
                    }
                    None
                }
            }
        },
    ))
}

fn response_body_stream(body: ProxyBody) -> ByteStream {
    match body {
        ProxyBody::Stream(stream) => stream,
        ProxyBody::Bytes(bytes) => {
            Box::pin(futures_util::stream::once(async move { Ok(bytes.into()) }))
        }
    }
}

impl Engine {
    pub fn new(config: Config, keyfile: &KeyFile, catalog: Catalog) -> Self {
        let keys = KeyManager::from_keyfile(keyfile);
        let breaker = CircuitBreaker::new(config.circuit_breaker_threshold, config.breaker_open_ms);
        let allowance = Allowance::new(config.el_daily_token_cap, config.el_safety_margin);
        let strategy = config.strategy;
        let routing = config.routing.clone();
        Engine {
            shared: Shared {
                config: Arc::new(config),
                keys: Arc::new(Mutex::new(keys)),
                breaker: Arc::new(Mutex::new(breaker)),
                allowance: Arc::new(Mutex::new(allowance)),
                catalog: Arc::new(RwLock::new(Arc::new(catalog))),
                events: EventBus::new(512),
                affinity: Arc::new(Mutex::new(SessionAffinity::new())),
                strategy: Arc::new(Mutex::new(strategy)),
                routing: Arc::new(Mutex::new(routing)),
            },
            el_default_org: None,
            key_orgs: HashMap::new(),
        }
    }

    pub fn shared(&self) -> Shared {
        self.shared.clone()
    }

    pub fn set_el_org(&mut self, org: Option<String>) {
        self.el_default_org = org;
    }

    /// Record the org a specific EL key belongs to.
    pub fn set_key_org(&mut self, key_id: &str, org: String) {
        self.key_orgs.insert(key_id.to_string(), org);
    }

    fn org_for(&self, key_id: &str) -> Option<String> {
        self.key_orgs
            .get(key_id)
            .or(self.el_default_org.as_ref())
            .cloned()
    }

    /// Distinct orgs behind the enabled EL keys, in key order.
    fn el_orgs(&self) -> Vec<String> {
        let ids: Vec<String> = self
            .shared
            .keys
            .lock()
            .map(|keys| {
                keys.all()
                    .iter()
                    .filter(|k| k.provider == Provider::Experiential && k.enabled)
                    .map(|k| k.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut orgs: Vec<String> = Vec::new();
        for org in ids.iter().filter_map(|id| self.org_for(id)) {
            if !orgs.contains(&org) {
                orgs.push(org);
            }
        }
        orgs
    }

    pub fn record_el_usage(&self, org: &str, tokens: u64, now_ms: u64) {
        if let Ok(mut allowance) = self.shared.allowance.lock() {
            allowance.record(org, tokens, now_ms);
        }
    }

    pub fn el_used(&self, org: &str) -> u64 {
        self.shared
            .allowance
            .lock()
            .map(|allowance| allowance.used(org))
            .unwrap_or(0)
    }

    /// Set the Experiential org. A failed lookup must not silently disable the
    /// free lane, so an unknown org falls back to a stable local label.
    pub fn ensure_el_org(&mut self, found: Option<String>) {
        self.el_default_org = Some(found.unwrap_or_else(|| "default".to_string()));
    }

    pub fn breaker_failures(&self, key_id: &str) -> u32 {
        self.shared
            .breaker
            .lock()
            .map(|breaker| breaker.consecutive_failures(key_id))
            .unwrap_or(0)
    }

    pub fn events(&self) -> &EventBus {
        &self.shared.events
    }

    fn targets(&self, explicit: Option<Lane>, model: Option<&str>) -> Vec<Target> {
        if explicit.is_none() {
            if let Some(route) = model.and_then(|m| {
                let routing = self.shared.routing.lock().ok()?;
                routing.route_for(m).cloned()
            }) {
                let keys = self.shared.keys.lock().expect("keys poisoned");
                return route
                    .steps
                    .into_iter()
                    .filter_map(|step| {
                        let provider = keys.get(&step.key)?.provider;
                        Some(Target {
                            provider,
                            key: Some(step.key),
                            model: Some(step.model),
                        })
                    })
                    .collect();
            }
        }
        self.lane_targets(explicit, model)
            .into_iter()
            .map(Target::lane)
            .collect()
    }

    fn lane_targets(&self, explicit: Option<Lane>, model: Option<&str>) -> Vec<Provider> {
        let catalog = self.shared.catalog();
        match explicit {
            Some(Lane::Go) => vec![Provider::OpenCodeGo],
            Some(Lane::Console) => vec![Provider::OpenCodeConsole],
            Some(Lane::Experiential) => {
                let mut targets = vec![Provider::Experiential];
                if model.is_some_and(|m| catalog.has_go_fallback(m)) {
                    targets.push(Provider::OpenCodeGo);
                }
                targets
            }
            None => {
                let model = model.unwrap_or_default();
                let orgs = self.el_orgs();
                let choice = {
                    let allowance = self.shared.allowance.lock().expect("allowance poisoned");
                    // Any org with budget left keeps the free lane open.
                    let org = orgs
                        .iter()
                        .find(|org| !allowance.is_exhausted(org))
                        .or(orgs.first());
                    choose_lane(
                        model,
                        &catalog,
                        &allowance,
                        org.map(String::as_str),
                        self.shared.config.el_allow_paid,
                    )
                };
                match choice {
                    Some(LaneChoice::Experiential) => {
                        let mut targets = vec![Provider::Experiential];
                        if catalog.has_go_fallback(model) {
                            targets.push(Provider::OpenCodeGo);
                        }
                        targets
                    }
                    Some(LaneChoice::Go) => vec![Provider::OpenCodeGo],
                    Some(LaneChoice::Console) => vec![Provider::OpenCodeConsole],
                    None => vec![],
                }
            }
        }
    }

    pub async fn handle<C: UpstreamClient>(
        &self,
        incoming: IncomingRequest,
        client: &C,
        now_ms: u64,
    ) -> ProxyOutcome {
        let (explicit_lane, forward_path) = strip_lane_prefix(&incoming.path);
        let model = extract_model(&incoming.body);
        let client_session = SessionAffinity::extract_session_key(&incoming.headers);
        let body = prepare_body(&incoming.body, &forward_path);

        // A long-running daemon must roll the allowance window over itself.
        if let Ok(mut allowance) = self.shared.allowance.lock() {
            allowance.reset_if_new_utc_day(now_ms);
        }

        if incoming.method.eq_ignore_ascii_case("GET") && forward_path.ends_with("/models") {
            let exposed = self
                .shared
                .routing
                .lock()
                .map(|r| r.exposed(&self.shared.catalog().models))
                .unwrap_or_default();
            let data: Vec<serde_json::Value> = exposed
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "object": "model",
                        "name": m.name,
                        "owned_by": m.provider.as_str(),
                        "limit": { "context": m.context_limit, "output": m.output_limit }
                    })
                })
                .collect();
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-type",
                http::HeaderValue::from_static("application/json"),
            );
            return ProxyOutcome {
                status: 200,
                headers,
                body: ProxyBody::bytes(
                    serde_json::json!({ "object": "list", "data": data }).to_string(),
                ),
            };
        }

        let targets = self.targets(explicit_lane, model.as_deref());
        if targets.is_empty() {
            self.shared.events.emit(EventRecord {
                at_ms: now_ms,
                level: Level::Warn,
                message: format!("no route for model {}", model.clone().unwrap_or_default()),
                detail: None,
            });
            return ProxyOutcome {
                status: 502,
                headers: HeaderMap::new(),
                body: ProxyBody::bytes("{\"error\":\"no route for model\"}"),
            };
        }

        // Go rejects a request without a session; one id per request keeps
        // retries on the same conversation. Without a client header there is
        // nothing stable to derive it from (bodies are never read for routing).
        let session = client_session.clone().unwrap_or_else(generate_session_id);
        let logged_path = incoming.path.split('?').next().unwrap_or_default();
        // Only the first failure of a request counts against a breaker, so one
        // bad body or one network outage cannot open every key's breaker.
        let mut failure_seen = false;
        let mut last: Option<ProxyOutcome> = None;

        for target in targets {
            let provider = target.provider;
            let upstream = upstream_for(provider, &self.shared.config);
            let host = host_of(&upstream.base_url);
            let body = match target.model.as_deref() {
                Some(model) => with_model(&body, model),
                None => body.clone(),
            };
            let available = self
                .shared
                .keys
                .lock()
                .map(|keys| {
                    keys.all()
                        .iter()
                        .filter(|k| k.provider == provider && k.enabled)
                        .count()
                })
                .unwrap_or(0);
            let mut excluded: HashSet<String> = HashSet::new();
            let tries = if target.key.is_some() {
                1
            } else {
                available.max(1)
            };

            for _ in 0..tries {
                // Check what is known before calling: a cooled key, or a spent
                // Experiential org allowance, is skipped without a request.
                let pinned = target.key.as_deref().map(|id| {
                    let el_spent = provider == Provider::Experiential
                        && !self.shared.config.el_allow_paid
                        && self.org_for(id).is_some_and(|org| {
                            self.shared
                                .allowance
                                .lock()
                                .map(|a| a.is_exhausted(&org))
                                .unwrap_or(false)
                        });
                    let keys = self.shared.keys.lock().ok()?;
                    let key = keys
                        .eligible(&excluded, now_ms)
                        .into_iter()
                        .find(|k| k.id == id)?
                        .clone();
                    (!el_spent).then(|| KeySelection {
                        reason: format!("route step {id}"),
                        key,
                    })
                });
                let sticky = client_session
                    .as_deref()
                    .filter(|_| target.key.is_none())
                    .and_then(|session| {
                        let affinity = self.shared.affinity.lock().ok()?;
                        let preferred = affinity.preferred(session)?;
                        if excluded.contains(&preferred) {
                            return None;
                        }
                        let keys = self.shared.keys.lock().ok()?;
                        let key = keys.get(&preferred)?;
                        if key.provider == provider
                            && keys.is_selectable(&preferred, &excluded, now_ms)
                        {
                            Some(KeySelection {
                                key: key.clone(),
                                reason: format!("sticky session reused {preferred}"),
                            })
                        } else {
                            None
                        }
                    });

                let selected = match (pinned, sticky) {
                    (Some(pinned), _) => pinned,
                    (None, Some(selection)) => Some(selection),
                    (None, None) => {
                        let strategy = self
                            .shared
                            .strategy
                            .lock()
                            .map(|s| *s)
                            .unwrap_or(RoutingStrategy::PriorityFailover);
                        self.shared.keys.lock().ok().and_then(|mut keys| {
                            keys.select_for(provider, strategy, &excluded, now_ms)
                        })
                    }
                };
                let Some(selection) = selected else {
                    break;
                };
                let available = self
                    .shared
                    .breaker
                    .lock()
                    .map(|breaker| breaker.is_available(&selection.key.id, now_ms))
                    .unwrap_or(false);
                if !available {
                    excluded.insert(selection.key.id.clone());
                    continue;
                }

                let headers = build_upstream_headers(
                    &incoming.headers,
                    &selection.key.secret,
                    &host,
                    upstream.auth,
                    &session,
                );
                let url = build_upstream_url(&upstream.base_url, &forward_path);
                let request = ProxyRequest {
                    method: incoming.method.clone(),
                    url,
                    headers,
                    body: body.clone(),
                };

                let response = match client.send(request).await {
                    Ok(response) => response,
                    Err(error) => {
                        if !failure_seen {
                            if let Ok(mut breaker) = self.shared.breaker.lock() {
                                breaker.record_failure(&selection.key.id, now_ms);
                            }
                        }
                        failure_seen = true;
                        self.shared.events.emit(EventRecord {
                            at_ms: now_ms,
                            level: Level::Error,
                            message: format!("transport error via {}: {error}", selection.key.id),
                            detail: None,
                        });
                        excluded.insert(selection.key.id.clone());
                        continue;
                    }
                };

                let status = response.status;
                let streaming = status < 400 && is_event_stream(&response.headers);
                let (raw, stream_body) = if streaming {
                    (Vec::new(), Some(response.body))
                } else {
                    (collect(response.body).await, None)
                };
                // Peek as text for quota markers and usage; relay `raw` untouched.
                let text = String::from_utf8_lossy(&raw).into_owned();
                let org = if provider == Provider::Experiential {
                    self.org_for(&selection.key.id)
                } else {
                    None
                };
                let usage = if streaming {
                    None
                } else {
                    parse_usage(&text, shape_for(&forward_path))
                };

                let count_breaker_failure = !failure_seen;
                let attempt = {
                    let signal = ResponseSignal {
                        status,
                        headers: &response.headers,
                        body_peek: &text,
                        key_id: &selection.key.id,
                        provider,
                        org: org.as_deref(),
                        usage,
                        now_ms,
                        fallback_cooldown_ms: self.shared.config.cooldown_ms,
                        count_breaker_failure,
                        advance_on_429: target.key.is_some(),
                    };
                    let mut breaker = self.shared.breaker.lock().expect("breaker poisoned");
                    let mut keys = self.shared.keys.lock().expect("keys poisoned");
                    let mut allowance = self.shared.allowance.lock().expect("allowance poisoned");
                    classify(signal, &mut breaker, &mut keys, &mut allowance)
                };
                if status >= 500 {
                    failure_seen = true;
                }

                let level = match status {
                    s if s >= 500 => Level::Error,
                    s if s >= 400 => Level::Warn,
                    _ => Level::Info,
                };
                self.shared.events.emit(EventRecord {
                    at_ms: now_ms,
                    level,
                    message: format!(
                        "{} {} -> {} via {}",
                        incoming.method, logged_path, status, selection.key.id
                    ),
                    detail: None,
                });

                if attempt == Attempt::Retry {
                    excluded.insert(selection.key.id.clone());
                    last = Some(ProxyOutcome {
                        status,
                        headers: response.headers.clone(),
                        body: ProxyBody::bytes(raw),
                    });
                    continue;
                }

                if let Some(session) = client_session.as_deref() {
                    if let Ok(mut affinity) = self.shared.affinity.lock() {
                        affinity.set(session, &selection.key.id);
                    }
                }

                let out_body = match stream_body {
                    Some(body) => ProxyBody::Stream(tee_usage(
                        response_body_stream(body),
                        Arc::clone(&self.shared.allowance),
                        org,
                        now_ms,
                    )),
                    None => ProxyBody::bytes(raw),
                };
                return ProxyOutcome {
                    status,
                    headers: response.headers,
                    body: out_body,
                };
            }
        }

        last.unwrap_or(ProxyOutcome {
            status: 503,
            headers: HeaderMap::new(),
            body: ProxyBody::bytes("{\"error\":\"all keys failed\"}"),
        })
    }
}
