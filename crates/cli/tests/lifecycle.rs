use std::sync::Mutex;

use http::HeaderMap;
use ocrouter_cli::body::ProxyBody;
use ocrouter_cli::engine::{Engine, IncomingRequest, ProxyRequest, ProxyResponse, UpstreamClient};
use ocrouter_core::config::Config;
use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::models::{Catalog, Lane, ModelInfo};
use ocrouter_core::provider::Provider;

#[derive(Default)]
struct FakeUpstream {
    scripted: Mutex<Vec<(u16, Option<String>, String)>>,
    calls: Mutex<Vec<ProxyRequest>>,
}

impl FakeUpstream {
    fn new(scripted: Vec<(u16, String)>) -> Self {
        Self::new_full(
            scripted
                .into_iter()
                .map(|(status, body)| (status, None, body))
                .collect(),
        )
    }

    fn new_full(scripted: Vec<(u16, Option<String>, String)>) -> Self {
        FakeUpstream {
            scripted: Mutex::new(scripted),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<ProxyRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl UpstreamClient for FakeUpstream {
    async fn send(&self, request: ProxyRequest) -> Result<ProxyResponse, String> {
        let index = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(request);
            calls.len() - 1
        };
        let scripted = self.scripted.lock().unwrap();
        let (status, content_type, body) =
            scripted
                .get(index)
                .cloned()
                .unwrap_or((200, None, "{}".to_string()));
        let mut headers = HeaderMap::new();
        if let Some(content_type) = content_type {
            headers.insert("content-type", content_type.parse().unwrap());
        }
        Ok(ProxyResponse {
            status,
            headers,
            body: ProxyBody::bytes(body),
        })
    }
}

/// Every send fails at the transport level, like a dropped network.
struct DownUpstream;

impl UpstreamClient for DownUpstream {
    async fn send(&self, _request: ProxyRequest) -> Result<ProxyResponse, String> {
        Err("dns error".to_string())
    }
}

fn model(id: &str, lane: Lane) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        lane,
        provider: Provider::OpenCodeGo,
        context_limit: None,
        output_limit: None,
        input_price: None,
        output_price: None,
        supports_tools: true,
        modalities_in: vec!["text".to_string()],
    }
}

fn keyfile(el: bool, go_keys: usize) -> KeyFile {
    let mut kf = KeyFile::default();
    if el {
        kf.add(KeyEntry {
            provider: Provider::Experiential,
            label: "el-1".to_string(),
            secret: "xpl_secret".to_string(),
            priority: 1,
            weight: 1,
            enabled: true,
        });
    }
    for i in 0..go_keys {
        kf.add(KeyEntry {
            provider: Provider::OpenCodeGo,
            label: format!("go-{i}"),
            secret: format!("oc_secret{i}"),
            priority: 1,
            weight: 1,
            enabled: true,
        });
    }
    kf
}

fn shared_catalog() -> Catalog {
    Catalog::merged(
        vec![
            model("deepseek-v4.1-flash", Lane::Go),
            model("kimi-k3", Lane::Go),
        ],
        vec![model("deepseek-v4.1-flash", Lane::Experiential)],
    )
}

fn engine(el: bool, go_keys: usize) -> Engine {
    Engine::new(Config::default(), &keyfile(el, go_keys), shared_catalog())
}

fn post(path: &str, body: &str) -> IncomingRequest {
    IncomingRequest {
        method: "POST".to_string(),
        path: path.to_string(),
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

const SHARED_BODY: &str =
    r#"{"model":"deepseek-v4.1-flash","messages":[{"role":"user","content":"hi"}]}"#;

#[tokio::test]
async fn el_quota_falls_over_to_go_with_the_same_body() {
    let fake = FakeUpstream::new(vec![
        (
            429,
            r#"{"error":{"code":"free_limit_reached"}}"#.to_string(),
        ),
        (200, "{}".to_string()),
    ]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-1".to_string()));

    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "expected one EL try then one Go try");
    assert!(
        calls[0].url.contains("experientiallabs"),
        "got {}",
        calls[0].url
    );
    assert!(
        calls[1].url.contains("opencode.ai/inference/go"),
        "got {}",
        calls[1].url
    );
    assert_eq!(
        calls[0].body, calls[1].body,
        "the body must be forwarded unmodified"
    );
    assert_eq!(out.status, 200);
}

#[tokio::test]
async fn a_go_only_model_goes_straight_to_go_with_a_synthesised_session() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let mut engine = engine(true, 1);
    let body = r#"{"model":"kimi-k3","messages":[{"role":"user","content":"hi"}],"stream":true}"#;

    let _ = engine
        .handle(post("/v1/chat/completions", body), &fake, 0)
        .await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].url.contains("opencode.ai/inference/go"));
    let session = calls[0]
        .headers
        .get("x-opencode-session")
        .expect("Go requires a session header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(session.starts_with("ses_"), "got {session}");
    assert!(calls[0].headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn an_exhausted_el_budget_routes_straight_to_go() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-1".to_string()));
    engine.record_el_usage("org-1", 1_000_000, 0);

    let _ = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].url.contains("opencode.ai/inference/go"),
        "got {}",
        calls[0].url
    );
}

