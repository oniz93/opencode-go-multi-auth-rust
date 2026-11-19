use ocrouter_cli::cli::{build_catalog, default_paths, load_or_create_control_token, run, RunOutcome};
use ocrouter_cli::control_api::{AppState, ControlApi};
use ocrouter_cli::engine::Engine;
use ocrouter_cli::proxy_server::ProxyServer;
use ocrouter_core::config::Config;
use ocrouter_core::keys::KeyFile;
use ocrouter_core::provider::Provider;
use ocrouter_core::router::types::ApiKey;
use ocrouter_core::state::StateStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match run(args)? {
        RunOutcome::Text(text) => {
            print!("{text}");
            Ok(())
        }
        RunOutcome::Serve => serve().await,
    }
}

async fn resolve_el_org(config: &Config, secret: &str) -> Option<String> {
    let url = format!("{}/whoami", config.el_management_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(secret)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let json: serde_json::Value = response.json().await.ok()?;
    json.get("org_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

async fn serve() -> anyhow::Result<()> {
    let (config_path, keys_path) = default_paths();
    let config = Config::load(&config_path)?;
    let keys = KeyFile::load(&keys_path)?;

    let catalog = build_catalog(&config, &keys).await;
    let mut engine = Engine::new(config.clone(), &keys, catalog);

    // The allowance is per org, so every EL key gets its own org lookup.
    let mut first_org = None;
    for el in keys
        .key
        .iter()
        .filter(|k| k.provider == Provider::Experiential)
    {
        match resolve_el_org(&config, &el.secret).await {
            Some(org) => {
                engine.set_key_org(&ApiKey::new_id(el.provider, &el.label), org.clone());
                first_org.get_or_insert(org);
            }
            None => eprintln!(
                "warning: could not resolve the Experiential org for {} via /api/whoami; \
                 using a local allowance bucket so the free lane still works",
                el.label
            ),
        }
    }
    // Also the bucket for EL keys added while running.
    engine.ensure_el_org(first_org);

    let shared = engine.shared();
    let state_path = config
        .state_path
        .clone()
        .unwrap_or_else(|| keys_path.with_file_name("state.json"));
    let mut store = StateStore::new(state_path, 5_000);
    shared.restore(&store.load(), now_ms());
    let proxy = ProxyServer::new(engine);

    let proxy_listener = tokio::net::TcpListener::bind(("127.0.0.1", config.proxy_port)).await?;
    let control_listener =
        tokio::net::TcpListener::bind(("127.0.0.1", config.control_port)).await?;

    let token = load_or_create_control_token()?;
    let state = AppState {
        shared: shared.clone(),
        token: String::new(),
        keys_path: Some(keys_path.clone()),
        config_path: Some(config_path.clone()),
    };
    let api = ControlApi::new(state, token).router();

    eprintln!(
        "ocrouter: proxy on 127.0.0.1:{}, control on 127.0.0.1:{}, {} models, {} keys",
        config.proxy_port,
        config.control_port,
        shared.catalog().models.len(),
        keys.key.len()
    );

    // Housekeeping: persist health, pick up `keys.toml` edits, refresh models.
    {
        let shared = shared.clone();
        let config = config.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            let mut keys_mtime = mtime(&keys_path);
            let mut last_saved = String::new();
            let mut last_catalog_ms = now_ms();
            loop {
                tick.tick().await;
                let now = now_ms();

                let mut snapshot = shared.health_snapshot();
                let fingerprint = serde_json::to_string(&snapshot).unwrap_or_default();
                if fingerprint != last_saved {
                    snapshot.saved_at_ms = now;
                    store.update(|s| *s = snapshot);
                    match store.flush() {
                        Ok(()) => last_saved = fingerprint,
                        Err(e) => eprintln!("warning: could not save state: {e}"),
                    }
                }

                let current = mtime(&keys_path);
                if current != keys_mtime {
                    keys_mtime = current;
                    match KeyFile::load(&keys_path) {
                        Ok(file) => {
                            if let Ok(mut keys) = shared.keys.lock() {
                                keys.sync_from_keyfile(&file);
                            }
                            eprintln!("ocrouter: reloaded {} keys", file.key.len());
                        }
                        Err(e) => eprintln!("warning: could not reload keys: {e}"),
                    }
                }

                // A start before the network is up leaves an empty catalog;
                // retry each minute until it fills, then refresh hourly.
                let every = if shared.catalog().models.is_empty() {
                    60_000
                } else {
                    3_600_000
                };
                if now.saturating_sub(last_catalog_ms) >= every {
                    last_catalog_ms = now;
                    let keys = KeyFile::load(&keys_path).unwrap_or_default();
                    let catalog = build_catalog(&config, &keys).await;
                    if !catalog.models.is_empty() {
                        shared.set_catalog(catalog);
                    }
                }
            }
        });
    }

    tokio::try_join!(proxy.serve(proxy_listener), async move {
        axum::serve(control_listener, api)
            .await
            .map_err(anyhow::Error::from)
    },)?;
    Ok(())
}
