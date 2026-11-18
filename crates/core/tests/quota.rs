use http::HeaderMap;
use ocrouter_core::proxy::quota::{extract_quota_message, is_quota_error, resolve_cooldown_ms};

#[test]
fn generic_429_is_not_quota() {
    assert!(!is_quota_error(
        429,
        &HeaderMap::new(),
        r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#
    ));
}

#[test]
fn free_limit_reached_is_quota() {
    assert!(is_quota_error(
        429,
        &HeaderMap::new(),
        r#"{"error":{"code":"free_limit_reached"}}"#
    ));
    assert!(is_quota_error(
        402,
        &HeaderMap::new(),
        r#"{"error":{"message":"credit balance exhausted"}}"#
    ));
    assert!(is_quota_error(
        429,
        &HeaderMap::new(),
        r#"{"error":{"code":"insufficient_quota"}}"#
    ));
    assert!(is_quota_error(
        429,
        &HeaderMap::new(),
        r#"{"error":{"code":"insufficient_credits"}}"#
    ));
}

#[test]
fn retry_after_header_wins() {
    let mut h = HeaderMap::new();
    h.insert("retry-after", "120".parse().unwrap());
    assert_eq!(resolve_cooldown_ms(&h, "", 0, 9_999_999), 120_000);
}

#[test]
fn retry_after_http_date_is_parsed() {
    let mut h = HeaderMap::new();
    // 2026-11-18T00:02:00Z == 1_763_433_720_000 ms; now = 00:00:00Z
    h.insert("retry-after", "Wed, 18 Nov 2026 00:02:00 GMT".parse().unwrap());
    let now = 1_794_960_000_000;
    assert_eq!(resolve_cooldown_ms(&h, "", now, 9), 120_000);
}

#[test]
fn codex_reset_header_is_used() {
    let mut h = HeaderMap::new();
    h.insert("x-codex-primary-reset-after-seconds", "90".parse().unwrap());
    assert_eq!(resolve_cooldown_ms(&h, "", 0, 9), 90_000);
}

#[test]
fn body_retry_after_ms_is_used_when_header_absent() {
    assert_eq!(
        resolve_cooldown_ms(
            &HeaderMap::new(),
            r#"{"error":{"retry_after_ms":60000}}"#,
            0,
            9
        ),
        60_000
    );
}

#[test]
fn body_resets_at_is_used_when_ms_absent() {
    let now = 1_794_960_000_000u64;
    assert_eq!(
        resolve_cooldown_ms(
            &HeaderMap::new(),
            r#"{"error":{"resets_at":"2026-11-18T00:05:00Z"}}"#,
            now,
            9
        ),
        300_000
    );
}

#[test]
fn retry_after_text_is_used_as_a_last_resort() {
    assert_eq!(
        resolve_cooldown_ms(
            &HeaderMap::new(),
            r#"{"error":{"message":"Please retry after 2 h"}}"#,
            0,
            9
        ),
        2 * 3_600_000
    );
    assert_eq!(
        resolve_cooldown_ms(
            &HeaderMap::new(),
            r#"{"error":{"message":"rate limited, retry after 30 m"}}"#,
            0,
            9
        ),
        30 * 60_000
    );
}

#[test]
fn fallback_used_when_nothing_present() {
    assert_eq!(
        resolve_cooldown_ms(&HeaderMap::new(), "{}", 0, 5 * 3_600_000),
        5 * 3_600_000
    );
}

#[test]
fn a_past_retry_after_is_floored_not_negative() {
    let mut h = HeaderMap::new();
    h.insert("retry-after", "Wed, 18 Nov 2026 00:00:00 GMT".parse().unwrap());
    let now = 1_794_960_000_000; // exactly that instant
    assert!(resolve_cooldown_ms(&h, "", now, 5_000) >= 1_000);
}

#[test]
fn quota_message_prefers_code_then_type_then_message() {
    let msg = extract_quota_message(r#"{"error":{"code":"insufficient_quota","type":"x","message":"y"}}"#, 429);
    assert!(msg.contains("insufficient_quota"));
    assert_eq!(extract_quota_message("not json", 429), "HTTP 429");
}
