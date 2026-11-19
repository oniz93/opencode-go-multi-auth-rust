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
    let s = km
        .select(RoutingStrategy::PriorityFailover, &none(), 0)
        .unwrap();
    assert_eq!(s.key.id, "b");
}

#[test]
fn excluded_and_cooling_keys_are_skipped() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 2)]);
    km.mark_exhausted("a", 10_000);
    assert_eq!(
        km.select(RoutingStrategy::PriorityFailover, &none(), 5_000)
            .unwrap()
            .key
            .id,
        "b"
    );

    let mut ex = HashSet::new();
    ex.insert("b".to_string());
    assert!(km
        .select(RoutingStrategy::PriorityFailover, &ex, 5_000)
        .is_none());
}

#[test]
fn a_cooled_key_returns_when_its_cooldown_expires() {
    let mut km = KeyManager::new(vec![key("a", 1)]);
    km.mark_exhausted("a", 10_000);
    assert!(km
        .select(RoutingStrategy::PriorityFailover, &none(), 9_999)
        .is_none());
    assert_eq!(
        km.select(RoutingStrategy::PriorityFailover, &none(), 10_000)
            .unwrap()
            .key
            .id,
        "a"
    );
}

#[test]
fn disabled_keys_are_never_selected() {
    let mut k = key("a", 1);
    k.enabled = false;
    let mut km = KeyManager::new(vec![k]);
    assert!(km
        .select(RoutingStrategy::PriorityFailover, &none(), 0)
        .is_none());
}

#[test]
fn round_robin_advances_then_wraps() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 1)]);
    let mut seen = vec![];
    for _ in 0..4 {
        seen.push(
            km.select(RoutingStrategy::RoundRobin, &none(), 0)
                .unwrap()
                .key
                .id,
        );
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

#[test]
fn a_cooling_key_is_not_selectable_until_its_cooldown_lapses() {
    let mut km = KeyManager::new(vec![key("a", 1)]);
    km.mark_exhausted("a", 1_000);
    assert!(!km.is_selectable("a", &none(), 999));
    assert!(km.is_selectable("a", &none(), 1_000));
    assert!(!km.is_selectable("missing", &none(), 0));
}

#[test]
fn syncing_from_the_key_file_keeps_health_and_follows_edits() {
    let entry = |label: &str| KeyEntry {
        provider: Provider::OpenCodeGo,
        label: label.to_string(),
        secret: format!("oc_{label}"),
        priority: 1,
        weight: 1,
        enabled: true,
    };
    let mut km = KeyManager::from_keyfile(&KeyFile {
        key: vec![entry("a"), entry("b")],
    });
    km.mark_exhausted("opencode-go:a", 5_000);

    km.sync_from_keyfile(&KeyFile {
        key: vec![entry("a"), entry("c")],
    });

    let ids: Vec<&str> = km.all().iter().map(|k| k.id.as_str()).collect();
    assert_eq!(ids, ["opencode-go:a", "opencode-go:c"]);
    assert_eq!(
        km.get("opencode-go:a").unwrap().cooldown_until_ms,
        Some(5_000)
    );
}
