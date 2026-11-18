use ocrouter_core::state::StateStore;

#[test]
fn write_is_atomic_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    let mut s = StateStore::new(p.clone(), 0);
    s.update(|st| st.saved_at_ms = 42);
    s.flush().unwrap();
    assert_eq!(StateStore::new(p.clone(), 0).load().saved_at_ms, 42);

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
}

#[test]
fn corrupt_file_loads_as_empty_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    std::fs::write(&p, "{ not json").unwrap();
    assert_eq!(StateStore::new(p, 0).load().saved_at_ms, 0);
}

#[test]
fn missing_file_loads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("nope.json");
    let s = StateStore::new(p, 0);
    assert_eq!(s.load().saved_at_ms, 0);
    assert!(s.load().keys.is_empty());
}

#[test]
fn update_marks_dirty_and_flush_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    let mut s = StateStore::new(p, 50);
    assert!(!s.is_dirty());
    s.update(|st| st.saved_at_ms = 1);
    assert!(s.is_dirty());
    s.flush().unwrap();
    assert!(!s.is_dirty());
}

#[test]
fn state_round_trips_key_health_without_any_secret() {
    use ocrouter_core::provider::Provider;
    use ocrouter_core::router::types::KeyStatus;
    use ocrouter_core::state::KeyHealth;

    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    let mut s = StateStore::new(p.clone(), 0);
    s.update(|st| {
        st.keys.push(KeyHealth {
            id: "opencode-go:go-1".to_string(),
            provider: Provider::OpenCodeGo,
            label: "go-1".to_string(),
            status: KeyStatus::Active,
            consecutive_errors: 0,
            cooldown_until_ms: None,
            tokens_today: 123,
            cost_accumulated: 0.5,
        });
    });
    s.flush().unwrap();

    let raw = std::fs::read_to_string(&p).unwrap();
    assert!(!raw.contains("oc_"), "state leaked a credential: {raw}");
    let reloaded = StateStore::new(p, 0).load();
    assert_eq!(reloaded.keys[0].id, "opencode-go:go-1");
    assert_eq!(reloaded.keys[0].tokens_today, 123);
}
