use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde_json::Value;

/// Substrings that mark a 429 as a real quota wall rather than a transient
/// rate limit. Matched case-insensitively against the whole body.
const QUOTA_MARKERS: &[&str] = &[
    "insufficient_quota",
    "insufficient_credits",
    "usage_not_included",
    "freeusagelimit",
    "free_limit_reached",
    "model_requires_payment",
    "credit balance",
    "exhausted",
];

pub fn is_quota_error(status: u16, _headers: &HeaderMap, body: &str) -> bool {
    if status == 402 {
        return true;
    }
    if status != 429 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    QUOTA_MARKERS.iter().any(|m| lower.contains(m))
}

fn parse_duration_text(s: &str) -> Option<u64> {
    let lower = s.to_ascii_lowercase();
    let idx = lower.find("retry after")?;
    let rest = lower[idx + "retry after".len()..].trim_start();
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = num.parse().ok()?;
    let after_num = rest[num.len()..].trim_start();
    let unit = after_num.chars().next()?;
    let ms = match unit {
        's' => n * 1_000,
        'm' => n * 60_000,
        'h' => n * 3_600_000,
        'd' => n * 86_400_000,
        _ => return None,
    };
    Some(ms)
}

fn parse_http_date_ms(value: &str) -> Option<u64> {
    DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis().max(0) as u64)
}

pub fn resolve_cooldown_ms(
    headers: &HeaderMap,
    body: &str,
    now_ms: u64,
    fallback_ms: u64,
) -> u64 {
    let resolved = from_headers(headers, now_ms).or_else(|| from_body(body, now_ms));
    resolved.unwrap_or(fallback_ms).max(1_000)
}

fn from_headers(headers: &HeaderMap, now_ms: u64) -> Option<u64> {
    for (name, value) in headers.iter() {
        let n = name.as_str().to_ascii_lowercase();
        let v = value.to_str().unwrap_or("").trim();
        if n == "retry-after" {
            // Either a delta-seconds value or an HTTP-date absolute instant.
            if let Ok(secs) = v.parse::<u64>() {
                return Some(secs * 1_000);
            }
            if let Some(abs) = parse_http_date_ms(v) {
                return Some(abs.saturating_sub(now_ms));
            }
        } else if n.starts_with("x-codex-") && n.ends_with("-reset-after-seconds") {
            if let Ok(secs) = v.parse::<u64>() {
                return Some(secs * 1_000);
            }
        } else if n.starts_with("x-codex-") && n.ends_with("-reset-at") {
            if let Some(ts) = parse_http_date_ms(v).or_else(|| rfc3339_ms(v)) {
                return Some(ts.saturating_sub(now_ms));
            }
        }
    }
    None
}

fn rfc3339_ms(value: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis().max(0) as u64)
}

fn error_field(body: &str) -> Option<Value> {
    let json: Value = serde_json::from_str(body).ok()?;
    json.get("error").cloned().or(Some(json))
}

fn from_body(body: &str, now_ms: u64) -> Option<u64> {
    if let Some(err) = error_field(body) {
        if let Some(ms) = err.get("retry_after_ms").and_then(Value::as_u64) {
            return Some(ms);
        }
        if let Some(secs) = err.get("retry_after").and_then(Value::as_u64) {
            return Some(secs * 1_000);
        }
        if let Some(at) = err.get("resets_at").and_then(Value::as_str) {
            if let Some(ts) = rfc3339_ms(at).or_else(|| parse_http_date_ms(at)) {
                return Some(ts.saturating_sub(now_ms));
            }
        }
    }
    parse_duration_text(body)
}

pub fn extract_quota_message(body: &str, status: u16) -> String {
    if let Some(err) = error_field(body) {
        let code = err.get("code").and_then(Value::as_str);
        let ty = err.get("type").and_then(Value::as_str);
        let message = err.get("message").and_then(Value::as_str);
        let parts: Vec<&str> = [code, ty, message].into_iter().flatten().collect();
        if !parts.is_empty() {
            return parts.join(" · ");
        }
    }
    format!("HTTP {status}")
}
