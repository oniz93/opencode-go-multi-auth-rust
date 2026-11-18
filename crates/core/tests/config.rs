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
