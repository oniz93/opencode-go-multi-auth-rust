use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::catalog::ModelInfo;

pub const PROVIDER_ID: &str = "ocrouter";
pub const PACKAGE: &str = "@opencode/ai/providers/openai-compatible";

fn model_entry(model: &ModelInfo) -> Value {
    let mut obj = Map::new();
    obj.insert("name".to_string(), json!(model.name));
    if model.context_limit.is_some() || model.output_limit.is_some() {
        obj.insert(
            "limit".to_string(),
            json!({ "context": model.context_limit, "output": model.output_limit }),
        );
    }
    if model.input_price.is_some() || model.output_price.is_some() {
        obj.insert(
            "cost".to_string(),
            json!({ "input": model.input_price, "output": model.output_price }),
        );
    }
    obj.insert(
        "capabilities".to_string(),
        json!({ "tools": model.supports_tools }),
    );
    obj.insert(
        "modalities".to_string(),
        json!({ "input": model.modalities_in, "output": ["text"] }),
    );
    Value::Object(obj)
}

/// The `providers.ocrouter` block to paste into (or merge into) `opencode.json`
/// in the OpenCode v2 shape.
pub fn generate_snippet(models: &[ModelInfo], proxy_base_url: &str) -> Value {
    let mut model_map = Map::new();
    for model in models {
        model_map.insert(model.id.clone(), model_entry(model));
    }
    json!({
        "providers": {
            PROVIDER_ID: {
                "name": "ocrouter (Experiential then Go)",
                "package": PACKAGE,
                "settings": {
                    "baseURL": proxy_base_url,
                    "apiKey": "ocrouter-local"
                },
                "models": Value::Object(model_map)
            }
        }
    })
}

fn backup_path(config_path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = config_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "opencode.json".to_string());
    config_path.with_file_name(format!("{name}.bak-{stamp}"))
}

/// Merge only `providers.ocrouter` into the existing config, preserving every
/// other key. Returns the path of the timestamped backup.
pub fn write_opencode_config(config_path: &Path, snippet: &Value) -> anyhow::Result<PathBuf> {
    let mut root: Value = if config_path.exists() {
        let raw = std::fs::read_to_string(config_path)?;
        // A file we cannot parse (JSONC comments, trailing commas) is left
        // alone rather than replaced by a config holding only our block.
        serde_json::from_str(&raw).map_err(|e| {
            anyhow::anyhow!(
                "{} is not plain JSON ({e}); add the snippet by hand",
                config_path.display()
            )
        })?
    } else {
        json!({})
    };
    if !root.is_object() {
        anyhow::bail!("{} is not a JSON object", config_path.display());
    }

    let block = snippet
        .get("providers")
        .and_then(|p| p.get(PROVIDER_ID))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("snippet is missing providers.{PROVIDER_ID}"))?;

    let root_obj = root.as_object_mut().expect("checked above");
    let providers = root_obj
        .entry("providers".to_string())
        .or_insert_with(|| json!({}));
    let providers_obj = providers
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("existing `providers` is not an object"))?;
    providers_obj.insert(PROVIDER_ID.to_string(), block);

    let backup = backup_path(config_path);
    if config_path.exists() {
        std::fs::copy(config_path, &backup)?;
    }
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(config_path, serde_json::to_string_pretty(&root)?)?;
    Ok(backup)
}

/// `OPENCODE_CONFIG`, then `XDG_CONFIG_HOME`, then `~/.config/opencode/opencode.json`.
pub fn find_config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("OPENCODE_CONFIG") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("opencode").join("opencode.json"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("opencode")
            .join("opencode.json"),
    )
}
