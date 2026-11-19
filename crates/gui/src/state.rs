use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyRow {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub preview: String,
    pub status: String,
    pub enabled: bool,
    pub consecutive_errors: u64,
    pub cooldown_until_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AllowanceRow {
    pub org: String,
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventRow {
    pub at_ms: u64,
    pub level: String,
    pub message: String,
}

/// One hop of a route: key id (`provider:label`) and the model id sent upstream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub key: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteRow {
    pub model: String,
    pub steps: Vec<Step>,
}

/// Mirror of the daemon's `[routing]` config (`GET/PUT /api/routing`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Routing {
    pub expose: Vec<String>,
    pub routes: Vec<RouteRow>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelRow {
    pub id: String,
    pub lane: String,
}

#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub connected: bool,
    pub strategy: String,
    pub keys: Vec<KeyRow>,
    pub allowance: Vec<AllowanceRow>,
    pub events: Vec<EventRow>,
    pub routing: Routing,
    pub models: Vec<ModelRow>,
    pub error: Option<String>,
}

fn as_u64(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn as_str(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
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
                    org: row
                        .first()
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    used: as_u64(row.get(1)),
                    limit: as_u64(row.get(2)),
                });
            }
        }
        if let Some(events) = json.get("events").and_then(Value::as_array) {
            for event in events.iter().rev().take(40) {
                state.events.push(EventRow {
                    at_ms: as_u64(event.get("at_ms")),
                    level: as_str(event.get("level")),
                    message: as_str(event.get("message")),
                });
            }
        }
    }

    if let Ok(json) = serde_json::from_str::<Value>(keys) {
        if let Some(rows) = json.as_array() {
            for row in rows {
                state.keys.push(KeyRow {
                    id: as_str(row.get("id")),
                    label: as_str(row.get("label")),
                    provider: as_str(row.get("provider")),
                    preview: as_str(row.get("preview")),
                    status: as_str(row.get("status")),
                    enabled: row.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    consecutive_errors: as_u64(row.get("consecutiveErrors")),
                    cooldown_until_ms: row.get("cooldownUntilMs").and_then(Value::as_u64),
                });
            }
        }
    }

    state
}

pub fn parse_routing(raw: &str) -> Routing {
    let Ok(json) = serde_json::from_str::<Value>(raw) else {
        return Routing::default();
    };
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let routes = json
        .get("route")
        .and_then(Value::as_array)
        .map(|routes| {
            routes
                .iter()
                .map(|r| RouteRow {
                    model: as_str(r.get("model")),
                    steps: r
                        .get("steps")
                        .and_then(Value::as_array)
                        .map(|steps| {
                            steps
                                .iter()
                                .map(|s| Step {
                                    key: as_str(s.get("key")),
                                    model: as_str(s.get("model")),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    Routing {
        expose: strings(json.get("expose")),
        routes,
    }
}

pub fn parse_models(raw: &str) -> Vec<ModelRow> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|json| json.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|m| ModelRow {
            id: as_str(m.get("id")),
            lane: as_str(m.get("lane")),
        })
        .collect()
}

impl Routing {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "expose": self.expose,
            "route": self.routes.iter().map(|r| serde_json::json!({
                "model": r.model,
                "steps": r.steps.iter().map(|s| serde_json::json!({
                    "key": s.key, "model": s.model
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
        .to_string()
    }

    /// Every id the proxy can list: catalog ids, then route aliases.
    pub fn listable(&self, models: &[ModelRow]) -> Vec<String> {
        let mut ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        for route in &self.routes {
            if !ids.contains(&route.model) {
                ids.push(route.model.clone());
            }
        }
        ids
    }

    /// An empty `expose` means "all".
    pub fn is_exposed(&self, id: &str) -> bool {
        self.expose.is_empty() || self.expose.iter().any(|e| e == id)
    }

    pub fn toggle_expose(&mut self, id: &str, all: &[String]) {
        if self.expose.is_empty() {
            self.expose = all.to_vec();
        }
        if let Some(i) = self.expose.iter().position(|e| e == id) {
            self.expose.remove(i);
        } else {
            self.expose.push(id.to_string());
        }
    }
}

/// `120000` -> `120k`, `1_250_000` -> `1.3M`.
pub fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_499 => format!("{:.0}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// UTC wall-clock `HH:MM:SS` for an epoch-ms timestamp.
pub fn clock(at_ms: u64) -> String {
    let s = at_ms / 1000 % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Time left until `until_ms`, e.g. `4m 12s`; `None` once it has passed.
pub fn remaining(until_ms: u64, now_ms: u64) -> Option<String> {
    let s = until_ms.checked_sub(now_ms)?.div_ceil(1000);
    Some(match s {
        0 => return None,
        1..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s / 60 % 60),
    })
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
        assert_eq!(state.events[0].level, "info");
        assert!(state.keys[0].enabled);
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(120_000), "120k");
        assert_eq!(compact(1_260_000), "1.3M");
        assert_eq!(clock(3_723_000), "01:02:03");
        assert_eq!(remaining(10_000, 10_000), None);
        assert_eq!(remaining(5_000, 10_000), None);
        assert_eq!(remaining(10_500, 10_000).as_deref(), Some("1s"));
        assert_eq!(remaining(262_000, 10_000).as_deref(), Some("4m 12s"));
        assert_eq!(remaining(7_210_000, 10_000).as_deref(), Some("2h 0m"));
    }

    #[test]
    fn routing_round_trips_and_toggles_exposure() {
        let raw = r#"{"expose":[],"route":[{"model":"ds","steps":[
            {"key":"opencode-go:go-1","model":"deepseek-v4.1-flash"}]}]}"#;
        let mut routing = parse_routing(raw);
        assert_eq!(routing.routes[0].steps[0].key, "opencode-go:go-1");
        assert_eq!(parse_routing(&routing.to_json()), routing);

        let models = parse_models(r#"[{"id":"glm","lane":"go"}]"#);
        let all = routing.listable(&models);
        assert_eq!(all, vec!["glm".to_string(), "ds".to_string()]);
        assert!(routing.is_exposed("glm"));
        routing.toggle_expose("glm", &all);
        assert_eq!(routing.expose, vec!["ds".to_string()]);
        assert!(!routing.is_exposed("glm"));
    }

    #[test]
    fn malformed_payloads_do_not_panic() {
        let state = parse_status("not json", "{");
        assert!(state.connected);
        assert!(state.keys.is_empty());
    }
}
