use super::usage::TokenUsage;

/// Per-1M-token prices. Display-only: never used for routing or quota
/// prediction. Prices are the published Console/Go list prices
/// (https://opencode.ai/docs/console/models#pricing); absent models are
/// reported as "unknown" rather than guessed.
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

fn rates(model: &str) -> Option<Rates> {
    let r = |input, output, cache_read, cache_write| Rates {
        input,
        output,
        cache_read,
        cache_write,
    };
    Some(match model {
        "deepseek-v4.1-flash" => r(0.30, 1.20, 0.006, 0.0),
        "deepseek-v4-pro" => r(1.74, 3.48, 0.145, 0.0),
        "glm-5.2" | "glm-5.3" => r(1.40, 4.40, 0.26, 0.0),
        "glm-5.3-flash" => r(0.15, 0.50, 0.03, 0.0),
        "grok-4.7" => r(2.00, 6.00, 0.50, 0.0),
        "gpt-6-luna" => r(0.10, 0.50, 0.01, 0.125),
        "gpt-5.6-luna" => r(0.20, 1.20, 0.02, 0.25),
        "claude-haiku-5-5" => r(0.10, 0.50, 0.01, 0.125),
        "kimi-k3" => r(3.00, 15.00, 0.30, 3.00),
        "kimi-k2.7-code" => r(0.95, 4.00, 0.19, 0.0),
        "kimi-k2.6" => r(0.95, 4.00, 0.16, 0.0),
        "minimax-m3" => r(0.30, 1.20, 0.06, 0.0),
        "qwen3.8-max" => r(2.00, 6.00, 0.25, 2.50),
        "qwen3.8-flash" => r(0.15, 0.47, 0.016, 0.20),
        _ => return None,
    })
}

pub fn estimate_cost(model: &str, usage: &TokenUsage) -> Option<f64> {
    let r = rates(model)?;
    let per_m = |tokens: u64, price: f64| (tokens as f64 / 1_000_000.0) * price;
    Some(
        per_m(usage.input, r.input)
            + per_m(usage.output, r.output)
            + per_m(usage.cache_read, r.cache_read)
            + per_m(usage.cache_write, r.cache_write),
    )
}
