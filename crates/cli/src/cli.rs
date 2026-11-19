use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use ocrouter_core::config::{Config, RoutingStrategy};
use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::provider::Provider;

pub enum RunOutcome {
    Text(String),
    Serve,
}

#[derive(Parser)]
#[command(
    name = "ocrouter",
    version,
    about = "Pool OpenCode accounts behind one local proxy"
)]
struct Cli {
    #[arg(long, global = true)]
    keys: Option<PathBuf>,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the proxy and control API
    Serve,
    /// Manage pooled API keys
    Key {
        #[command(subcommand)]
        action: KeyAction,
    },
    /// Model catalog operations
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Print the current routing status
    Status,
    /// Show or set the routing strategy
    Strategy { name: Option<String> },
    /// Install, remove, or inspect the launchd agent
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum KeyAction {
    Add {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        label: String,
        #[arg(long)]
        secret: String,
    },
    List,
    Rm {
        label: String,
    },
}

#[derive(Subcommand)]
enum ModelsAction {
    /// Print (or write) the opencode.json provider block
    Sync {
        #[arg(long)]
        write: bool,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    Install,
    Uninstall,
    Status,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

fn default_keys_path() -> PathBuf {
    home().join(".config").join("ocrouter").join("keys.toml")
}

fn default_config_path() -> PathBuf {
    home().join(".config").join("ocrouter").join("config.toml")
}

/// `(config.toml, keys.toml)`, honouring `OCROUTER_KEYS`.
pub fn default_paths() -> (PathBuf, PathBuf) {
    let keys = std::env::var("OCROUTER_KEYS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_keys_path());
    (default_config_path(), keys)
}

/// `~/.config/ocrouter/control.token`, created on first use so the control API
/// is never unauthenticated.
pub fn load_or_create_control_token() -> std::io::Result<String> {
    let path = home()
        .join(".config")
        .join("ocrouter")
        .join("control.token");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        // Older builds wrote a guessable time+pid token; replace those.
        if trimmed.len() >= TOKEN_HEX_LEN {
            return Ok(trimmed.to_string());
        }
    }
    let token = random_token()?;
    ocrouter_core::keys::write_private(&path, token.as_bytes())?;
    Ok(token)
}

const TOKEN_HEX_LEN: usize = 64;

/// 32 bytes from the OS CSPRNG, hex-encoded.
pub fn random_token() -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; TOKEN_HEX_LEN / 2];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// `--secret -` reads one line from stdin so a credential never has to appear
/// in shell history or `ps` output.
pub fn resolve_secret(value: &str, reader: &mut impl std::io::BufRead) -> std::io::Result<String> {
    if value == "-" {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    } else {
        Ok(value.to_string())
    }
}

fn parse_provider(value: &str) -> anyhow::Result<Provider> {
    match value {
        "opencode-go" | "go" => Ok(Provider::OpenCodeGo),
        "opencode-console" | "console" => Ok(Provider::OpenCodeConsole),
        "experiential" | "el" => Ok(Provider::Experiential),
        other => anyhow::bail!(
            "unknown provider '{other}' (use opencode-go, opencode-console, or experiential)"
        ),
    }
}

fn parse_strategy(value: &str) -> anyhow::Result<RoutingStrategy> {
    match value {
        "priority_failover" | "priority-failover" => Ok(RoutingStrategy::PriorityFailover),
        "round_robin" | "round-robin" => Ok(RoutingStrategy::RoundRobin),
        "session_affinity" | "session-affinity" => Ok(RoutingStrategy::SessionAffinity),
        other => anyhow::bail!("unknown strategy '{other}'"),
    }
}

fn strategy_name(strategy: RoutingStrategy) -> &'static str {
    match strategy {
        RoutingStrategy::PriorityFailover => "priority_failover",
        RoutingStrategy::RoundRobin => "round_robin",
        RoutingStrategy::SessionAffinity => "session_affinity",
    }
}

pub fn run(args: Vec<String>) -> anyhow::Result<RunOutcome> {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            use clap::error::ErrorKind;
            match err.kind() {
                // `--help` / `--version` are successful exits, not failures.
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    return Ok(RunOutcome::Text(err.to_string()));
                }
                _ => return Err(err.into()),
            }
        }
    };
    let keys_path = cli.keys.unwrap_or_else(default_keys_path);
    let config_path = cli.config.unwrap_or_else(default_config_path);

    match cli.command {
        Command::Serve => Ok(RunOutcome::Serve),
        Command::Key { action } => {
            let mut keys = KeyFile::load(&keys_path)?;
            match action {
                KeyAction::Add {
                    provider,
                    label,
                    secret,
                } => {
                    let provider = parse_provider(&provider)?;
                    let secret = resolve_secret(&secret, &mut std::io::stdin().lock())?;
                    let id = format!("{}:{}", provider.as_str(), label);
                    keys.add(KeyEntry {
                        provider,
                        label,
                        secret,
                        priority: 1,
                        weight: 1,
                        enabled: true,
                    });
                    keys.save(&keys_path)?;
                    Ok(RunOutcome::Text(format!(
                        "added {id} to {}\n",
                        keys_path.display()
                    )))
                }
                KeyAction::List => {
                    if keys.key.is_empty() {
                        return Ok(RunOutcome::Text(format!(
                            "no keys in {}\n",
                            keys_path.display()
                        )));
                    }
                    let mut out = String::new();
                    for masked in keys.masked() {
                        out.push_str(&format!(
                            "{}\t{}\t{}\n",
                            masked.label,
                            masked.provider.as_str(),
                            masked.preview
                        ));
                    }
                    Ok(RunOutcome::Text(out))
                }
                KeyAction::Rm { label } => {
                    let removed = keys.remove(&label);
                    keys.save(&keys_path)?;
                    if removed {
                        Ok(RunOutcome::Text(format!("removed {label}\n")))
                    } else {
                        anyhow::bail!("no key labelled '{label}'")
                    }
                }
            }
        }
        Command::Strategy { name } => {
            let mut config = Config::load(&config_path)?;
            match name {
                Some(name) => {
                    config.strategy = parse_strategy(&name)?;
                    config.save(&config_path)?;
                    Ok(RunOutcome::Text(format!(
                        "strategy set to {}\n",
                        strategy_name(config.strategy)
                    )))
                }
                None => Ok(RunOutcome::Text(format!(
                    "{}\n",
                    strategy_name(config.strategy)
                ))),
            }
        }
        Command::Status => {
            let config = Config::load(&config_path)?;
            let keys = KeyFile::load(&keys_path)?;
            Ok(RunOutcome::Text(format!(
                "proxy: 127.0.0.1:{}\ncontrol: 127.0.0.1:{}\nstrategy: {}\nkeys: {}\n",
                config.proxy_port,
                config.control_port,
                strategy_name(config.strategy),
                keys.key.len()
            )))
        }
        Command::Models { action } => match action {
            ModelsAction::Sync { write } => sync_models(&config_path, &keys_path, write),
        },
        Command::Service { action } => match action {
            ServiceAction::Install => Ok(RunOutcome::Text(install_service()?)),
            ServiceAction::Uninstall => Ok(RunOutcome::Text(uninstall_service()?)),
            ServiceAction::Status => Ok(RunOutcome::Text(service_status()?)),
        },
    }
}

