use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::provider::Provider;

fn entry(label: &str, secret: &str) -> KeyEntry {
    KeyEntry {
        provider: Provider::OpenCodeGo,
        label: label.to_string(),
        secret: secret.to_string(),
        priority: 1,
        weight: 1,
        enabled: true,
    }
}

#[test]
fn saved_file_is_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("keys.toml");
    let mut kf = KeyFile::default();
    kf.add(entry("go-1", "oc_secretvalue"));
    kf.save(&p).unwrap();

    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let raw = std::fs::read_to_string(&p).unwrap();
    assert!(raw.contains("oc_secretvalue"));
}

#[test]
fn round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("keys.toml");
    let mut kf = KeyFile::default();
    kf.add(entry("go-1", "oc_aaaaaaaaaaaa"));
    kf.add(KeyEntry {
        provider: Provider::Experiential,
        label: "el-1".to_string(),
        secret: "xpl_bbbbbbbbbbbb".to_string(),
        priority: 2,
        weight: 1,
        enabled: false,
    });
    kf.save(&p).unwrap();

    let loaded = KeyFile::load(&p).unwrap();
    assert_eq!(loaded.key.len(), 2);
    assert_eq!(loaded.key[0].provider, Provider::OpenCodeGo);
    assert_eq!(loaded.key[1].provider, Provider::Experiential);
    assert!(!loaded.key[1].enabled);
}

#[test]
fn masked_never_reveals_the_middle() {
    let kf = KeyFile {
        key: vec![KeyEntry {
            provider: Provider::Experiential,
            label: "el".to_string(),
            secret: "xpl_abcdef123456".to_string(),
            priority: 1,
            weight: 1,
            enabled: true,
        }],
    };
    let m = &kf.masked()[0];
    assert_eq!(m.preview, "xpl_…56");
    assert!(!m.preview.contains("cdef"));
}

#[test]
fn add_and_remove_by_label() {
    let mut kf = KeyFile::default();
    kf.add(entry("a", "oc_111111111111"));
    kf.add(entry("b", "oc_222222222222"));
    assert_eq!(kf.key.len(), 2);
    assert!(kf.remove("a"));
    assert_eq!(kf.key.len(), 1);
    assert_eq!(kf.key[0].label, "b");
    assert!(!kf.remove("missing"));
}

#[test]
fn missing_file_loads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let kf = KeyFile::load(&dir.path().join("nope.toml")).unwrap();
    assert!(kf.key.is_empty());
}

#[test]
fn saving_over_a_loose_file_tightens_it_to_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("keys.toml");
    std::fs::write(&p, "").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut kf = KeyFile::default();
    kf.add(entry("go-1", "oc_secretvalue"));
    kf.save(&p).unwrap();
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(
        !dir.path().join("keys.tmp").exists(),
        "no temp file is left behind"
    );
}

#[test]
fn adding_an_existing_label_replaces_it() {
    let mut kf = KeyFile::default();
    kf.add(entry("go-1", "oc_old"));
    kf.add(entry("go-1", "oc_new"));
    assert_eq!(kf.key.len(), 1);
    assert_eq!(kf.key[0].secret, "oc_new");
}
