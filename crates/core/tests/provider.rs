use ocrouter_core::config::Config;
use ocrouter_core::provider::{upstream_for, AuthStyle, Provider};

#[test]
fn go_and_el_auth_styles_differ() {
    let cfg = Config::default();
    assert!(matches!(
        upstream_for(Provider::OpenCodeGo, &cfg).auth,
        AuthStyle::BearerAndApiKey
    ));
    assert!(matches!(
        upstream_for(Provider::Experiential, &cfg).auth,
        AuthStyle::BearerOnly
    ));
}

#[test]
fn providers_serialise_kebab_case() {
    assert_eq!(
        serde_json::to_string(&Provider::OpenCodeGo).unwrap(),
        "\"opencode-go\""
    );
    assert_eq!(
        serde_json::to_string(&Provider::Experiential).unwrap(),
        "\"experiential\""
    );
}

#[test]
fn upstream_base_urls_come_from_config() {
    let mut cfg = Config::default();
    cfg.go_base_url = "https://example.test/go".to_string();
    assert_eq!(upstream_for(Provider::OpenCodeGo, &cfg).base_url, "https://example.test/go");
}
