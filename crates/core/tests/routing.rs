use ocrouter_core::models::Lane;
use ocrouter_core::proxy::routing::{build_upstream_url, strip_lane_prefix};

#[test]
fn upstream_url_does_not_duplicate_v1() {
    assert_eq!(
        build_upstream_url("https://opencode.ai/inference/go", "/v1/chat/completions?x=1"),
        "https://opencode.ai/inference/go/v1/chat/completions?x=1"
    );
    assert_eq!(
        build_upstream_url("https://opencode.ai/inference/go/v1", "/v1/messages"),
        "https://opencode.ai/inference/go/v1/messages"
    );
}

#[test]
fn a_path_without_v1_gains_one() {
    assert_eq!(
        build_upstream_url("https://opencode.ai/inference/go", "/chat/completions"),
        "https://opencode.ai/inference/go/v1/chat/completions"
    );
}

#[test]
fn a_trailing_slash_on_the_base_is_tolerated() {
    assert_eq!(
        build_upstream_url("https://api.experientiallabs.ai/v1/", "/v1/models"),
        "https://api.experientiallabs.ai/v1/models"
    );
}

#[test]
fn lane_prefix_is_stripped() {
    assert_eq!(
        strip_lane_prefix("/go/v1/chat/completions"),
        (Some(Lane::Go), "/v1/chat/completions".to_string())
    );
    assert_eq!(
        strip_lane_prefix("/el/v1/messages"),
        (Some(Lane::Experiential), "/v1/messages".to_string())
    );
    assert_eq!(
        strip_lane_prefix("/v1/chat/completions"),
        (None, "/v1/chat/completions".to_string())
    );
    assert_eq!(strip_lane_prefix("/v1"), (None, "/v1".to_string()));
    assert_eq!(strip_lane_prefix("/go"), (Some(Lane::Go), "/".to_string()));
}

#[test]
fn a_prefix_that_is_only_a_similar_word_is_not_a_lane() {
    // "/gopher" must not match the "/go" lane
    assert_eq!(strip_lane_prefix("/gopher"), (None, "/gopher".to_string()));
}