#[tokio::test]
async fn a_repeated_5xx_across_two_keys_counts_against_only_one_breaker() {
    let mut config = Config::default();
    config.circuit_breaker_threshold = 1;
    let mut engine = Engine::new(config, &keyfile(false, 2), shared_catalog());
    let fake = FakeUpstream::new(vec![(500, "boom".to_string()), (500, "boom".to_string())]);

    let _ = engine
        .handle(post("/go/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "both Go keys were tried");
    assert_eq!(
        engine.breaker_failures("opencode-go:go-0"),
        1,
        "the first key legitimately saw the failure"
    );
    assert_eq!(
        engine.breaker_failures("opencode-go:go-1"),
        0,
        "the same body must not count against a second key"
    );
}

#[tokio::test]
async fn an_explicit_lane_prefix_skips_lane_choice() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let mut engine = engine(true, 1);

    let _ = engine
        .handle(post("/go/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].url.contains("opencode.ai/inference/go"));
}

async fn drain(body: ocrouter_cli::body::ProxyBody) -> Vec<u8> {
    use futures_util::StreamExt;
    match body {
        ocrouter_cli::body::ProxyBody::Bytes(bytes) => bytes,
        ocrouter_cli::body::ProxyBody::Stream(mut stream) => {
            let mut out = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                out.extend_from_slice(&chunk);
            }
            out
        }
    }
}

const USAGE_SSE: &str = "data: {\"usage\":{\"prompt_tokens\":900,\"completion_tokens\":100}}\n\n";

#[tokio::test]
async fn streamed_go_usage_does_not_touch_the_el_budget() {
    let fake = FakeUpstream::new_full(vec![(
        200,
        Some("text/event-stream".to_string()),
        USAGE_SSE.to_string(),
    )]);
    let mut engine = engine(false, 1);
    engine.set_el_org(Some("org-1".to_string()));

    let out = engine
        .handle(post("/go/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    let body = drain(out.body).await;
    assert!(String::from_utf8_lossy(&body).contains("prompt_tokens"));
    assert_eq!(
        engine.el_used("org-1"),
        0,
        "a paid lane's streamed usage must not consume the free allowance"
    );
}

#[tokio::test]
async fn streamed_el_usage_is_recorded_against_the_org() {
    let fake = FakeUpstream::new_full(vec![(
        200,
        Some("text/event-stream".to_string()),
        USAGE_SSE.to_string(),
    )]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-1".to_string()));

    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    let _ = drain(out.body).await;
    assert_eq!(engine.el_used("org-1"), 1_000);
}

#[tokio::test]
async fn an_allowance_exhausted_yesterday_is_usable_after_utc_midnight() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-1".to_string()));
    engine.record_el_usage("org-1", 1_000_000, 0);

    let midnight = ocrouter_core::time::utc_midnight_after(0);
    let _ = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, midnight)
        .await;

    let calls = fake.calls();
    assert!(
        calls[0].url.contains("experientiallabs"),
        "a new UTC day must restore the free lane, got {}",
        calls[0].url
    );
}

#[tokio::test]
async fn the_unified_models_route_serves_the_merged_catalog() {
    let fake = FakeUpstream::new(vec![]);
    let mut engine = engine(true, 1);

    let out = engine
        .handle(
            IncomingRequest {
                method: "GET".to_string(),
                path: "/v1/models".to_string(),
                headers: HeaderMap::new(),
                body: Vec::new(),
            },
            &fake,
            0,
        )
        .await;

    assert_eq!(out.status, 200);
    let body = drain(out.body).await;
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("deepseek-v4.1-flash"), "got {text}");
}

#[tokio::test]
async fn an_unresolved_org_falls_back_instead_of_disabling_el() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let mut engine = engine(true, 1);
    engine.ensure_el_org(None);

    let _ = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;

    assert!(
        fake.calls()[0].url.contains("experientiallabs"),
        "an unknown org must not silently switch the product to Go-only"
    );
}

#[tokio::test]
async fn a_network_outage_does_not_open_every_breaker() {
    let config = Config::default();
    assert_eq!(
        config.breaker_open_ms, 60_000,
        "an open breaker lasts a minute, not 5h"
    );
    let engine = Engine::new(config, &keyfile(false, 2), shared_catalog());

    for _ in 0..3 {
        let out = engine
            .handle(
                post("/go/v1/chat/completions", SHARED_BODY),
                &DownUpstream,
                0,
            )
            .await;
        assert_eq!(out.status, 503);
    }
    // Each request tried both keys, but only its first failure counted.
    assert_eq!(engine.breaker_failures("opencode-go:go-0"), 3);
    assert_eq!(
        engine.breaker_failures("opencode-go:go-1"),
        0,
        "one outage must count once per request, not against every key"
    );
}

#[tokio::test]
async fn a_non_utf8_body_is_relayed_byte_for_byte() {
    struct Binary;
    impl UpstreamClient for Binary {
        async fn send(&self, _request: ProxyRequest) -> Result<ProxyResponse, String> {
            Ok(ProxyResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: ProxyBody::bytes(vec![0x1f, 0x8b, 0xff, 0x00, 0xfe]),
            })
        }
    }
    let engine = engine(false, 1);
    let out = engine
        .handle(post("/go/v1/chat/completions", SHARED_BODY), &Binary, 0)
        .await;
    assert_eq!(drain(out.body).await, vec![0x1f, 0x8b, 0xff, 0x00, 0xfe]);
}

