use ocrouter_core::proxy::rate_card::estimate_cost;
use ocrouter_core::proxy::usage::{parse_usage, parse_usage_from_sse, ApiShape, TokenUsage};

fn u(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenUsage {
    TokenUsage {
        input,
        output,
        cache_read,
        cache_write,
    }
}

#[test]
fn reads_openai_usage() {
    let parsed = parse_usage(
        r#"{"usage":{"prompt_tokens":100,"completion_tokens":50}}"#,
        ApiShape::Chat,
    )
    .unwrap();
    assert_eq!((parsed.input, parsed.output), (100, 50));
}

#[test]
fn reads_anthropic_usage_including_cache() {
    let parsed = parse_usage(
        r#"{"usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":900,"cache_creation_input_tokens":20}}"#,
        ApiShape::Messages,
    )
    .unwrap();
    assert_eq!(
        (parsed.input, parsed.output, parsed.cache_read, parsed.cache_write),
        (10, 4, 900, 20)
    );
    assert_eq!(parsed.total(), 10 + 4 + 900 + 20);
}

#[test]
fn reads_responses_api_usage() {
    let parsed = parse_usage(
        r#"{"usage":{"input_tokens":7,"output_tokens":3,"input_tokens_details":{"cached_tokens":5}}}"#,
        ApiShape::Responses,
    )
    .unwrap();
    assert_eq!((parsed.input, parsed.output, parsed.cache_read), (7, 3, 5));
}

#[test]
fn malformed_bodies_return_none_instead_of_panicking() {
    assert!(parse_usage("not json", ApiShape::Chat).is_none());
    assert!(parse_usage(r#"{"usage":null}"#, ApiShape::Chat).is_none());
    assert!(parse_usage(r#"{}"#, ApiShape::Chat).is_none());
}

#[test]
fn sse_usage_is_extracted_from_a_final_chunk() {
    let chunk = "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n";
    let parsed = parse_usage_from_sse(chunk).unwrap();
    assert_eq!((parsed.input, parsed.output), (10, 2));
    assert!(parse_usage_from_sse("data: [DONE]\n\n").is_none());
}

#[test]
fn unknown_model_has_no_estimate() {
    assert!(estimate_cost("not-a-real-model", &u(1, 1, 0, 0)).is_none());
}

#[test]
fn known_model_estimates_from_the_rate_card() {
    // Kimi K3: 3.00 in / 15.00 out / 0.30 cache read per 1M tokens
    let cost = estimate_cost("kimi-k3", &u(1_000_000, 1_000_000, 0, 0)).unwrap();
    assert!((cost - 18.0).abs() < 1e-9, "got {cost}");
}

#[test]
fn free_models_cost_zero() {
    let cost = estimate_cost("clef", &u(1_000_000, 1_000_000, 0, 0));
    // clef is not in the Go rate card; it must be absent, not zero
    assert!(cost.is_none() || cost == Some(0.0));
}
