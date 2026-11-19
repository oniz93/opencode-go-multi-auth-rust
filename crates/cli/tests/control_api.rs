use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use ocrouter_cli::control_api::{AppState, ControlApi};
use ocrouter_cli::engine::Shared;
use ocrouter_core::config::Config;
use ocrouter_core::events::EventBus;
use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::models::Catalog;
use ocrouter_core::provider::Provider;
use ocrouter_core::router::allowance::Allowance;
use ocrouter_core::router::circuit_breaker::CircuitBreaker;
use ocrouter_core::router::key_manager::KeyManager;
use ocrouter_core::router::session_affinity::SessionAffinity;

fn app_with(secret: Option<&str>) -> (axum::Router, String) {
    app_at(secret, None, None)
}

fn app_at(
    secret: Option<&str>,
    keys_path: Option<std::path::PathBuf>,
    config_path: Option<std::path::PathBuf>,
) -> (axum::Router, String) {
    let token = "test-token".to_string();
    let mut kf = KeyFile::default();
    if let Some(secret) = secret {
        kf.add(KeyEntry {
            provider: Provider::OpenCodeGo,
            label: "go-1".to_string(),
            secret: secret.to_string(),
            priority: 1,
            weight: 1,
            enabled: true,
        });
    }
    let config = Config::default();
    let shared = Shared {
        config: Arc::new(config.clone()),
        keys: Arc::new(Mutex::new(KeyManager::from_keyfile(&kf))),
        breaker: Arc::new(Mutex::new(CircuitBreaker::new(
            config.circuit_breaker_threshold,
            config.cooldown_ms,
        ))),
        allowance: Arc::new(Mutex::new(Allowance::new(1_000_000, 0.05))),
        catalog: Arc::new(Catalog::merged(vec![], vec![])),
        events: EventBus::new(64),
        affinity: Arc::new(Mutex::new(SessionAffinity::new())),
        strategy: Arc::new(Mutex::new(config.strategy)),
        routing: Arc::new(Mutex::new(config.routing.clone())),
    };
    (
        ControlApi::new(
            AppState {
                shared,
                token: String::new(),
                keys_path,
                config_path,
            },
            token.clone(),
        )
        .router(),
        token,
    )
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    auth: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(auth) = auth {
        builder = builder.header("authorization", auth);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
async fn api_requires_the_token() {
    let (app, token) = app_with(Some("oc_VERYSECRET1234"));
    assert_eq!(
        send(&app, "GET", "/api/keys", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", "/api/keys", Some("Bearer wrong")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", "/api/keys", Some(&format!("Bearer {token}")))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn healthz_is_public() {
    let (app, _token) = app_with(None);
    assert_eq!(send(&app, "GET", "/healthz", None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn key_listing_never_leaks_a_secret() {
    let (app, token) = app_with(Some("oc_VERYSECRET1234"));
    let (status, body) = send(&app, "GET", "/api/keys", Some(&format!("Bearer {token}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("VERYSECRET"), "leaked: {body}");
    assert!(body.contains("go-1"));
}

#[tokio::test]
async fn strategy_can_be_read_and_written() {
    let (app, token) = app_with(None);
    let auth = format!("Bearer {token}");
    let (status, body) = send(&app, "GET", "/api/strategy", Some(&auth)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("priority_failover"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/strategy")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"strategy":"round_robin"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (_, body) = send(&app, "GET", "/api/strategy", Some(&auth)).await;
    assert!(body.contains("round_robin"), "got {body}");
}

#[tokio::test]
async fn status_reports_key_counts() {
    let (app, token) = app_with(Some("oc_x"));
    let (status, body) = send(&app, "GET", "/api/status", Some(&format!("Bearer {token}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"keys\""));
}

async fn send_json(
    app: &axum::Router,
    method: &str,
    path: &str,
    auth: &str,
    body: &str,
) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", auth)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
async fn keys_and_routing_edits_are_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let keys_path = dir.path().join("keys.toml");
    let config_path = dir.path().join("config.toml");
    let (app, token) = app_at(None, Some(keys_path.clone()), Some(config_path.clone()));
    let auth = format!("Bearer {token}");

    let (status, body) = send_json(
        &app,
        "POST",
        "/api/keys",
        &auth,
        r#"{"provider":"opencode-console","label":"zen-1","secret":"sk_ZENSECRET99"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains("ZENSECRET"), "leaked: {body}");
    assert_eq!(KeyFile::load(&keys_path).unwrap().key.len(), 1);

    let routing = r#"{"expose":["ds"],"route":[{"model":"ds","steps":[
        {"key":"opencode-console:zen-1","model":"deepseek-v4.1-flash-free"}]}]}"#;
    let (status, _) = send_json(&app, "PUT", "/api/routing", &auth, routing).await;
    assert_eq!(status, StatusCode::OK);
    let saved = Config::read(&config_path).unwrap();
    assert_eq!(saved.routing.expose, vec!["ds".to_string()]);
    assert_eq!(
        saved.routing.route[0].steps[0].model,
        "deepseek-v4.1-flash-free"
    );

    let (_, body) = send(&app, "GET", "/api/routing", Some(&auth)).await;
    assert!(body.contains("opencode-console:zen-1"), "{body}");

    let (status, _) = send(
        &app,
        "DELETE",
        "/api/keys/opencode-console:zen-1",
        Some(&auth),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(KeyFile::load(&keys_path).unwrap().key.is_empty());

    let (status, _) = send_json(
        &app,
        "PUT",
        "/api/routing",
        &auth,
        r#"{"route":[{"model":"","steps":[]}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
