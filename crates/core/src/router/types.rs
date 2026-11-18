use serde::{Deserialize, Serialize};

use crate::provider::Provider;

pub use crate::config::RoutingStrategy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Exhausted,
    Disabled,
}

/// A pooled credential. Deliberately not `Serialize`: `secret` must never
/// reach the state file, the control API, or a log.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiKey {
    pub id: String,
    pub provider: Provider,
    pub label: String,
    pub secret: String,
    pub priority: u32,
    pub weight: u32,
    pub enabled: bool,
    pub status: KeyStatus,
    pub consecutive_errors: u32,
    pub cooldown_until_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeySelection {
    pub key: ApiKey,
    pub reason: String,
}

impl ApiKey {
    pub fn new_id(provider: Provider, label: &str) -> String {
        format!("{}:{}", provider.as_str(), label)
    }
}
