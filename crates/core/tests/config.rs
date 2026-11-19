use ocrouter_core::config::Config;

#[test]
fn defaults_match_spec() {
    let c = Config::default();
    assert_eq!(c.proxy_port, 18905);
    assert_eq!(c.control_port, 18904);
    assert_eq!(c.el_daily_token_cap, 1_000_000);
    assert!(!c.el_allow_paid);
    assert_eq!(c.go_base_url, "https://opencode.ai/inference/go");
    assert_eq!(c.el_base_url, "https://api.experientiallabs.ai/v1");
}

#[test]
fn missing_file_yields_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let c = Config::load(&dir.path().join("nope.toml")).unwrap();
    assert_eq!(c.proxy_port, 18905);
}

#[test]
fn routing_round_trips_and_filters_exposed_models() {
    use ocrouter_core::models::{Lane, ModelInfo};
    use ocrouter_core::provider::Provider;

    let raw = r#"
[routing]
expose = ["ds", "kimi"]

[[routing.route]]
model = "ds"
steps = [{ key = "opencode-go:go-1", model = "deepseek-v4.1-flash" }]
"#;
    let c: Config = toml::from_str(raw).unwrap();
    assert_eq!(c.routing.route_for("ds").unwrap().steps.len(), 1);
    assert!(c.routing.route_for("other").is_none());
    let again: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
    assert_eq!(again.routing, c.routing);

    let model = |id: &str, ctx| ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        lane: Lane::Go,
        provider: Provider::OpenCodeGo,
        context_limit: ctx,
        output_limit: None,
        input_price: None,
        output_price: None,
        supports_tools: true,
        modalities_in: vec![],
    };
    let catalog = vec![
        model("deepseek-v4.1-flash", Some(128_000)),
        model("glm", None),
    ];
    let exposed = c.routing.exposed(&catalog);
    assert_eq!(
        exposed.len(),
        1,
        "kimi is not in the catalog, glm is not exposed"
    );
    assert_eq!(exposed[0].id, "ds");
    assert_eq!(
        exposed[0].context_limit,
        Some(128_000),
        "alias borrows its step's limits"
    );
}
