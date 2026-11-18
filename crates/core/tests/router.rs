use std::collections::HashSet;

use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::provider::Provider;
use ocrouter_core::router::key_manager::KeyManager;
use ocrouter_core::router::types::{ApiKey, KeyStatus, RoutingStrategy};

fn key(id: &str, priority: u32) -> ApiKey {
    ApiKey {
        id: id.to_string(),
        provider: Provider::OpenCodeGo,
        label: id.to_string(),
        secret: format!("oc_{id}"),
        priority,
        weight: 1,
        enabled: true,
        status: KeyStatus::Active,
        consecutive_errors: 0,
        cooldown_until_ms: None,
    }
}

fn none() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn priority_failover_prefers_lowest_priority_number() {
    let mut km = KeyManager::new(vec![key("a", 2), key("b", 1)]);
    let s = km.select(RoutingStrategy::PriorityFailover, &none(), 0).unwrap();
    assert_eq!(s.key.id, "b");
}

#[test]
fn excluded_and_cooling_keys_are_skipped() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 2)]);
    km.mark_exhausted("a", 10_000);
    assert_eq!(
        km.select(RoutingStrategy::PriorityFailover, &none(), 5_000).unwrap().key.id,
        "b"
    );

    let mut ex = HashSet::new();
    ex.insert("b".to_string());
    assert!(km.select(RoutingStrategy::PriorityFailover, &ex, 5_000).is_none());
}

#[test]
fn a_cooled_key_returns_when_its_cooldown_expires() {
    let mut km = KeyManager::new(vec![key("a", 1)]);
    km.mark_exhausted("a", 10_000);
    assert!(km.select(RoutingStrategy::PriorityFailover, &none(), 9_999).is_none());
    assert_eq!(
        km.select(RoutingStrategy::PriorityFailover, &none(), 10_000).unwrap().key.id,
        "a"
    );
}

#[test]
fn disabled_keys_are_never_selected() {
    let mut k = key("a", 1);
    k.enabled = false;
    let mut km = KeyManager::new(vec![k]);
    assert!(km.select(RoutingStrategy::PriorityFailover, &none(), 0).is_none());
}

#[test]
fn round_robin_advances_then_wraps() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 1)]);
    let mut seen = vec![];
    for _ in 0..4 {
        seen.push(km.select(RoutingStrategy::RoundRobin, &none(), 0).unwrap().key.id);
    }
    assert_eq!(seen, vec!["a", "b", "a", "b"]);
}

#[test]
fn from_keyfile_builds_stable_provider_qualified_ids() {
    let kf = KeyFile {
        key: vec![KeyEntry {
            provider: Provider::Experiential,
            label: "el-1".to_string(),
            secret: "xpl_x".to_string(),
            priority: 1,
            weight: 1,
            enabled: true,
        }],
    };
    let km = KeyManager::from_keyfile(&kf);
    assert_eq!(km.all()[0].id, "experiential:el-1");
}