/// Fetch every lane's model list. Zen is fetched only when a Zen key exists,
/// so its models are never listed without a way to call them.
pub async fn build_catalog(config: &Config, keys: &KeyFile) -> ocrouter_core::models::Catalog {
    use ocrouter_core::models::fetch_models;
    let client = reqwest::Client::new();
    let secret = |provider: Provider| {
        keys.key
            .iter()
            .find(|k| k.provider == provider)
            .map(|k| k.secret.clone())
    };
    let fetch = |provider: Provider, secret: Option<String>| {
        let upstream = ocrouter_core::provider::upstream_for(provider, config);
        let client = client.clone();
        async move {
            fetch_models(&client, &upstream, secret.as_deref())
                .await
                .unwrap_or_default()
        }
    };

    let go = fetch(Provider::OpenCodeGo, secret(Provider::OpenCodeGo)).await;
    let el = fetch(Provider::Experiential, secret(Provider::Experiential)).await;
    // Zen's list is public and answers 401 to a bad key, so ask without one.
    let zen = if secret(Provider::OpenCodeConsole).is_some() {
        fetch(Provider::OpenCodeConsole, None).await
    } else {
        Vec::new()
    };

    if go.is_empty() && el.is_empty() && zen.is_empty() {
        eprintln!("warning: could not fetch any model catalog; add a key or check connectivity");
    }
    ocrouter_core::models::Catalog::merged(go, el).with_console(zen)
}

fn sync_models(config_path: &Path, keys_path: &Path, write: bool) -> anyhow::Result<RunOutcome> {
    let config = Config::load(config_path)?;
    let keys = KeyFile::load(keys_path)?;
    let runtime = tokio::runtime::Runtime::new()?;

    let catalog = runtime.block_on(build_catalog(&config, &keys));
    let base = format!("http://127.0.0.1:{}/v1", config.proxy_port);
    let models = config.routing.exposed(&catalog.models);
    let snippet = ocrouter_core::models::opencode::generate_snippet(&models, &base);

    if !write {
        return Ok(RunOutcome::Text(format!(
            "{}\n",
            serde_json::to_string_pretty(&snippet)?
        )));
    }

    let target = ocrouter_core::models::opencode::find_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not locate opencode.json"))?;
    let backup = ocrouter_core::models::opencode::write_opencode_config(&target, &snippet)?;
    Ok(RunOutcome::Text(format!(
        "wrote {} models to {} (backup: {})\n",
        models.len(),
        target.display(),
        backup.display()
    )))
}

fn plist_path() -> PathBuf {
    home()
        .join("Library")
        .join("LaunchAgents")
        .join("com.ocrouter.daemon.plist")
}

fn install_service() -> anyhow::Result<String> {
    let exe = std::env::current_exe()?;
    let path = plist_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let logs = home().join("Library").join("Logs");
    std::fs::create_dir_all(&logs)?;
    let xml = |s: String| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ocrouter.daemon</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{out}</string>
  <key>StandardErrorPath</key><string>{err}</string>
</dict>
</plist>
"#,
        exe = xml(exe.display().to_string()),
        out = xml(logs.join("ocrouter.out.log").display().to_string()),
        err = xml(logs.join("ocrouter.err.log").display().to_string()),
    );
    std::fs::write(&path, plist)?;
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&path)
        .status();
    Ok(format!("installed launchd agent at {}\n", path.display()))
}

fn uninstall_service() -> anyhow::Result<String> {
    let path = plist_path();
    let _ = std::process::Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&path)
        .status();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(format!("removed launchd agent at {}\n", path.display()))
}

fn service_status() -> anyhow::Result<String> {
    let path = plist_path();
    if path.exists() {
        Ok(format!("installed: {}\n", path.display()))
    } else {
        Ok("not installed\n".to_string())
    }
}
