use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiShape {
    Chat,
    Messages,
    Responses,
}

fn num(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn details(usage: &Value, key: &str, field: &str) -> u64 {
    num(usage.get(key).and_then(|d| d.get(field)))
}

fn extract(json: &Value, shape: ApiShape) -> Option<TokenUsage> {
    let usage = json.get("usage")?;
    if !usage.is_object() {
        return None;
    }
    let out = match shape {
        ApiShape::Chat => TokenUsage {
            input: num(usage.get("prompt_tokens")),
            output: num(usage.get("completion_tokens")),
            cache_read: details(usage, "prompt_tokens_details", "cached_tokens"),
            cache_write: 0,
        },
        ApiShape::Messages => TokenUsage {
            input: num(usage.get("input_tokens")),
            output: num(usage.get("output_tokens")),
            cache_read: num(usage.get("cache_read_input_tokens")),
            cache_write: num(usage.get("cache_creation_input_tokens")),
        },
        ApiShape::Responses => TokenUsage {
            input: num(usage.get("input_tokens")),
            output: num(usage.get("output_tokens")),
            cache_read: details(usage, "input_tokens_details", "cached_tokens"),
            cache_write: 0,
        },
    };
    Some(out)
}

pub fn parse_usage(body: &str, shape: ApiShape) -> Option<TokenUsage> {
    let json: Value = serde_json::from_str(body).ok()?;
    extract(&json, shape)
}

/// Usage carried on the final frame of an OpenAI-style SSE stream.
pub fn parse_usage_from_sse(chunk: &str) -> Option<TokenUsage> {
    for line in chunk.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(json) = serde_json::from_str::<Value>(data.trim()) {
            if let Some(usage) = extract(&json, ApiShape::Chat) {
                return Some(usage);
            }
        }
    }
    None
}
