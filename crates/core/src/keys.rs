use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::provider::Provider;

fn default_priority() -> u32 {
    1
}

fn default_weight() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyEntry {
    pub provider: Provider,
    pub label: String,
    pub secret: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyFile {
    #[serde(default)]
    pub key: Vec<KeyEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaskedKey {
    pub label: String,
    pub provider: Provider,
    pub preview: String,
}

fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 6 {
        return "…".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}…{tail}")
}

/// Mask a credential for display: first 4 and last 2 characters only.
pub fn mask_secret(secret: &str) -> String {
    mask(secret)
}

impl KeyFile {
    pub fn load(path: &Path) -> anyhow::Result<KeyFile> {
        if !path.exists() {
            return Ok(KeyFile::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str::<KeyFile>(&raw)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(raw.as_bytes())?;
        Ok(())
    }

    pub fn add(&mut self, entry: KeyEntry) {
        self.key.push(entry);
    }

    pub fn remove(&mut self, label: &str) -> bool {
        let before = self.key.len();
        self.key.retain(|k| k.label != label);
        self.key.len() != before
    }

    pub fn masked(&self) -> Vec<MaskedKey> {
        self.key
            .iter()
            .map(|k| MaskedKey {
                label: k.label.clone(),
                provider: k.provider,
                preview: mask(&k.secret),
            })
            .collect()
    }
}
