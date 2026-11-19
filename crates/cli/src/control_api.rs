use std::convert::Infallible;
use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::Stream;
use serde::Deserialize;
use serde_json::{json, Value};

use ocrouter_core::keys::{mask_secret, KeyEntry, KeyFile};
use ocrouter_core::models::opencode::generate_snippet;
use ocrouter_core::provider::Provider;
use ocrouter_core::router::types::{ApiKey, KeyStatus, RoutingStrategy};

use crate::engine::Shared;

#[derive(Clone)]
pub struct AppState {
    pub shared: Shared,
    pub token: String,
    /// `keys.toml`; when set, keys added or removed here survive a restart.
    pub keys_path: Option<PathBuf>,
}

/// Apply `change` to `keys.toml`. A no-op when no path is configured.
fn persist_keys(state: &AppState, change: impl FnOnce(&mut KeyFile)) -> anyhow::Result<()> {
    let Some(path) = state.keys_path.as_deref() else {
        return Ok(());
    };
    let mut file = KeyFile::load(path)?;
    change(&mut file);
    file.save(path)
}

fn persist_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": format!("could not update keys file: {error}") })),
    )
        .into_response()
}

pub struct ControlApi {
    state: AppState,
}

impl ControlApi {
    pub fn new(mut state: AppState, token: String) -> Self {
        state.token = token;
        ControlApi { state }
    }

    pub fn token(&self) -> &str {
        &self.state.token
    }

    pub fn router(self) -> Router {
        let state = self.state;

        let api = Router::new()
            .route("/keys", get(list_keys).post(add_key))
            .route("/keys/{id}", delete(remove_key))
            .route("/keys/{id}/test", post(test_key))
            .route("/status", get(status))
            .route("/models", get(list_models))
            .route("/models/opencode-snippet", get(snippet))
            .route("/strategy", get(get_strategy).put(put_strategy))
            .route("/events", get(sse_events))
            .layer(middleware::from_fn_with_state(state.clone(), require_token));

        Router::new()
            .route("/healthz", get(healthz))
            .nest("/api", api)
            .with_state(state)
    }
}

async fn require_token(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let ok = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|value| value == format!("Bearer {}", state.token))
        .unwrap_or(false);
    if ok {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn healthz(State(state): State<AppState>) -> Json<Value> {
    let keys = state.shared.keys.lock().expect("keys poisoned");
    Json(json!({
        "ok": true,
        "keys": keys.all().len(),
        "activeKeys": keys.enabled().len()
    }))
}

async fn list_keys(State(state): State<AppState>) -> Json<Value> {
    let keys = state.shared.keys.lock().expect("keys poisoned");
    let entries: Vec<Value> = keys
        .all()
        .iter()
        .map(|key| {
            json!({
                "id": key.id,
                "provider": key.provider,
                "label": key.label,
                "preview": mask_secret(&key.secret),
                "enabled": key.enabled,
                "status": key.status,
                "consecutiveErrors": key.consecutive_errors,
                "cooldownUntilMs": key.cooldown_until_ms,
            })
        })
        .collect();
    Json(Value::Array(entries))
}

fn default_one() -> u32 {
    1
}

#[derive(Deserialize)]
struct AddKeyRequest {
    secret: String,
    provider: Provider,
    label: String,
    #[serde(default = "default_one")]
    priority: u32,
    #[serde(default = "default_one")]
    weight: u32,
}

async fn add_key(State(state): State<AppState>, Json(body): Json<AddKeyRequest>) -> Response {
    let entry = KeyEntry {
        provider: body.provider,
        label: body.label.clone(),
        secret: body.secret.clone(),
        priority: body.priority,
        weight: body.weight,
        enabled: true,
    };
    if let Err(error) = persist_keys(&state, |file| file.add(entry)) {
        return persist_error(error);
    }
    let id = ApiKey::new_id(body.provider, &body.label);
    let key = ApiKey {
        id: id.clone(),
        provider: body.provider,
        label: body.label,
        secret: body.secret,
        priority: body.priority,
        weight: body.weight,
        enabled: true,
        status: KeyStatus::Active,
        consecutive_errors: 0,
        cooldown_until_ms: None,
    };
    let preview = mask_secret(&key.secret);
    state.shared.keys.lock().expect("keys poisoned").add(key);
    Json(json!({ "id": id, "preview": preview })).into_response()
}

async fn remove_key(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if let Err(error) = persist_keys(&state, |file| {
        file.key
            .retain(|k| ApiKey::new_id(k.provider, &k.label) != id)
    }) {
        return persist_error(error);
    }
    let removed = state.shared.keys.lock().expect("keys poisoned").remove(&id);
    if removed {
        Json(json!({ "removed": id })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "key not found" })),
        )
            .into_response()
    }
}

async fn test_key(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let present = state
        .shared
        .keys
        .lock()
        .expect("keys poisoned")
        .get(&id)
        .is_some();
    if present {
        // A live probe needs the upstream; the CLI/GUI runs it. Here we report
        // that the key exists and is selectable.
        Json(json!({ "id": id, "present": true })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "key not found" })),
        )
            .into_response()
    }
}

async fn status(State(state): State<AppState>) -> Json<Value> {
    let keys = state.shared.keys.lock().expect("keys poisoned");
    let allowance = state.shared.allowance.lock().expect("allowance poisoned");
    let strategy = state
        .shared
        .strategy
        .lock()
        .map(|s| *s)
        .unwrap_or(RoutingStrategy::PriorityFailover);
    Json(json!({
        "keys": keys.all().len(),
        "activeKeys": keys.enabled().len(),
        "strategy": strategy,
        "allowance": allowance.snapshot(),
        "events": state.shared.events.recent(50),
    }))
}

async fn list_models(State(state): State<AppState>) -> Json<Value> {
    let catalog = state.shared.catalog();
    let models: Vec<Value> = catalog
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "lane": m.lane,
                "provider": m.provider,
                "contextLimit": m.context_limit,
                "outputLimit": m.output_limit,
            })
        })
        .collect();
    Json(Value::Array(models))
}

async fn snippet(State(state): State<AppState>) -> Json<Value> {
    let catalog = state.shared.catalog();
    let base = format!("http://127.0.0.1:{}/v1", state.shared.config.proxy_port);
    Json(generate_snippet(&catalog.models, &base))
}

async fn get_strategy(State(state): State<AppState>) -> Json<Value> {
    let strategy = state
        .shared
        .strategy
        .lock()
        .map(|s| *s)
        .unwrap_or(RoutingStrategy::PriorityFailover);
    Json(json!({ "strategy": strategy }))
}

#[derive(Deserialize)]
struct StrategyRequest {
    strategy: RoutingStrategy,
}

async fn put_strategy(
    State(state): State<AppState>,
    Json(body): Json<StrategyRequest>,
) -> Json<Value> {
    if let Ok(mut strategy) = state.shared.strategy.lock() {
        *strategy = body.strategy;
    }
    Json(json!({ "strategy": body.strategy }))
}

async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.shared.events.subscribe();
    let stream = futures_util::stream::unfold(receiver, |mut rx| async move {
        match rx.recv().await {
            Ok(record) => {
                let event = Event::default().json_data(&record).unwrap_or_default();
                Some((Ok::<_, Infallible>(event), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Some((Ok(Event::default()), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Sse::new(stream)
}
