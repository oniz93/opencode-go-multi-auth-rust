use ocrouter_core::models::catalog::{parse_el_catalog, parse_model_list};
use ocrouter_core::models::Lane;
use ocrouter_core::provider::Provider;

#[test]
fn normalises_openai_list_shape_and_skips_non_chat() {
    let raw = r#"{"object":"list","data":[
      {"id":"kimi-k3","object":"model","name":"Kimi K3","type":"chat","limit":{"context":256000,"output":32000}},
      {"id":"some-image","object":"model","type":"image"}
    ]}"#;
    let models = parse_model_list(raw, Provider::OpenCodeGo, Lane::Go).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "kimi-k3");
    assert_eq!(models[0].name, "Kimi K3");
    assert_eq!(models[0].context_limit, Some(256000));
    assert_eq!(models[0].output_limit, Some(32000));
}

#[test]
fn a_missing_name_falls_back_to_the_id() {
    let raw = r#"{"data":[{"id":"mystery","object":"model","type":"chat"}]}"#;
    let models = parse_model_list(raw, Provider::OpenCodeGo, Lane::Go).unwrap();
    assert_eq!(models[0].name, "mystery");
    assert_eq!(models[0].context_limit, None);
}

#[test]
fn el_catalog_shape_is_parsed() {
    let raw = r#"{"models":[{"model":{"slug":"claude-opus-5.5","display_name":"Claude Opus 5.5","context_window":1000000,"max_output_tokens":128000}}]}"#;
    let models = parse_el_catalog(raw).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "claude-opus-5.5");
    assert_eq!(models[0].name, "Claude Opus 5.5");
    assert_eq!(models[0].context_limit, Some(1_000_000));
    assert_eq!(models[0].output_limit, Some(128_000));
    assert_eq!(models[0].lane, Lane::Experiential);
    assert_eq!(models[0].provider, Provider::Experiential);
}

#[test]
fn malformed_catalogs_return_none_rather_than_panicking() {
    assert!(parse_model_list("not json", Provider::OpenCodeGo, Lane::Go).is_none());
    assert!(parse_el_catalog("{\"nope\":1}").is_none());
    assert!(parse_el_catalog("[]").is_none());
}

#[test]
fn el_entries_without_a_slug_are_skipped() {
    let raw = r#"{"models":[{"model":{"display_name":"Nameless"}},{"model":{"slug":"good"}}]}"#;
    let models = parse_el_catalog(raw).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "good");
}

#[test]
fn model_list_urls_hit_v1_models_exactly_once() {
    use ocrouter_core::config::Config;
    use ocrouter_core::models::catalog::models_url;
    use ocrouter_core::provider::{upstream_for, Provider};
    let config = Config::default();
    assert_eq!(
        models_url(&upstream_for(Provider::Experiential, &config)),
        "https://api.experientiallabs.ai/v1/models"
    );
    assert_eq!(
        models_url(&upstream_for(Provider::OpenCodeGo, &config)),
        "https://opencode.ai/inference/go/v1/models"
    );
}
