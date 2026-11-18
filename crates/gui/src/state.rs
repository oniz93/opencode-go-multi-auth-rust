use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyRow {
    pub label: String,
    pub provider: String,
    pub preview: String,
    pub status: String,
    pub cooldown_until_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AllowanceRow {
    pub org: String,
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub connected: bool,
    pub strategy: String,
    pub keys: Vec<KeyRow>,
    pub allowance: Vec<AllowanceRow>,
    pub events: Vec<String>,
    pub error: Option<String>,
}

fn as_u64(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn as_str(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Build the view state from `GET /api/status` and `GET /api/keys`.
/// The key list only ever contains the masked `preview`, never a secret.
pub fn parse_status(status: &str, keys: &str) -> UiState {
    let mut state = UiState {
        connected: true,
        ..UiState::default()
    };

    if let Ok(json) = serde_json::from_str::<Value>(status) {
        state.strategy = as_str(json.get("strategy"));
        if let Some(rows) = json.get("allowance").and_then(Value::as_array) {
            for row in rows {
                let row = row.as_array().cloned().unwrap_or_default();
                state.allowance.push(AllowanceRow {
                    org: row.first().and_then(Value::as_str).unwrap_or("unknown").to_string(),
                    used: as_u64(row.get(1)),
                    limit: as_u64(row.get(2)),
                });
            }
        }
        if let Some(events) = json.get("events").and_then(Value::as_array) {
            for event in events.iter().rev().take(40) {
                let level = as_str(event.get("level"));
                let message = as_str(event.get("message"));
                state.events.push(format!("[{level}] {message}"));
            }
        }
    }

    if let Ok(json) = serde_json::from_str::<Value>(keys) {
        if let Some(rows) = json.as_array() {
            for row in rows {
                state.keys.push(KeyRow {
                    label: as_str(row.get("label")),
                    provider: as_str(row.get("provider")),
                    preview: as_str(row.get("preview")),
                    status: as_str(row.get("status")),
                    cooldown_until_ms: row.get("cooldownUntilMs").and_then(Value::as_u64),
                });
            }
        }
    }

    state
}

/// `~/.config/ocrouter/control.token`, trimmed. Never rendered.
pub fn control_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home)
        .join(".config")
        .join("ocrouter")
        .join("control.token");
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_keys_without_any_secret() {
        let status = r#"{
            "keys": 2, "activeKeys": 2, "strategy": "priority_failover",
            "allowance": [["org-1", 120000, 950000]],
            "events": [{"at_ms":1,"level":"info","message":"POST /v1/chat/completions -> 200"}]
        }"#;
        let keys = r#"[
            {"id":"opencode-go:go-1","provider":"opencode-go","label":"go-1",
             "preview":"oc_x…yz","enabled":true,"status":"active","consecutiveErrors":0,
             "cooldownUntilMs":null}
        ]"#;

        let state = parse_status(status, keys);
        assert!(state.connected);
        assert_eq!(state.strategy, "priority_failover");
        assert_eq!(state.keys.len(), 1);
        assert_eq!(state.keys[0].label, "go-1");
        assert_eq!(state.keys[0].preview, "oc_x…yz");
        assert_eq!(state.allowance[0].used, 120_000);
        assert_eq!(state.allowance[0].limit, 950_000);
        assert_eq!(state.events.len(), 1);
    }

    #[test]
    fn malformed_payloads_do_not_panic() {
        let state = parse_status("not json", "{");
        assert!(state.connected);
        assert!(state.keys.is_empty());
    }
}
