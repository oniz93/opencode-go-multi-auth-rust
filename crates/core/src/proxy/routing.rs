use crate::models::Lane;

/// Lane prefixes on the client-facing URL. Explicit and debuggable; the unified
/// lane (no prefix) routes per model in the request handler.
pub const LANE_PREFIXES: [(&str, Lane); 3] = [
    ("/go", Lane::Go),
    ("/el", Lane::Experiential),
    ("/oc", Lane::Console),
];

/// Split an incoming path (`/go/v1/chat/completions`) into the lane it names and
/// the path to forward. The unified lane (no prefix) returns `None`.
pub fn strip_lane_prefix(path: &str) -> (Option<Lane>, String) {
    for (prefix, lane) in LANE_PREFIXES {
        if path == prefix {
            return (Some(lane), "/".to_string());
        }
        if let Some(rest) = path.strip_prefix(prefix) {
            if rest.starts_with('/') {
                return (Some(lane), rest.to_string());
            }
        }
    }
    (None, path.to_string())
}

/// Join an upstream base URL with an incoming path, never duplicating `/v1`.
pub fn build_upstream_url(base: &str, incoming: &str) -> String {
    let base_no_query = base.split('?').next().unwrap_or(base);
    let (incoming_path, incoming_query) = match incoming.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (incoming, None),
    };

    let base_trimmed = base_no_query.trim_end_matches('/');
    let needs_version = !incoming_path.starts_with("/v1/") && incoming_path != "/v1";
    let normalized = if needs_version {
        format!("/v1{incoming_path}")
    } else {
        incoming_path.to_string()
    };

    let deduped = if base_trimmed.ends_with("/v1") && normalized.starts_with("/v1") {
        let rest = &normalized[3..];
        if rest.is_empty() {
            "/".to_string()
        } else {
            rest.to_string()
        }
    } else {
        normalized
    };

    let mut url = format!("{base_trimmed}{deduped}");
    if let Some(query) = incoming_query {
        url.push('?');
        url.push_str(query);
    }
    url
}
