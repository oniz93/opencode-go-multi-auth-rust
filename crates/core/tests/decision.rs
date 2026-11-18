use http::HeaderMap;
use ocrouter_core::models::{Catalog, Lane, ModelInfo};
use ocrouter_core::provider::Provider;
use ocrouter_core::proxy::decision::{classify, choose_lane, Attempt, LaneChoice, ResponseSignal};
use ocrouter_core::proxy::usage::TokenUsage;
use ocrouter_core::router::allowance::Allowance;
use ocrouter_core::router::circuit_breaker::{CircuitBreaker, CircuitState};
use ocrouter_core::router::key_manager::KeyManager;
use ocrouter_core::router::types::{ApiKey, KeyStatus};

fn m(id: &str, lane: Lane) -> ModelInfo {
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

fn key(id: &str) -> ApiKey {
    ApiKey {
        id: id.to_string(),
        provider: Provider::Experiential,
        label: id.to_string(),
        secret: "xpl_x".to_string(),
        priority: 1,
        weight: 1,
        enabled: true,
        status: KeyStatus::Active,
        consecutive_errors: 0,
        cooldown_until_ms: None,
    }
}

fn shared_catalog() -> Catalog {
    Catalog::merged(
        vec![m("deepseek-v4.1-flash", Lane::Go)],
        vec![m("deepseek-v4.1-flash", Lane::Experiential)],
    )
}

#[test]
fn a_shared_model_goes_to_el_first_then_go_when_the_budget_is_gone() {
    let catalog = shared_catalog();
    let mut a = Allowance::new(1_000_000, 0.0);
    assert_eq!(
        choose_lane("deepseek-v4.1-flash", &catalog, &a, Some("org-1"), false),
        Some(LaneChoice::Experiential)
    );
    a.record("org-1", 1_000_000, 0);
    assert_eq!(
        choose_lane("deepseek-v4.1-flash", &catalog, &a, Some("org-1"), false),
        Some(LaneChoice::Go)
    );
}

#[test]
fn a_go_only_model_ignores_the_el_budget() {
    let catalog = Catalog::merged(vec![m("kimi-k3", Lane::Go)], vec![]);
    let a = Allowance::new(1_000_000, 0.0);
    assert_eq!(choose_lane("kimi-k3", &catalog, &a, None, false), Some(LaneChoice::Go));
}

#[test]
fn a_shared_model_that_el_rejected_is_still_reachable_on_go() {
    let catalog = shared_catalog();
    let a = Allowance::new(1_000_000, 0.0);
    assert_eq!(
        choose_lane("deepseek-v4.1-flash", &catalog, &a, None, false),
        Some(LaneChoice::Go)
    );
}

#[test]
fn an_el_only_model_with_no_budget_is_unservable() {
    let catalog = Catalog::merged(vec![], vec![m("clef", Lane::Experiential)]);
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 1_000_000, 0);
    assert_eq!(choose_lane("clef", &catalog, &a, Some("org-1"), false), None);
}

#[test]
fn an_unknown_model_is_unservable() {
    assert_eq!(choose_lane("nope", &shared_catalog(), &Allowance::new(1, 0.0), None, false), None);
}

fn signal<'a>(
    status: u16,
    body: &'a str,
    headers: &'a HeaderMap,
    provider: Provider,
    usage: Option<TokenUsage>,
    count_breaker_failure: bool,
) -> ResponseSignal<'a> {
    ResponseSignal {
        status,
        headers,
        body_peek: body,
        key_id: "a",
        provider,
        org: Some("org-1"),
        usage,
        now_ms: 0,
        fallback_cooldown_ms: 1_000,
        count_breaker_failure,
    }
}

#[test]
fn a_quota_429_cools_the_key_and_asks_for_a_retry() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    let mut km = KeyManager::new(vec![key("a")]);
    let mut allowance = Allowance::new(1_000_000, 0.0);
    let headers = HeaderMap::new();
    let out = classify(
        signal(
            429,
            r#"{"error":{"code":"free_limit_reached"}}"#,
            &headers,
            Provider::Experiential,
            None,
            true,
        ),
        &mut cb,
        &mut km,
        &mut allowance,
    );
    assert_eq!(out, Attempt::Retry);
    assert!(km.get("a").unwrap().cooldown_until_ms.is_some());
}

#[test]
fn a_plain_429_is_not_a_quota_and_is_passed_through() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    let mut km = KeyManager::new(vec![key("a")]);
    let mut allowance = Allowance::new(1_000_000, 0.0);
    let headers = HeaderMap::new();
    let out = classify(
        signal(
            429,
            r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#,
            &headers,
            Provider::OpenCodeGo,
            None,
            true,
        ),
        &mut cb,
        &mut km,
        &mut allowance,
    );
    assert_eq!(out, Attempt::Stop);
    assert!(km.get("a").unwrap().cooldown_until_ms.is_none());
}

#[test]
fn a_repeated_5xx_does_not_open_the_breaker() {
    let mut cb = CircuitBreaker::new(1, 60_000);
    let mut km = KeyManager::new(vec![key("a")]);
    let mut allowance = Allowance::new(1_000_000, 0.0);
    let headers = HeaderMap::new();

    classify(
        signal(500, "", &headers, Provider::OpenCodeGo, None, true),
        &mut cb,
        &mut km,
        &mut allowance,
    );
    assert_eq!(cb.state_of("a", 0), CircuitState::Open);

    // A second, different key seeing the SAME body must not be counted.
    let mut km2 = KeyManager::new(vec![key("b")]);
    classify(
        signal(500, "", &headers, Provider::OpenCodeGo, None, false),
        &mut cb,
        &mut km2,
        &mut allowance,
    );
    assert_eq!(cb.state_of("b", 0), CircuitState::Closed);
}

#[test]
fn a_2xx_records_success_and_adds_el_usage_to_the_org() {
    let mut cb = CircuitBreaker::new(1, 60_000);
    let mut km = KeyManager::new(vec![key("a")]);
    let mut allowance = Allowance::new(1_000_000, 0.0);
    let headers = HeaderMap::new();
    let usage = TokenUsage {
        input: 700,
        output: 300,
        cache_read: 0,
        cache_write: 0,
    };
    let out = classify(
        signal(200, "", &headers, Provider::Experiential, Some(usage), true),
        &mut cb,
        &mut km,
        &mut allowance,
    );
    assert_eq!(out, Attempt::Stop);
    assert_eq!(allowance.used("org-1"), 1_000);
}

#[test]
fn a_2xx_for_go_does_not_touch_the_allowance() {
    let mut cb = CircuitBreaker::new(1, 60_000);
    let mut km = KeyManager::new(vec![key("a")]);
    let mut allowance = Allowance::new(1_000_000, 0.0);
    let headers = HeaderMap::new();
    let usage = TokenUsage {
        input: 700,
        output: 300,
        cache_read: 0,
        cache_write: 0,
    };
    classify(
        signal(200, "", &headers, Provider::OpenCodeGo, Some(usage), true),
        &mut cb,
        &mut km,
        &mut allowance,
    );
    assert_eq!(allowance.used("org-1"), 0);
}

#[test]
fn allow_paid_keeps_using_el_after_the_free_allowance_is_spent() {
    let catalog = shared_catalog();
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 1_000_000, 0);
    assert_eq!(
        choose_lane("deepseek-v4.1-flash", &catalog, &a, Some("org-1"), true),
        Some(LaneChoice::Experiential)
    );
}
