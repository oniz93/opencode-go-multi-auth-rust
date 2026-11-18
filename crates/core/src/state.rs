use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::events::EventRecord;
use crate::provider::Provider;
use crate::router::types::KeyStatus;

/// Per-key health. Deliberately carries **no secret**: this struct is what
/// lands in `state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyHealth {
    pub id: String,
    pub provider: Provider,
    pub label: String,
    pub status: KeyStatus,
    pub consecutive_errors: u32,
    pub cooldown_until_ms: Option<u64>,
    pub tokens_today: u64,
    pub cost_accumulated: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrgUsage {
    pub org: String,
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouterState {
    #[serde(default)]
    pub keys: Vec<KeyHealth>,
    #[serde(default)]
    pub allowance: Vec<OrgUsage>,
    #[serde(default)]
    pub recent_events: Vec<EventRecord>,
    #[serde(default)]
    pub saved_at_ms: u64,
}

/// Debounced, atomically-written state. A corrupt or missing file loads as
/// empty rather than failing.
pub struct StateStore {
    path: PathBuf,
    debounce_ms: u64,
    state: RouterState,
    dirty: bool,
}

impl StateStore {
    pub fn new(path: PathBuf, debounce_ms: u64) -> Self {
        let state = Self::read(&path);
        StateStore {
            path,
            debounce_ms,
            state,
            dirty: false,
        }
    }

    fn read(path: &Path) -> RouterState {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn load(&self) -> RouterState {
        self.state.clone()
    }

    pub fn update(&mut self, f: impl FnOnce(&mut RouterState)) {
        f(&mut self.state);
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn debounce_ms(&self) -> u64 {
        self.debounce_ms
    }

    /// Write to a sibling `.tmp` then rename over the target, so a crash
    /// mid-write can never leave a half-written state file.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.state)?)?;
        std::fs::rename(&tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}
