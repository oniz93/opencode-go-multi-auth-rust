use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::{Lane, ModelInfo};
use crate::provider::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    #[default]
    PriorityFailover,
    RoundRobin,
    SessionAffinity,
}

/// One hop of a route: a key id (`provider:label`) and the model id that
/// key's upstream knows the requested model as.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteStep {
    pub key: String,
    pub model: String,
}

/// An exposed model id served by an ordered chain of keys. Each step is used
/// until the upstream answers 429/402, then the next one takes over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub model: String,
    #[serde(default)]
    pub steps: Vec<RouteStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routing {
    /// Model ids listed to clients. Empty means every catalog model.
    pub expose: Vec<String>,
    pub route: Vec<Route>,
}

impl Routing {
    pub fn route_for(&self, model: &str) -> Option<&Route> {
        self.route
            .iter()
            .find(|r| r.model == model && !r.steps.is_empty())
    }

    /// What `/v1/models` and the OpenCode snippet list: the catalog plus every
    /// route alias, narrowed to `expose` when it is set.
    pub fn exposed(&self, models: &[ModelInfo]) -> Vec<ModelInfo> {
        let mut out = models.to_vec();
        for route in &self.route {
            if out.iter().any(|m| m.id == route.model) {
                continue;
            }
            // An alias borrows its first known step's limits so OpenCode sees real ones.
            let mut entry = route
                .steps
                .iter()
                .find_map(|s| models.iter().find(|m| m.id == s.model))
                .cloned()
                .unwrap_or(ModelInfo {
                    id: String::new(),
                    name: String::new(),
                    lane: Lane::Go,
                    provider: Provider::OpenCodeGo,
                    context_limit: None,
                    output_limit: None,
                    input_price: None,
                    output_price: None,
                    supports_tools: true,
                    modalities_in: vec!["text".to_string()],
                });
            entry.id = route.model.clone();
            entry.name = route.model.clone();
            out.push(entry);
        }
        if !self.expose.is_empty() {
            out.retain(|m| self.expose.contains(&m.id));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub proxy_port: u16,
    pub control_port: u16,
    pub strategy: RoutingStrategy,
    pub go_base_url: String,
    pub oc_base_url: String,
    pub el_base_url: String,
    pub el_management_url: String,
    pub el_daily_token_cap: u64,
    pub el_safety_margin: f64,
    pub el_allow_paid: bool,
    pub cooldown_ms: u64,
    pub circuit_breaker_threshold: u32,
    /// How long an open breaker sidelines a key. Short on purpose: it guards
    /// against transport errors and 5xx, not quota (that is `cooldown_ms`).
    pub breaker_open_ms: u64,
    pub log_level: String,
    pub keys_path: Option<PathBuf>,
    pub state_path: Option<PathBuf>,
    pub routing: Routing,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            proxy_port: 18905,
            control_port: 18904,
            strategy: RoutingStrategy::default(),
            go_base_url: "https://opencode.ai/inference/go".to_string(),
            oc_base_url: "https://opencode.ai/inference".to_string(),
            el_base_url: "https://api.experientiallabs.ai/v1".to_string(),
            el_management_url: "https://api.experientiallabs.ai/api".to_string(),
            el_daily_token_cap: 1_000_000,
            el_safety_margin: 0.05,
            el_allow_paid: false,
            cooldown_ms: 5 * 60 * 60 * 1000,
            circuit_breaker_threshold: 3,
            breaker_open_ms: 60_000,
            log_level: "info".to_string(),
            keys_path: None,
            state_path: None,
            routing: Routing::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let mut cfg = Config::read(path)?;
        cfg.apply_env();
        Ok(cfg)
    }

    /// The file as written, without env overrides, so a save never bakes them in.
    pub fn read(path: &Path) -> anyhow::Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str::<Config>(&raw)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("OCROUTER_PROXY_PORT") {
            if let Ok(p) = v.parse() {
                self.proxy_port = p;
            }
        }
        if let Ok(v) = std::env::var("OCROUTER_CONTROL_PORT") {
            if let Ok(p) = v.parse() {
                self.control_port = p;
            }
        }
        if let Ok(v) = std::env::var("OCROUTER_GO_BASE_URL") {
            self.go_base_url = v;
        }
        if let Ok(v) = std::env::var("OCROUTER_EL_BASE_URL") {
            self.el_base_url = v;
        }
        if let Ok(v) = std::env::var("OCROUTER_KEYS") {
            self.keys_path = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("OCROUTER_STATE") {
            self.state_path = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("OCROUTER_LOG_LEVEL") {
            self.log_level = v;
        }
    }
}
