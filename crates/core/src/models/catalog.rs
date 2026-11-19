use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{Provider, Upstream};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Experiential,
    Go,
    Console,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub lane: Lane,
    pub provider: Provider,
    pub context_limit: Option<u64>,
    pub output_limit: Option<u64>,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    pub supports_tools: bool,
    pub modalities_in: Vec<String>,
}

pub fn lane_for(provider: Provider) -> Lane {
    match provider {
        Provider::OpenCodeGo => Lane::Go,
        Provider::OpenCodeConsole => Lane::Console,
        Provider::Experiential => Lane::Experiential,
    }
}

fn is_chat(item: &Value) -> bool {
    match item.get("type").and_then(Value::as_str) {
        None | Some("chat") => true,
        _ => false,
    }
}

fn str_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// OpenAI list shape: `{"object":"list","data":[{...}]}`. Used by
/// `/inference/v1/models` on Go, Console and Experiential.
pub fn parse_model_list(raw: &str, provider: Provider, lane: Lane) -> Option<Vec<ModelInfo>> {
    let json: Value = serde_json::from_str(raw).ok()?;
    let data = json.get("data")?.as_array()?;
    let models = data
        .iter()
        .filter(|item| is_chat(item))
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            let limit = item.get("limit");
            let text = item.get("cost").and_then(|c| c.get("text"));
            Some(ModelInfo {
                name,
                context_limit: limit.and_then(|l| l.get("context")).and_then(Value::as_u64),
                output_limit: limit.and_then(|l| l.get("output")).and_then(Value::as_u64),
                input_price: text
                    .and_then(|t| t.get("input"))
                    .and_then(|i| i.get("price"))
                    .and_then(Value::as_f64),
                output_price: text
                    .and_then(|t| t.get("output"))
                    .and_then(|o| o.get("price"))
                    .and_then(Value::as_f64),
                supports_tools: item
                    .get("capabilities")
                    .and_then(|c| c.get("tools"))
                    .and_then(|t| t.get("supported"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                modalities_in: {
                    let m = str_list(item.get("modalities").and_then(|m| m.get("input")));
                    if m.is_empty() {
                        vec!["text".to_string()]
                    } else {
                        m
                    }
                },
                id,
                lane,
                provider,
            })
        })
        .collect();
    Some(models)
}

/// Experiential console catalog shape: `{"models":[{"model":{...}}]}` from
/// the keyless `GET /api/models`. Entries without a `slug` are dropped.
pub fn parse_el_catalog(raw: &str) -> Option<Vec<ModelInfo>> {
    let json: Value = serde_json::from_str(raw).ok()?;
    let entries = json.get("models")?.as_array()?;
    let models = entries
        .iter()
        .filter_map(|entry| entry.get("model"))
        .filter_map(|model| {
            let id = model.get("slug").and_then(Value::as_str)?.to_string();
            let name = model
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            Some(ModelInfo {
                id,
                name,
                lane: Lane::Experiential,
                provider: Provider::Experiential,
                context_limit: model.get("context_window").and_then(Value::as_u64),
                output_limit: model.get("max_output_tokens").and_then(Value::as_u64),
                input_price: None,
                output_price: None,
                supports_tools: true,
                modalities_in: vec!["text".to_string()],
            })
        })
        .collect();
    Some(models)
}

/// The model-list URL. The EL base already ends in `/v1`, so plain
/// concatenation produced `/v1/v1/models` (404) and an empty EL catalog.
pub fn models_url(upstream: &Upstream) -> String {
    crate::proxy::routing::build_upstream_url(&upstream.base_url, upstream.model_list_path)
}

pub async fn fetch_models(
    client: &reqwest::Client,
    upstream: &Upstream,
    secret: Option<&str>,
) -> anyhow::Result<Vec<ModelInfo>> {
    let url = models_url(upstream);
    let mut request = client.get(&url);
    if let Some(secret) = secret {
        request = request.bearer_auth(secret);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("model list {url} returned HTTP {}", response.status());
    }
    let body = response.text().await?;
    let lane = lane_for(upstream.provider);
    parse_model_list(&body, upstream.provider, lane)
        .or_else(|| parse_el_catalog(&body))
        .ok_or_else(|| anyhow::anyhow!("unrecognised model list shape from {url}"))
}
