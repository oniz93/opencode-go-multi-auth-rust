use ocrouter_core::models::opencode::{generate_snippet, write_opencode_config};
use ocrouter_core::models::{Lane, ModelInfo};
use ocrouter_core::provider::Provider;

fn m(id: &str, lane: Lane) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: format!("{id} display"),
        lane,
        provider: Provider::OpenCodeGo,
        context_limit: Some(256_000),
        output_limit: Some(32_000),
        input_price: Some(3.0),
        output_price: Some(15.0),
        supports_tools: true,
        modalities_in: vec!["text".to_string()],
    }
}

fn snippet() -> serde_json::Value {
    generate_snippet(&[m("kimi-k3", Lane::Go)], "http://127.0.0.1:18905/v1")
}

#[test]
fn snippet_uses_v2_providers_shape() {
    let v = snippet();
    let p = &v["providers"]["ocrouter"];
    assert_eq!(p["package"], "@opencode/ai/providers/openai-compatible");
    assert_eq!(p["settings"]["baseURL"], "http://127.0.0.1:18905/v1");
    assert!(p["models"]["kimi-k3"].is_object());
}

#[test]
fn snippet_carries_limit_cost_and_capabilities() {
    let v = snippet();
    let model = &v["providers"]["ocrouter"]["models"]["kimi-k3"];
    assert_eq!(model["name"], "kimi-k3 display");
    assert_eq!(model["limit"]["context"], 256_000);
    assert_eq!(model["limit"]["output"], 32_000);
    assert_eq!(model["cost"]["input"], 3.0);
    assert_eq!(model["cost"]["output"], 15.0);
    assert_eq!(model["capabilities"]["tools"], true);
}

#[test]
fn write_preserves_unrelated_keys_and_backs_up() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("opencode.json");
    std::fs::write(
        &cfg,
        r#"{"$schema":"x","model":"openai/gpt","provider":{"google":{}}}"#,
    )
    .unwrap();

    let backup = write_opencode_config(&cfg, &snippet()).unwrap();
    assert!(backup.exists());

    let out: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(out["model"], "openai/gpt");
    assert!(out["provider"]["google"].is_object());
    assert!(out["providers"]["ocrouter"]["models"]["kimi-k3"].is_object());
}

#[test]
fn write_is_idempotent_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("opencode.json");
    write_opencode_config(&cfg, &snippet()).unwrap();
    write_opencode_config(&cfg, &snippet()).unwrap();
    let out: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(
        out["providers"]["ocrouter"]["models"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn write_creates_the_file_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("nested").join("opencode.json");
    write_opencode_config(&cfg, &snippet()).unwrap();
    assert!(cfg.exists());
}
