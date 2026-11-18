use http::HeaderMap;
use ocrouter_core::router::session_affinity::SessionAffinity;

#[test]
fn extracts_first_present_session_header() {
    let mut h = HeaderMap::new();
    h.insert("prompt-cache-key", "abc".parse().unwrap());
    assert_eq!(SessionAffinity::extract_session_key(&h).unwrap(), "abc");

    h.insert("x-opencode-session", "ses_1".parse().unwrap());
    assert_eq!(SessionAffinity::extract_session_key(&h).unwrap(), "ses_1");
}

#[test]
fn empty_header_values_are_ignored() {
    let mut h = HeaderMap::new();
    h.insert("x-opencode-session", "".parse().unwrap());
    h.insert("prompt-cache-key", "fallback".parse().unwrap());
    assert_eq!(SessionAffinity::extract_session_key(&h).unwrap(), "fallback");
    assert!(SessionAffinity::extract_session_key(&HeaderMap::new()).is_none());
}

#[test]
fn remembers_and_returns_preferred_key() {
    let mut sa = SessionAffinity::new();
    sa.set("s1", "key-a");
    assert_eq!(sa.preferred("s1").unwrap(), "key-a");
    assert!(sa.preferred("s2").is_none());
}

#[test]
fn later_sets_repoint_the_same_session() {
    let mut sa = SessionAffinity::new();
    sa.set("s1", "key-a");
    sa.set("s1", "key-b");
    assert_eq!(sa.preferred("s1").unwrap(), "key-b");
    assert_eq!(sa.len(), 1);
}

#[test]
fn evicts_oldest_beyond_capacity() {
    let mut sa = SessionAffinity::new();
    for i in 0..4097 {
        sa.set(&format!("s{i}"), "k");
    }
    assert!(sa.len() <= 4096);
    assert!(sa.preferred("s0").is_none());
    assert!(sa.preferred("s4096").is_some());
}
