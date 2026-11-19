use http::HeaderMap;
use ocrouter_core::provider::AuthStyle;
use ocrouter_core::proxy::headers::{
    build_upstream_headers, ensure_go_session, generate_session_id, is_hop_by_hop, PASSTHROUGH,
};

#[test]
fn both_auth_headers_are_sent_for_go() {
    let h = build_upstream_headers(
        &HeaderMap::new(),
        "oc_x",
        "opencode.ai",
        AuthStyle::BearerAndApiKey,
        "ses_1",
    );
    assert_eq!(h["authorization"], "Bearer oc_x");
    assert_eq!(h["x-api-key"], "oc_x");
}

#[test]
fn el_sends_only_bearer() {
    let h = build_upstream_headers(
        &HeaderMap::new(),
        "xpl_x",
        "api.experientiallabs.ai",
        AuthStyle::BearerOnly,
        "ses_1",
    );
    assert_eq!(h["authorization"], "Bearer xpl_x");
    assert!(!h.contains_key("x-api-key"));
}

#[test]
fn the_clients_own_credentials_are_replaced() {
    let mut client = HeaderMap::new();
    client.insert("authorization", "Bearer someone-elses-key".parse().unwrap());
    client.insert("x-api-key", "someone-elses-key".parse().unwrap());
    let h = build_upstream_headers(&client, "ours", "h", AuthStyle::BearerAndApiKey, "ses_1");
    assert_eq!(h["authorization"], "Bearer ours");
    assert_eq!(h["x-api-key"], "ours");
}

#[test]
fn go_lane_synthesises_a_session_when_the_client_sent_none() {
    let mut h = HeaderMap::new();
    ensure_go_session(&mut h, None);
    let v = h["x-opencode-session"].to_str().unwrap();
    assert!(v.starts_with("ses_"), "got {v}");
}

#[test]
fn go_lane_derives_from_prompt_cache_key_when_present() {
    let mut h = HeaderMap::new();
    ensure_go_session(&mut h, Some("abc"));
    assert_eq!(h["x-opencode-session"], "ses_abc");
}

#[test]
fn an_existing_session_is_left_alone() {
    let mut h = HeaderMap::new();
    h.insert("x-opencode-session", "ses_client".parse().unwrap());
    ensure_go_session(&mut h, Some("abc"));
    assert_eq!(h["x-opencode-session"], "ses_client");
}

#[test]
fn client_cache_headers_survive() {
    let mut client = HeaderMap::new();
    client.insert("prompt-cache-key", "keep-me".parse().unwrap());
    client.insert(
        "anthropic-beta",
        "prompt-caching-2024-07-31".parse().unwrap(),
    );
    let out = build_upstream_headers(&client, "k", "h", AuthStyle::BearerOnly, "ses_1");
    assert_eq!(out["prompt-cache-key"], "keep-me");
    assert_eq!(out["anthropic-beta"], "prompt-caching-2024-07-31");
}

#[test]
fn hop_by_hop_headers_are_dropped() {
    let mut client = HeaderMap::new();
    client.insert("connection", "keep-alive".parse().unwrap());
    client.insert("transfer-encoding", "chunked".parse().unwrap());
    client.insert("te", "trailers".parse().unwrap());
    let out = build_upstream_headers(&client, "k", "h", AuthStyle::BearerOnly, "ses_1");
    for name in ["connection", "transfer-encoding", "te"] {
        assert!(!out.contains_key(name), "{name} leaked upstream");
    }
    assert!(is_hop_by_hop("connection"));
    assert!(!is_hop_by_hop("prompt-cache-key"));
}

#[test]
fn passthrough_list_names_the_cache_headers() {
    for name in [
        "x-opencode-session",
        "prompt-cache-key",
        "cache-control",
        "anthropic-beta",
    ] {
        assert!(
            PASSTHROUGH.contains(&name),
            "{name} missing from PASSTHROUGH"
        );
    }
}

#[test]
fn generated_session_ids_are_unique_enough() {
    let a = generate_session_id();
    let b = generate_session_id();
    assert_ne!(a, b);
    assert!(a.starts_with("ses_"));
}

#[test]
fn accept_encoding_is_not_forwarded_and_repeated_headers_survive() {
    let mut client = HeaderMap::new();
    client.insert("accept-encoding", "gzip, br".parse().unwrap());
    client.append("anthropic-beta", "a".parse().unwrap());
    client.append("anthropic-beta", "b".parse().unwrap());
    let h = build_upstream_headers(
        &client,
        "oc_x",
        "opencode.ai",
        AuthStyle::BearerAndApiKey,
        "s",
    );
    assert!(
        !h.contains_key("accept-encoding"),
        "upstream must answer uncompressed"
    );
    assert_eq!(h.get_all("anthropic-beta").iter().count(), 2);
}