#[tokio::test]
async fn a_long_non_ascii_stream_is_relayed_without_panicking() {
    // Odd-length padding puts the 64 KiB cut inside a two-byte character.
    let filler = format!("data: {{\"x\":\"a{}\"}}\n\n", "é".repeat(40_000));
    let sse = format!("{filler}{USAGE_SSE}");
    let fake = FakeUpstream::new_full(vec![(
        200,
        Some("text/event-stream".to_string()),
        sse.clone(),
    )]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-1".to_string()));

    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    assert_eq!(drain(out.body).await, sse.into_bytes());
    assert_eq!(engine.el_used("org-1"), 1_000);
}

#[tokio::test]
async fn a_sticky_session_does_not_reuse_a_cooling_key() {
    let fake = FakeUpstream::new(vec![
        (200, "{}".to_string()),
        (
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#.to_string(),
        ),
        (200, "{}".to_string()),
        (200, "{}".to_string()),
    ]);
    let engine = engine(false, 2);
    let mut request = post("/go/v1/chat/completions", SHARED_BODY);
    request
        .headers
        .insert("x-opencode-session", "ses_sticky".parse().unwrap());

    // 1: go-0 serves and becomes sticky. 2: go-0 hits quota, go-1 serves.
    engine.handle(request.clone(), &fake, 0).await;
    engine.handle(request.clone(), &fake, 0).await;
    // 3: the session must go straight to go-1, not retry the cooling go-0.
    engine.handle(request, &fake, 1).await;

    let calls = fake.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[3].headers["authorization"], "Bearer oc_secret1");
}

#[tokio::test]
async fn retries_keep_the_same_synthesised_session() {
    let fake = FakeUpstream::new(vec![(500, "boom".to_string()), (200, "{}".to_string())]);
    let engine = engine(false, 2);
    engine
        .handle(post("/go/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].headers["x-opencode-session"],
        calls[1].headers["x-opencode-session"]
    );
}

#[tokio::test]
async fn el_usage_is_charged_to_the_serving_keys_own_org() {
    let fake = FakeUpstream::new_full(vec![(
        200,
        Some("text/event-stream".to_string()),
        USAGE_SSE.to_string(),
    )]);
    let mut engine = engine(true, 1);
    engine.set_el_org(Some("org-default".to_string()));
    engine.set_key_org("experiential:el-1", "org-2".to_string());

    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    let _ = drain(out.body).await;
    assert_eq!(engine.el_used("org-2"), 1_000);
    assert_eq!(engine.el_used("org-default"), 0);
}

#[tokio::test]
async fn health_survives_a_snapshot_and_restore() {
    let engine = engine(true, 1);
    let shared = engine.shared();
    shared
        .keys
        .lock()
        .unwrap()
        .mark_exhausted("opencode-go:go-0", 9_000);
    engine.record_el_usage("org-1", 1_234, 0);
    let mut saved = shared.health_snapshot();
    saved.saved_at_ms = 0;

    let fresh = self::engine(true, 1);
    fresh.shared().restore(&saved, 10);
    let keys = fresh
        .shared()
        .keys
        .lock()
        .unwrap()
        .get("opencode-go:go-0")
        .cloned()
        .unwrap();
    assert_eq!(keys.cooldown_until_ms, Some(9_000));
    assert_eq!(fresh.el_used("org-1"), 1_234);

    let next_day = self::engine(true, 1);
    next_day
        .shared()
        .restore(&saved, ocrouter_core::time::utc_midnight_after(0));
    assert_eq!(
        next_day.el_used("org-1"),
        0,
        "yesterday's usage must not carry over"
    );
}

#[tokio::test]
async fn a_refreshed_catalog_is_used_for_routing() {
    let fake = FakeUpstream::new(vec![(200, "{}".to_string())]);
    let engine = Engine::new(
        Config::default(),
        &keyfile(false, 1),
        Catalog::merged(vec![], vec![]),
    );
    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    assert_eq!(out.status, 502, "empty catalog: no route");

    engine.shared().set_catalog(shared_catalog());
    let out = engine
        .handle(post("/v1/chat/completions", SHARED_BODY), &fake, 0)
        .await;
    assert_eq!(out.status, 200);
}
