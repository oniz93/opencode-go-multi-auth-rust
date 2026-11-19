use std::sync::atomic::{AtomicU64, Ordering};

use http::{HeaderMap, HeaderValue};

use crate::provider::AuthStyle;

/// Headers that carry prompt-cache or session identity and must reach the
/// upstream untouched. Kept as a documented list; the builder copies every
/// header except the hop-by-hop set and the credentials it replaces.
pub const PASSTHROUGH: &[&str] = &[
    "x-opencode-session",
    "x-session-id",
    "prompt-cache-key",
    "x-session-affinity",
    "anthropic-beta",
    "cache-control",
    "user-agent",
    "content-type",
    "accept",
];

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "proxy-authorization",
    "proxy-authenticate",
    "host",
    "content-length",
];

pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.contains(&name.to_ascii_lowercase().as_str())
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A stable-per-generation, collision-resistant session id for clients that
/// send none. Go rejects a request without one (`400 MissingSessionID`).
pub fn generate_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("ses_{nanos:x}{n:x}")
}

/// Ensure the Go lane carries a session id, deriving one from `fallback` (the
/// client's `prompt-cache-key`) when possible so affinity stays stable.
pub fn ensure_go_session(headers: &mut HeaderMap, fallback: Option<&str>) {
    let present = headers
        .get("x-opencode-session")
        .and_then(|v| v.to_str().ok())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if present {
        return;
    }
    let id = match fallback {
        Some(seed) if !seed.is_empty() => format!("ses_{seed}"),
        _ => generate_session_id(),
    };
    if let Ok(value) = HeaderValue::from_str(&id) {
        headers.insert("x-opencode-session", value);
    }
}

pub fn build_upstream_headers(
    client: &HeaderMap,
    secret: &str,
    _host: &str,
    auth: AuthStyle,
    session: &str,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in client.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        // `accept-encoding` is dropped so the upstream answers uncompressed: the
        // proxy reads error bodies for quota signals and has no decompressor.
        if is_hop_by_hop(&lower)
            || lower == "authorization"
            || lower == "x-api-key"
            || lower == "accept-encoding"
        {
            continue;
        }
        out.append(name.clone(), value.clone());
    }

    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {secret}")) {
        out.insert("authorization", value);
    }
    if matches!(auth, AuthStyle::BearerAndApiKey) {
        if let Ok(value) = HeaderValue::from_str(secret) {
            out.insert("x-api-key", value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(session) {
        out.insert("x-opencode-session", value);
    }
    out
}
