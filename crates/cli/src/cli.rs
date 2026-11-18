use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use ocrouter_core::config::{Config, RoutingStrategy};
use ocrouter_core::keys::{KeyEntry, KeyFile};
use ocrouter_core::provider::Provider;

use std::os::unix::fs::OpenOptionsExt;

pub enum RunOutcome {
    Text(String),
    Serve,
}

#[derive(Parser)]
#[command(name = "ocrouter", version, about = "Pool OpenCode accounts behind one local proxy")]
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
    Rm { label: String },
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
    let path = home().join(".config").join("ocrouter").join("control.token");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let token = format!("{nanos:x}{:x}", std::process::id());
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(token.as_bytes())?;
    }
    Ok(token)
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
        other => anyhow::bail!("unknown provider '{other}' (use opencode-go, opencode-console, or experiential)"),
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
    let cli = Cli::try_parse_from(args)?;
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
                    Ok(RunOutcome::Text(format!("added {id} to {}\n", keys_path.display())))
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

fn sync_models(config_path: &Path, keys_path: &Path, write: bool) -> anyhow::Result<RunOutcome> {
    let config = Config::load(config_path)?;
    let keys = KeyFile::load(keys_path)?;
    let runtime = tokio::runtime::Runtime::new()?;

    let secret = keys
        .key
        .iter()
        .find(|k| k.provider == Provider::OpenCodeGo)
        .map(|k| k.secret.clone());

    let client = reqwest::Client::new();
    let go_upstream = ocrouter_core::provider::upstream_for(Provider::OpenCodeGo, &config);
    let el_upstream = ocrouter_core::provider::upstream_for(Provider::Experiential, &config);

    let (go, el) = runtime.block_on(async {
        let go = ocrouter_core::models::fetch_models(&client, &go_upstream, secret.as_deref())
            .await
            .unwrap_or_default();
        let el = ocrouter_core::models::fetch_models(
            &client,
            &el_upstream,
            keys.key
                .iter()
                .find(|k| k.provider == Provider::Experiential)
                .map(|k| k.secret.as_str()),
        )
        .await
        .unwrap_or_default();
        (go, el)
    });

    let catalog = ocrouter_core::models::Catalog::merged(go, el);
    let base = format!("http://127.0.0.1:{}/v1", config.proxy_port);
    let snippet = ocrouter_core::models::opencode::generate_snippet(&catalog.models, &base);

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
        catalog.models.len(),
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
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ocrouter.daemon</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/ocrouter.out.log</string>
  <key>StandardErrorPath</key><string>/tmp/ocrouter.err.log</string>
</dict>
</plist>
"#,
        exe.display()
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
