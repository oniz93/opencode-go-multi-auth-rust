use ocrouter_cli::cli::{run, RunOutcome};

fn text(outcome: RunOutcome) -> String {
    match outcome {
        RunOutcome::Text(text) => text,
        RunOutcome::Serve => panic!("expected text output, got Serve"),
    }
}

fn arg_list(keys: &str, rest: &[&str]) -> Vec<String> {
    let mut args = vec![
        "ocrouter".to_string(),
        "--keys".to_string(),
        keys.to_string(),
    ];
    args.extend(rest.iter().map(|s| s.to_string()));
    args
}

#[test]
fn key_add_then_list_shows_only_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys.toml");
    let keys = keys.to_str().unwrap();

    run(arg_list(
        keys,
        &[
            "key",
            "add",
            "--provider",
            "opencode-go",
            "--label",
            "go-1",
            "--secret",
            "oc_abcdef123456",
        ],
    ))
    .unwrap();

    let out = text(run(arg_list(keys, &["key", "list"])).unwrap());
    assert!(out.contains("go-1"), "{out}");
    assert!(!out.contains("abcdef"), "secret leaked: {out}");
}

#[test]
fn key_rm_removes_by_label() {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys.toml");
    let keys = keys.to_str().unwrap();

    run(arg_list(
        keys,
        &[
            "key",
            "add",
            "--provider",
            "experiential",
            "--label",
            "el-1",
            "--secret",
            "xpl_zzzzzzzz",
        ],
    ))
    .unwrap();
    let out = text(run(arg_list(keys, &["key", "rm", "el-1"])).unwrap());
    assert!(out.contains("removed"), "{out}");
    let list = text(run(arg_list(keys, &["key", "list"])).unwrap());
    assert!(!list.contains("el-1"));
}

#[test]
fn strategy_reports_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys.toml");
    let out = text(run(arg_list(keys.to_str().unwrap(), &["strategy"])).unwrap());
    assert!(out.contains("priority_failover"), "{out}");
}

#[test]
fn serve_is_reported_as_a_run_mode() {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys.toml");
    assert!(matches!(
        run(arg_list(keys.to_str().unwrap(), &["serve"])).unwrap(),
        RunOutcome::Serve
    ));
}

#[test]
fn secret_dash_reads_one_line_from_stdin() {
    let mut input = std::io::Cursor::new(b"oc_from_stdin\n");
    assert_eq!(
        ocrouter_cli::cli::resolve_secret("-", &mut input).unwrap(),
        "oc_from_stdin"
    );

    let mut empty = std::io::Cursor::new(b"");
    assert_eq!(
        ocrouter_cli::cli::resolve_secret("literal", &mut empty).unwrap(),
        "literal"
    );
}

#[test]
fn version_and_help_are_successful_outputs() {
    let version = text(run(vec!["ocrouter".to_string(), "--version".to_string()]).unwrap());
    assert!(version.contains("ocrouter"), "{version}");

    let help = text(run(vec!["ocrouter".to_string(), "--help".to_string()]).unwrap());
    assert!(help.contains("serve"), "{help}");
}

#[test]
fn control_tokens_are_long_and_unpredictable() {
    let a = ocrouter_cli::cli::random_token().unwrap();
    let b = ocrouter_cli::cli::random_token().unwrap();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}

#[tokio::test(flavor = "multi_thread")]
async fn models_sync_runs_inside_the_binarys_runtime() {
    // `main` is `#[tokio::main]`; sync must not start or drop a nested runtime there.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "go_base_url = \"http://127.0.0.1:9\"\nel_base_url = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    let keys = dir.path().join("keys.toml");
    let mut args = arg_list(keys.to_str().unwrap(), &["keys", "list"]);
    run(args.clone()).unwrap();
    args.truncate(3);
    args.extend(["--config", config.to_str().unwrap(), "models", "sync"].map(String::from));
    let out = tokio::task::block_in_place(|| run(args)).unwrap();
    assert!(text(out).contains("ocrouter"));
}
