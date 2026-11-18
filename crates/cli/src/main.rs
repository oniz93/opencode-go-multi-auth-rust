use ocrouter_cli::cli::{default_paths, load_or_create_control_token, run, RunOutcome};
use ocrouter_cli::control_api::{AppState, ControlApi};
use ocrouter_cli::engine::Engine;
use ocrouter_cli::proxy_server::ProxyServer;
use ocrouter_core::config::Config;
use ocrouter_core::keys::KeyFile;
use ocrouter_core::models::{fetch_models, Catalog};
use ocrouter_core::provider::{upstream_for, Provider};

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

async fn build_catalog(config: &Config, keys: &KeyFile) -> Catalog {
    let client = reqwest::Client::new();
    let go_secret = keys
        .key
        .iter()
        .find(|k| k.provider == Provider::OpenCodeGo)
        .map(|k| k.secret.clone());
    let el_secret = keys
        .key
        .iter()
        .find(|k| k.provider == Provider::Experiential)
        .map(|k| k.secret.clone());

    let go_upstream = upstream_for(Provider::OpenCodeGo, config);
    let el_upstream = upstream_for(Provider::Experiential, config);

    let go = fetch_models(&client, &go_upstream, go_secret.as_deref())
        .await
        .unwrap_or_default();
    let el = fetch_models(&client, &el_upstream, el_secret.as_deref())
        .await
        .unwrap_or_default();

    if go.is_empty() && el.is_empty() {
        eprintln!("warning: could not fetch any model catalog; add a key or check connectivity");
    }
    Catalog::merged(go, el)
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
    json.get("org_id").and_then(|v| v.as_str()).map(str::to_string)
}

async fn serve() -> anyhow::Result<()> {
    let (config_path, keys_path) = default_paths();
    let config = Config::load(&config_path)?;
    let keys = KeyFile::load(&keys_path)?;

    let catalog = build_catalog(&config, &keys).await;
    let mut engine = Engine::new(config.clone(), &keys, catalog);

    if let Some(el) = keys
        .key
        .iter()
        .find(|k| k.provider == Provider::Experiential)
    {
        match resolve_el_org(&config, &el.secret).await {
            Some(org) => engine.ensure_el_org(Some(org)),
            None => {
                eprintln!(
                    "warning: could not resolve the Experiential org via /api/whoami; \
                     using a local allowance bucket so the free lane still works"
                );
                engine.ensure_el_org(None);
            }
        }
    }

    let shared = engine.shared();
    let proxy = ProxyServer::new(engine);

    let proxy_listener =
        tokio::net::TcpListener::bind(("127.0.0.1", config.proxy_port)).await?;
    let control_listener =
        tokio::net::TcpListener::bind(("127.0.0.1", config.control_port)).await?;

    let token = load_or_create_control_token()?;
    let state = AppState {
        shared: shared.clone(),
        token: String::new(),
    };
    let api = ControlApi::new(state, token).router();

    eprintln!(
        "ocrouter: proxy on 127.0.0.1:{}, control on 127.0.0.1:{}, {} models, {} keys",
        config.proxy_port,
        config.control_port,
        shared.catalog.models.len(),
        keys.key.len()
    );

    tokio::try_join!(
        proxy.serve(proxy_listener),
        async move { axum::serve(control_listener, api).await.map_err(anyhow::Error::from) },
    )?;
    Ok(())
}
