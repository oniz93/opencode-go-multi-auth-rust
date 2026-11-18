# ocrouter Daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single Rust binary that pools OpenCode Go keys and routes to Experiential Labs first, rewriting the TypeScript `opencode-go-multi-auth` proxy as `ocrouter`.

**Architecture:** A Cargo workspace with `ocrouter-core` (pure logic — config, keys, router, quota, model catalog; no HTTP, no UI) and `ocrouter-cli` (a binary producing `ocrouter`, which serves the reverse proxy on 18905 and a token-gated control API on 18904). The proxy is a byte-for-byte streaming passthrough; lane choice (Experiential → Go) happens per model, and every routing decision reacts to upstream signals rather than predicting quota.

**Tech Stack:** Rust 1.98 (edition 2021), `tokio`, `hyper` + `hyper-util`, `axum` (control API), `reqwest` (upstream calls with streaming), `serde`/`toml`, `tracing`, `clap` (CLI).

**Spec:** `docs/superpowers/specs/2026-11-18-ocrouter-rust-design.md` — read it; this plan argues from it.

## Global Constraints

- **Native target pinned.** `.cargo/config.toml` sets `[build] target = "aarch64-apple-darwin"`. The `cargo` on PATH is Homebrew **x86_64**; without the pin you ship a Rosetta binary.
- Ports: proxy **18905**, control API **18904**. Host: macOS first.
- **Never** add a GUI dependency to `ocrouter-cli` or `ocrouter-core`. GPUI lives in a separate crate (plan 2).
- The proxy is **byte-for-byte** on request and response bodies. The one allowed body mutation is `stream_options.include_usage = true` on streamed OpenAI chat-completions requests. Model IDs are **never** rewritten.
- Route on **path and the `model` field only**. Never read `tools`, `messages`, or `tool_use` to make a routing decision. The proxy speaks exactly three upstream routes: OpenAI Chat Completions `POST /v1/chat/completions`, Anthropic `POST /v1/messages`, and the model list `GET /v1/models`.
- Upstream base URLs (overridable in config): Go `https://opencode.ai/inference/go`, Console `https://opencode.ai/inference`, EL `https://api.experientiallabs.ai/v1`, EL management `https://api.experientiallabs.ai/api`.
- The Go lane **always** sends `x-opencode-session` (synthesise when absent). Both `Authorization: Bearer` and `x-api-key` are sent upstream.
- No hard-coded upstream wall-clock timeout; the only timeout is env-gated and defaults off.
- Experiential free core = plain model ids `gpt-6-luna`, `deepseek-v4.1-flash`, `clef`; **1,000,000** input+output tokens/day shared per **org**, reset 00:00 UTC; send the plain slug, never `:free`, never rewrite the model id.
- Keys live in a plain file `~/.config/ocrouter/keys.toml`, mode **0600**. Secrets never enter logs, the control-API responses, or git.
- Verification per task = `cargo test -p <crate>` and `cargo build` clean. No new formatter/linter config beyond `rustfmt` defaults.

## Review Focus

The spec implies these but no single task's happy-path test covers them; each is pinned to a task below.

1. **A same-body double 5xx must not open every breaker** (a bad request must not evict healthy keys) → Task 6, Task 18.
2. **A client that omits `x-opencode-session` must not get a 400 from Go** → Task 17.
3. **Two EL keys from one org must not each claim 1,000,000 tokens** → Task 9.
4. **A model served by both lanes must still reach Go after EL's allowance is spent** → Task 14, Task 18.
5. **A non-streaming error body must not be consumed before it is sized/inspected** (buffering vs streaming) → Task 16.

---

### Task 1: Workspace, native target, and the first test

**Files:**
- Create: `Cargo.toml` (workspace), `.cargo/config.toml`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`

**Interfaces:**
- Produces: crate names `ocrouter-core` (lib `ocrouter_core`) and `ocrouter-cli` (bin `ocrouter`).

- [ ] **Step 1: Write the failing test**

In `crates/core/src/lib.rs`, add `pub fn version() -> &'static str { env!("CARGO_PKG_VERSION") }` and a test:

```rust
#[test]
fn version_is_non_empty() {
    assert!(!crate::version().is_empty());
}
```

- [ ] **Step 2: Run it to verify the workspace is wired**

Run: `cargo test -p ocrouter-core`
Expected: PASS once the file exists (proves the workspace resolves).

- [ ] **Step 3: Create the workspace**

`Cargo.toml` declares `[workspace] members = ["crates/core", "crates/cli"]`, `resolver = "2"`, and shared dependency versions under `[workspace.dependencies]` (`tokio` `features=["full"]`, `hyper`, `hyper-util`, `axum`, `reqwest` `default-features=false` `features=["rustls-tls","stream"]`, `serde`, `serde_json`, `toml`, `tracing`, `tracing-subscriber`, `clap` `features=["derive"]`, `thiserror`, `anyhow`, `chrono`, `rand`, `sha2`, `chacha20poly1305`). `.cargo/config.toml` sets `[build] target = "aarch64-apple-darwin"`. `crates/cli/src/main.rs` is a stub `fn main() {}`.

- [ ] **Step 4: Verify build and test**

Run: `cargo build` then `cargo test -p ocrouter-core`
Expected: both succeed; `cargo build` output does **not** mention `x86_64`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml .cargo crates
git commit -m "chore: scaffold ocrouter cargo workspace"
```

---

### Task 2: Config model and TOML load/save

**Files:**
- Create: `crates/core/src/config.rs`, `crates/core/tests/config.rs`
- Modify: `crates/core/src/lib.rs:1` (add `pub mod config;`)

**Interfaces:**
- Produces: `pub struct Config { pub proxy_port: u16, pub control_port: u16, pub strategy: RoutingStrategy, pub go_base_url: String, pub oc_base_url: String, pub el_base_url: String, pub el_management_url: String, pub el_daily_token_cap: u64, pub el_safety_margin: f64, pub el_allow_paid: bool, pub cooldown_ms: u64, pub circuit_breaker_threshold: u32, pub log_level: String, pub keys_path: Option<PathBuf>, pub state_path: Option<PathBuf> }`; `Config::default()`, `Config::load(path: &Path) -> anyhow::Result<Config>` (missing file → default), `Config::save(&self, path: &Path) -> anyhow::Result<()>`, `Config::apply_env(&mut self)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn defaults_match_spec() {
    let c = Config::default();
    assert_eq!(c.proxy_port, 18905);
    assert_eq!(c.control_port, 18904);
    assert_eq!(c.el_daily_token_cap, 1_000_000);
    assert_eq!(c.el_allow_paid, false);
    assert_eq!(c.go_base_url, "https://opencode.ai/inference/go");
    assert_eq!(c.el_base_url, "https://api.experientiallabs.ai/v1");
}

#[test]
fn missing_file_yields_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let c = Config::load(&dir.path().join("nope.toml")).unwrap();
    assert_eq!(c.proxy_port, 18905);
}
```

(Add `tempfile` to `[dev-dependencies]`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ocrouter-core config::`
Expected: FAIL — `Config` not defined.

- [ ] **Step 3: Implement `Config`**

Derive `Serialize, Deserialize, Clone, Debug`; `#[serde(default)]` on the struct so a partial file fills defaults. `apply_env` reads `OCROUTER_PROXY_PORT`, `OCROUTER_CONTROL_PORT`, `OCROUTER_GO_BASE_URL`, `OCROUTER_EL_BASE_URL`, `OCROUTER_KEYS`, `OCROUTER_STATE`, `OCROUTER_LOG_LEVEL` and overrides. `RoutingStrategy` comes from Task 5 — for now add a placeholder enum in `config.rs` and move it to `router/types.rs` in Task 5, re-exporting it.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p ocrouter-core config::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): config model with TOML load/save and env overrides"
```

---

### Task 3: Key file (plaintext, 0600, portable)

**Files:**
- Create: `crates/core/src/keys.rs`, `crates/core/tests/keys.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Consumes: `Provider` (Task 4).
- Produces: `pub struct KeyEntry { pub provider: Provider, pub label: String, pub secret: String, pub priority: u32, pub weight: u32, pub enabled: bool }`; `pub struct KeyFile { pub key: Vec<KeyEntry> }`; `KeyFile::load(&Path)`, `KeyFile::save(&self, &Path)` (writes with mode `0o600`), `KeyFile::add(KeyEntry)`, `KeyFile::remove(&str label)`, `KeyFile::masked(&self) -> Vec<MaskedKey>` where `MaskedKey { label, provider, preview }` and `preview` shows only the first 4 and last 2 chars.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn saved_file_is_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("keys.toml");
    let mut kf = KeyFile::default();
    kf.add(KeyEntry { provider: Provider::OpenCodeGo, label: "go-1".into(),
        secret: "oc_secretvalue".into(), priority: 1, weight: 1, enabled: true });
    kf.save(&p).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let raw = std::fs::read_to_string(&p).unwrap();
    assert!(raw.contains("oc_secretvalue"));
}

#[test]
fn masked_never_reveals_the_middle() {
    let kf = KeyFile { key: vec![KeyEntry { provider: Provider::Experiential,
        label: "el".into(), secret: "xpl_abcdef123456".into(),
        priority: 1, weight: 1, enabled: true }] };
    let m = &kf.masked()[0];
    // preview = first 4 chars, ellipsis, last 2 chars
    assert_eq!(m.preview, "xpl_…56");
    assert!(!m.preview.contains("cdef"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ocrouter-core keys::`
Expected: FAIL.

- [ ] **Step 3: Implement, enforcing mode on every write**

`save` uses `OpenOptions::new().create(true).write(true).truncate(true).mode(0o600)`. `save` must not log the contents.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p ocrouter-core keys::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): portable 0600 key file with masked views"
```

---

### Task 4: Provider and upstream registry

**Files:**
- Create: `crates/core/src/provider.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Produces: `pub enum Provider { OpenCodeGo, OpenCodeConsole, Experiential }` (serde `rename_all="kebab-case"`, serialises as `opencode-go`, `opencode-console`, `experiential`); `pub enum AuthStyle { BearerAndApiKey, BearerOnly }`; `pub struct Upstream { pub provider: Provider, pub base_url: String, pub model_list_path: &'static str, pub auth: AuthStyle }`; `pub fn upstream_for(provider: Provider, cfg: &Config) -> Upstream`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn go_and_el_auth_styles_differ() {
    let cfg = Config::default();
    assert!(matches!(upstream_for(Provider::OpenCodeGo, &cfg).auth, AuthStyle::BearerAndApiKey));
    assert!(matches!(upstream_for(Provider::Experiential, &cfg).auth, AuthStyle::BearerOnly));
}

#[test]
fn providers_serialise_kebab_case() {
    assert_eq!(serde_json::to_string(&Provider::OpenCodeGo).unwrap(), "\"opencode-go\"");
}
```

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core provider::` — Expected: FAIL.

- [ ] **Step 3: Implement the registry**, reading base URLs from `Config`.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core provider::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): provider and upstream registry"
```

---

### Task 5: Router types and key manager

**Files:**
- Create: `crates/core/src/router/types.rs`, `crates/core/src/router/key_manager.rs`, `crates/core/src/router/mod.rs`, `crates/core/tests/router.rs`
- Modify: `crates/core/src/lib.rs:1`, `crates/core/src/config.rs:1` (import `RoutingStrategy` from here)

**Interfaces:**
- Consumes: `KeyEntry`, `Provider`.
- Produces: `pub enum RoutingStrategy { PriorityFailover, RoundRobin, SessionAffinity }` (default `PriorityFailover`); `pub struct ApiKey { pub id: String, pub provider: Provider, pub label: String, pub secret: String, pub priority: u32, pub weight: u32, pub enabled: bool, pub status: KeyStatus, pub consecutive_errors: u32, pub cooldown_until_ms: Option<u64> }`; `pub enum KeyStatus { Active, Exhausted, Disabled }`; `pub struct KeySelection { pub key: ApiKey, pub reason: String }`; `pub struct KeyManager` with `new(Vec<ApiKey>)`, `from_keyfile(&KeyFile)`, `all(&self)`, `enabled(&self)`, `eligible(&self, exclude: &HashSet<String>, now_ms: u64) -> Vec<&ApiKey>`, `select(&mut self, strategy, exclude, now_ms) -> Option<KeySelection>`, `mark_exhausted(&mut self, id, until_ms)`, `record_error(&mut self, id)`, `iter_mut()`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn priority_failover_prefers_lowest_priority_number() {
    let mut km = KeyManager::new(vec![key("a", 2), key("b", 1)]);
    let s = km.select(RoutingStrategy::PriorityFailover, &HashSet::new(), 0).unwrap();
    assert_eq!(s.key.id, "b");
}

#[test]
fn excluded_and_cooling_keys_are_skipped() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 2)]);
    km.mark_exhausted("a", 10_000);
    let excluded = HashSet::new();
    assert_eq!(km.select(RoutingStrategy::PriorityFailover, &excluded, 5_000).unwrap().key.id, "b");
    let mut ex = HashSet::new(); ex.insert("b".to_string());
    assert!(km.select(RoutingStrategy::PriorityFailover, &ex, 5_000).is_none());
}

#[test]
fn round_robin_advances_then_wraps() {
    let mut km = KeyManager::new(vec![key("a", 1), key("b", 1)]);
    let mut seen = vec![];
    for _ in 0..4 { seen.push(km.select(RoutingStrategy::RoundRobin, &HashSet::new(), 0).unwrap().key.id); }
    assert_eq!(seen, vec!["a", "b", "a", "b"]);
}
```

Helper `fn key(id: &str, priority: u32) -> ApiKey` builds an enabled `OpenCodeGo` key.

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core router::` — Expected: FAIL.

- [ ] **Step 3: Implement `KeyManager`.** `eligible` = enabled, `status == Active`, and `cooldown_until_ms.map_or(true, |t| t <= now_ms)`, minus `exclude`. `RoundRobin` keeps a cursor over the eligible list and advances on each `select`. `PriorityFailover` picks the lowest `priority` number, tie-broken by `weight` descending then `id`. `SessionAffinity` behaves like `PriorityFailover` here; affinity is layered on in Task 8.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core router::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): router types and key manager with strategies"
```

---

### Task 6: Circuit breaker

**Files:**
- Create: `crates/core/src/router/circuit_breaker.rs`
- Modify: `crates/core/src/router/mod.rs:1`

**Interfaces:**
- Produces: `pub enum CircuitState { Closed, Open, HalfOpen }`; `pub struct CircuitBreaker` with `new(threshold: u32, open_ms: u64)`, `is_available(&self, id: &str, now_ms: u64) -> bool`, `record_failure(&mut self, id: &str, now_ms: u64) -> CircuitState`, `record_success(&mut self, id: &str)`, `state_of(&self, id: &str) -> CircuitState`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn opens_only_after_threshold_consecutive_failures() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Open);
    assert!(!cb.is_available("k", 0));
}

#[test]
fn success_resets_the_counter() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    cb.record_failure("k", 0); cb.record_failure("k", 0);
    cb.record_success("k");
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
}

#[test]
fn half_opens_after_open_ms() {
    let mut cb = CircuitBreaker::new(1, 10_000);
    cb.record_failure("k", 0);
    assert!(!cb.is_available("k", 5_000));
    assert!(cb.is_available("k", 10_000));
    assert_eq!(cb.state_of("k"), CircuitState::HalfOpen);
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with a `HashMap<String, Entry>`; entry holds `consecutive_failures`, `opened_at_ms: Option<u64>`.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): per-key circuit breaker"
```

---

### Task 7: Quota detection and cooldown resolution

**Files:**
- Create: `crates/core/src/proxy/quota.rs`, `crates/core/src/proxy/mod.rs`, `crates/core/tests/quota.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Produces: `pub fn is_quota_error(status: u16, headers: &HeaderMap, body: &str) -> bool`; `pub fn resolve_cooldown_ms(headers: &HeaderMap, body: &str, now_ms: u64, fallback_ms: u64) -> u64`; `pub fn extract_quota_message(body: &str, status: u16) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn generic_429_is_not_quota() {
    assert!(!is_quota_error(429, &HeaderMap::new(), r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#));
}

#[test]
fn free_limit_reached_is_quota() {
    assert!(is_quota_error(429, &HeaderMap::new(), r#"{"error":{"code":"free_limit_reached"}}"#));
    assert!(is_quota_error(402, &HeaderMap::new(), r#"{"error":{"message":"credit balance exhausted"}}"#));
}

#[test]
fn retry_after_header_wins() {
    let mut h = HeaderMap::new();
    h.insert("retry-after", "120".parse().unwrap());
    assert_eq!(resolve_cooldown_ms(&h, "", 0, 9_999_999), 120_000);
}

#[test]
fn body_retry_after_ms_is_used_when_header_absent() {
    assert_eq!(resolve_cooldown_ms(&HeaderMap::new(), r#"{"error":{"retry_after_ms":60000}}"#, 0, 9), 60_000);
}

#[test]
fn fallback_used_when_nothing_present() {
    assert_eq!(resolve_cooldown_ms(&HeaderMap::new(), "{}", 0, 5 * 3_600_000), 5 * 3_600_000);
}
```

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core quota::` — Expected: FAIL.

- [ ] **Step 3: Implement.** Quota markers (case-insensitive, substring): `insufficient_quota`, `usage_not_included`, `freeusagelimit`, `free_limit_reached`, `insufficient_credits`, `model_requires_payment`, `exhausted`, `credit balance`. Status 402 is always quota. Cooldown order: `retry-after` (seconds or HTTP date) → `x-codex-*-reset-at` / `x-codex-*-reset-after-seconds` → `error.retry_after_ms` → `error.retry_after` (seconds) → `error.resets_at` (RFC3339) → `retry after N (m|h|s)` text → `fallback_ms`. Clamp to ≥ 1000 ms.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core quota::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): quota detection and cooldown resolution"
```

---

### Task 8: Session affinity

**Files:**
- Create: `crates/core/src/router/session_affinity.rs`
- Modify: `crates/core/src/router/mod.rs:1`

**Interfaces:**
- Produces: `pub struct SessionAffinity` with `new()`, `extract_session_key(headers: &HeaderMap) -> Option<String>` (first of `x-opencode-session`, `x-session-id`, `prompt-cache-key`, `x-session-affinity`), `preferred(&self, session: &str) -> Option<String>`, `set(&mut self, session: &str, key_id: &str)`, `refine_cooldowns(&mut self, now_ms)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn extracts_first_present_session_header() {
    let mut h = HeaderMap::new();
    h.insert("prompt-cache-key", "abc".parse().unwrap());
    assert_eq!(SessionAffinity::extract_session_key(&h).unwrap(), "abc");
    h.insert("x-opencode-session", "ses_1".parse().unwrap());
    assert_eq!(SessionAffinity::extract_session_key(&h).unwrap(), "ses_1");
}

#[test]
fn remembers_and_returns_preferred_key() {
    let mut sa = SessionAffinity::new();
    sa.set("s1", "key-a");
    assert_eq!(sa.preferred("s1").unwrap(), "key-a");
    assert!(sa.preferred("s2").is_none());
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with an insertion-ordered `HashMap` and a hard cap (e.g. 4096 entries, evict oldest) so it cannot grow unbounded.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): session affinity store"
```

---

### Task 9: Daily allowance tracker (per org, UTC)

**Files:**
- Create: `crates/core/src/router/allowance.rs`, `crates/core/tests/allowance.rs`
- Modify: `crates/core/src/router/mod.rs:1`

**Interfaces:**
- Produces: `pub struct Allowance { pub cap: u64, pub safety_margin: f64 }` with `new(cap, safety_margin)`, `record(&mut self, org: &str, tokens: u64, now_ms: u64)`, `used(&self, org: &str) -> u64`, `budget_left(&self, org: &str) -> u64` (= `floor(cap * (1.0 - margin))` minus used, saturating), `is_exhausted(&self, org: &str) -> bool`, `reset_if_new_utc_day(&mut self, now_ms: u64)`, `snapshot(&self) -> Vec<(String, u64, u64)>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn two_keys_one_org_share_one_budget() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 400_000, 0);
    // a second key on the same org adds to the SAME bucket
    a.record("org-1", 300_000, 0);
    assert_eq!(a.used("org-1"), 700_000);
    assert_eq!(a.budget_left("org-1"), 300_000);
}

#[test]
fn safety_margin_reserves_headroom() {
    let mut a = Allowance::new(1_000_000, 0.05);
    assert_eq!(a.budget_left("org-1"), 950_000);
    a.record("org-1", 950_000, 0);
    assert!(a.is_exhausted("org-1"));
}

#[test]
fn resets_at_the_next_utc_midnight() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 999_999, 0);
    a.reset_if_new_utc_day(utc_midnight_after(0));
    assert_eq!(a.used("org-1"), 0);
}
```

`utc_midnight_after(ms)` is a helper in `time.rs` (Task 9 also creates `crates/core/src/time.rs` with `utc_day_index(ms: u64) -> u64`).

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core allowance::` — Expected: FAIL.

- [ ] **Step 3: Implement.** Store per-org `{ day_index, used }`; on `reset_if_new_utc_day`, zero any org whose `day_index` differs. `budget_left` never underflows.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core allowance::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): per-org daily allowance tracker (UTC reset)"
```

---

### Task 10: Usage parsing and rate card

**Files:**
- Create: `crates/core/src/proxy/usage.rs`, `crates/core/src/proxy/rate_card.rs`
- Modify: `crates/core/src/proxy/mod.rs:1`

**Interfaces:**
- Produces: `pub struct TokenUsage { pub input: u64, pub output: u64, pub cache_read: u64, pub cache_write: u64 }` with `pub fn total(&self) -> u64`; `pub enum ApiShape { Chat, Messages, Responses }`; `pub fn parse_usage(body: &str, shape: ApiShape) -> Option<TokenUsage>`; `pub fn parse_usage_from_sse(chunk: &str) -> Option<TokenUsage>`; `pub struct RateCard;` `pub fn estimate_cost(model: &str, usage: &TokenUsage) -> Option<f64>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn reads_openai_usage() {
    let u = parse_usage(r#"{"usage":{"prompt_tokens":100,"completion_tokens":50}}"#, ApiShape::Chat).unwrap();
    assert_eq!((u.input, u.output), (100, 50));
}

#[test]
fn reads_anthropic_usage_including_cache() {
    let u = parse_usage(r#"{"usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":900,"cache_creation_input_tokens":20}}"#, ApiShape::Messages).unwrap();
    assert_eq!((u.input, u.output, u.cache_read, u.cache_write), (10, 4, 900, 20));
    assert_eq!(u.total(), 10 + 4 + 900 + 20);
}

#[test]
fn unknown_model_has_no_estimate() {
    assert!(estimate_cost("not-a-real-model", &TokenUsage { input: 1, output: 1, cache_read: 0, cache_write: 0 }).is_none());
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement.** Port the TS `rate-card.ts` values for the Go models (input/output/cacheRead/cacheWrite per 1M tokens), keyed by model id; return `None` for unknown. Parsing must tolerate missing fields and malformed JSON (return `None`, never panic).

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): token usage parsing and display-only rate card"
```

---

### Task 11: State store

**Files:**
- Create: `crates/core/src/state.rs`, `crates/core/tests/state.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Produces: `pub struct RouterState { pub keys: Vec<KeyHealth>, pub allowance: Vec<OrgUsage>, pub recent_events: Vec<EventRecord>, pub saved_at_ms: u64 }`; `pub struct StateStore` with `new(path: PathBuf, debounce_ms: u64)`, `load() -> RouterState`, `update(&mut self, f: impl FnOnce(&mut RouterState))`, `flush(&self) -> anyhow::Result<()>`.
- Consumes: `EventRecord` (Task 12) — define a minimal `EventRecord` struct here and have Task 12 reuse it.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn write_is_atomic_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    let mut s = StateStore::new(p.clone(), 0);
    s.update(|st| st.saved_at_ms = 42);
    s.flush().unwrap();
    assert_eq!(StateStore::new(p.clone(), 0).load().saved_at_ms, 42);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty());
}

#[test]
fn corrupt_file_loads_as_empty_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("state.json");
    std::fs::write(&p, "{ not json").unwrap();
    assert_eq!(StateStore::new(p, 0).load().saved_at_ms, 0);
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement.** `flush` writes `state.json.tmp` then `rename`s over the target. A corrupt or missing file yields `RouterState::default()`.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): atomic debounced state store"
```

---

### Task 12: Event bus and ring buffer

**Files:**
- Create: `crates/core/src/events.rs`, `crates/core/tests/events.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Produces: `pub struct EventRecord { pub at_ms: u64, pub level: Level, pub message: String, pub detail: Option<serde_json::Value> }`; `pub enum Level { Info, Warn, Error }`; `pub struct EventBus` with `new(capacity: usize)`, `emit(EventRecord)`, `recent(&self, n: usize) -> Vec<EventRecord>`, `subscribe(&self) -> tokio::sync::broadcast::Receiver<EventRecord>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn ring_buffer_keeps_only_the_last_capacity() {
    let bus = EventBus::new(3);
    for i in 0..5 { bus.emit(EventRecord { at_ms: i, level: Level::Info, message: format!("e{i}"), detail: None }); }
    let recent = bus.recent(10);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "e2");
    assert_eq!(recent[2].message, "e4");
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with a `VecDeque` behind a `std::sync::Mutex` and a `tokio::sync::broadcast` sender.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): event bus with ring buffer and broadcast"
```

---

### Task 13: Model catalog fetch and normalise

**Files:**
- Create: `crates/core/src/models/catalog.rs`, `crates/core/src/models/mod.rs`, `crates/core/tests/catalog.rs`
- Modify: `crates/core/src/lib.rs:1`

**Interfaces:**
- Consumes: `Upstream`, `Provider`.
- Produces: `pub struct ModelInfo { pub id: String, pub name: String, pub lane: Lane, pub provider: Provider, pub context_limit: Option<u64>, pub output_limit: Option<u64>, pub input_price: Option<f64>, pub output_price: Option<f64>, pub supports_tools: bool, pub modalities_in: Vec<String> }`; `pub enum Lane { Experiential, Go, Console }`; `pub async fn fetch_models(client: &reqwest::Client, up: &Upstream, secret: Option<&str>) -> anyhow::Result<Vec<ModelInfo>>`; `pub async fn fetch_opencode_snippet_models(client, base_url, secret) -> anyhow::Result<Vec<ModelInfo>>`; `pub fn normalise(id, name, ...) -> ModelInfo` (pure, unit-testable).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn normalises_openai_list_shape_and_skips_non_chat() {
    let raw = r#"{"object":"list","data":[
      {"id":"kimi-k3","object":"model","name":"Kimi K3","type":"chat","limit":{"context":256000,"output":32000}},
      {"id":"some-image","object":"model","type":"image"}
    ]}"#;
    let models = parse_model_list(raw, Provider::OpenCodeGo, Lane::Go).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "kimi-k3");
    assert_eq!(models[0].context_limit, Some(256000));
}

#[test]
fn el_catalog_shape_is_parsed() {
    // GET /api/models -> {"models":[{"model":{"slug":"claude-opus-5.5","context_window":1000000,...}}]}
    let raw = r#"{"models":[{"model":{"slug":"claude-opus-5.5","display_name":"Claude Opus 5.5","context_window":1000000,"max_output_tokens":128000}}]}"#;
    let models = parse_el_catalog(raw).unwrap();
    assert_eq!(models[0].id, "claude-opus-5.5");
    assert_eq!(models[0].context_limit, Some(1_000_000));
}
```

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core catalog::` — Expected: FAIL.

- [ ] **Step 3: Implement** `parse_model_list` (OpenAI shape) and `parse_el_catalog` (EL shape) as pure functions with the parsing tests above; `fetch_models` calls the right one per `Upstream`. Fetching is covered by Task 21's live check — do not write a network test here.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core catalog::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): model catalog fetch and normalisation"
```

---

### Task 14: Merged catalog and lane resolution

**Files:**
- Create: `crates/core/src/models/merge.rs`
- Modify: `crates/core/src/models/mod.rs:1`

**Interfaces:**
- Produces: `pub struct Catalog { pub models: Vec<ModelInfo> }`; `Catalog::merged(go: Vec<ModelInfo>, el: Vec<ModelInfo>) -> Catalog` (dedupe by id, EL takes precedence when both define the id); `impl Catalog { pub fn lane_for(&self, model_id: &str) -> Option<Lane> }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_model_on_both_lanes_prefers_el_but_is_still_known_to_go() {
    let go = vec![m("deepseek-v4.1-flash", Lane::Go)];
    let el = vec![m("deepseek-v4.1-flash", Lane::Experiential)];
    let c = Catalog::merged(go, el);
    assert_eq!(c.lane_for("deepseek-v4.1-flash"), Some(Lane::Experiential));
    assert_eq!(c.models.len(), 1);
}

#[test]
fn an_el_only_model_resolves_to_el() {
    let c = Catalog::merged(vec![m("kimi-k3", Lane::Go)], vec![m("clef", Lane::Experiential)]);
    assert_eq!(c.lane_for("clef"), Some(Lane::Experiential));
}

#[test]
fn an_unknown_model_resolves_to_none() {
    assert_eq!(Catalog::merged(vec![], vec![]).lane_for("nope"), None);
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement.** Keep the Go entry available as a fallback target for a shared id (store `go_fallback: bool` or keep a parallel map) so Task 18 can route the *same* id to Go when EL is exhausted.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): merged catalog with EL-preferred lane resolution"
```

---

### Task 15: OpenCode provider snippet generation and write

**Files:**
- Create: `crates/core/src/models/opencode.rs`, `crates/core/tests/opencode.rs`
- Modify: `crates/core/src/models/mod.rs:1`

**Interfaces:**
- Consumes: `Catalog`, `ModelInfo`.
- Produces: `pub fn generate_snippet(models: &[ModelInfo], proxy_base_url: &str) -> serde_json::Value` producing the v2 `{"providers":{"ocrouter":{...}}}` object; `pub fn write_opencode_config(config_path: &Path, snippet: &serde_json::Value) -> anyhow::Result<PathBuf>` (backup to `<name>.bak-<unix_ts>`, merge only the `providers.ocrouter` key, preserve everything else); `pub fn find_config_path() -> Option<PathBuf>` honouring `OPENCODE_CONFIG` then `~/.config/opencode/opencode.json`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn snippet_uses_v2_providers_shape() {
    let v = generate_snippet(&[m("kimi-k3", Lane::Go)], "http://127.0.0.1:18905/v1");
    let p = &v["providers"]["ocrouter"];
    assert_eq!(p["package"], "@opencode/ai/providers/openai-compatible");
    assert_eq!(p["settings"]["baseURL"], "http://127.0.0.1:18905/v1");
    assert!(p["models"]["kimi-k3"].is_object());
}

#[test]
fn write_preserves_unrelated_keys_and_backs_up() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("opencode.json");
    std::fs::write(&cfg, r#"{"$schema":"x","model":"openai/gpt","provider":{"google":{}}}"#).unwrap();
    let backup = write_opencode_config(&cfg, &generate_snippet(&[m("kimi-k3", Lane::Go)], "http://127.0.0.1:18905/v1")).unwrap();
    assert!(backup.exists());
    let out: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(out["model"], "openai/gpt");
    assert!(out["provider"]["google"].is_object());
    assert!(out["providers"]["ocrouter"]["models"]["kimi-k3"].is_object());
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement.** `write_opencode_config` must never drop existing top-level keys; it only inserts/overwrites `providers.ocrouter`.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): OpenCode v2 provider snippet generation with safe write"
```

---

### Task 16: Proxy server core (streaming passthrough)

**Files:**
- Create: `crates/cli/src/proxy_server.rs`
- Modify: `crates/cli/src/main.rs:1`, `crates/cli/Cargo.toml`

**Interfaces:**
- Consumes: `Config`, `Upstream`, `upstream_for`.
- Produces: `pub struct ProxyServer` with `new(cfg: Arc<Config>, deps: ProxyDeps)`, `async fn run(self, listener: TcpListener) -> anyhow::Result<()>`; `pub fn build_upstream_url(base: &str, incoming_path_and_query: &str) -> String`; `pub fn strip_lane_prefix(path: &str) -> (Option<Lane>, String)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn upstream_url_does_not_duplicate_v1() {
    assert_eq!(build_upstream_url("https://opencode.ai/inference/go", "/v1/chat/completions?x=1"),
               "https://opencode.ai/inference/go/v1/chat/completions?x=1");
    assert_eq!(build_upstream_url("https://opencode.ai/inference/go/v1", "/v1/messages"),
               "https://opencode.ai/inference/go/v1/messages");
}

#[test]
fn lane_prefix_is_stripped() {
    assert_eq!(strip_lane_prefix("/go/v1/chat/completions"), (Some(Lane::Go), "/v1/chat/completions".into()));
    assert_eq!(strip_lane_prefix("/v1/chat/completions"), (None, "/v1/chat/completions".into()));
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with `hyper`. Body is streamed with `Body::wrap_stream`; **do not** call `.collect()` on a response body. Request bodies are read into a `Bytes` once (needed for `model` inspection and `include_usage`), capped only by the upstream's own limit.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-cli proxy_server::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): streaming proxy core with lane prefix routing"
```

---

### Task 17: Header injection, session ensure, cache passthrough

**Files:**
- Create: `crates/core/src/proxy/headers.rs`, `crates/core/tests/headers.rs`
- Modify: `crates/core/src/proxy/mod.rs:1`

**Interfaces:**
- Consumes: `Provider`, `AuthStyle`.
- Produces: `pub fn build_upstream_headers(client: &HeaderMap, secret: &str, host: &str, auth: AuthStyle, session: &str) -> HeaderMap`; `pub fn ensure_go_session(headers: &mut HeaderMap, fallback: Option<&str>)`; `pub const PASSTHROUGH: &[&str]` (the cache/session headers preserved verbatim); `pub fn generate_session_id() -> String` (prefix `ses_`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn both_auth_headers_are_sent_for_go() {
    let h = build_upstream_headers(&HeaderMap::new(), "oc_x", "opencode.ai", AuthStyle::BearerAndApiKey, "ses_1");
    assert_eq!(h["authorization"], "Bearer oc_x");
    assert_eq!(h["x-api-key"], "oc_x");
}

#[test]
fn el_sends_only_bearer() {
    let h = build_upstream_headers(&HeaderMap::new(), "xpl_x", "api.experientiallabs.ai", AuthStyle::BearerOnly, "ses_1");
    assert_eq!(h["authorization"], "Bearer xpl_x");
    assert!(!h.contains_key("x-api-key"));
}

#[test]
fn go_lane_synthesises_a_session_when_the_client_sent_none() {
    let mut h = HeaderMap::new();
    ensure_go_session(&mut h, None);
    let v = h["x-opencode-session"].to_str().unwrap();
    assert!(v.starts_with("ses_"));
}

#[test]
fn go_lane_derives_from_prompt_cache_key_when_present() {
    let mut h = HeaderMap::new();
    ensure_go_session(&mut h, Some("abc"));
    assert_eq!(h["x-opencode-session"], "ses_abc");
}

#[test]
fn client_cache_headers_survive() {
    let mut client = HeaderMap::new();
    client.insert("prompt-cache-key", "keep-me".parse().unwrap());
    client.insert("anthropic-beta", "prompt-caching-2024-07-31".parse().unwrap());
    let out = build_upstream_headers(&client, "k", "h", AuthStyle::BearerOnly, "ses_1");
    assert_eq!(out["prompt-cache-key"], "keep-me");
    assert_eq!(out["anthropic-beta"], "prompt-caching-2024-07-31");
}
```

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-core headers::` — Expected: FAIL.

- [ ] **Step 3: Implement.** Hop-by-hop headers (`connection`, `keep-alive`, `transfer-encoding`, `te`, `trailer`, `upgrade`, `proxy-*`) are dropped; the client's `authorization`/`x-api-key` are overridden by the selected key. `PASSTHROUGH` = `x-opencode-session`, `x-session-id`, `prompt-cache-key`, `x-session-affinity`, `anthropic-beta`, `cache-control`, `user-agent`.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-core headers::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): upstream header construction and Go session synthesis"
```

---

### Task 18: Request lifecycle — select, fetch, fail over, stream, record

**Files:**
- Create: `crates/core/src/proxy/decision.rs`, `crates/core/tests/decision.rs`
- Modify: `crates/core/src/proxy/mod.rs:1`

**Interfaces:**
- Consumes: `KeyManager`, `CircuitBreaker`, `Allowance`, `Catalog`.
- Produces: `pub enum LaneChoice { Experiential, Go }`; `pub fn choose_lane(model: &str, catalog: &Catalog, allowance: &Allowance, el_org: Option<&str>) -> Option<LaneChoice>`; `pub enum Attempt { Retry, Stop }`; `pub fn classify_response(status: u16, headers: &HeaderMap, body_peek: &str, key_id: &str, now_ms: u64, cb: &mut CircuitBreaker, km: &mut KeyManager, allowance: &mut Allowance, org: Option<&str>, provider: Provider) -> Attempt`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_shared_model_goes_to_el_first_then_go_when_the_budget_is_gone() {
    let catalog = Catalog::merged(vec![m("deepseek-v4.1-flash", Lane::Go)], vec![m("deepseek-v4.1-flash", Lane::Experiential)]);
    let mut a = Allowance::new(1_000_000, 0.0);
    assert_eq!(choose_lane("deepseek-v4.1-flash", &catalog, &a, Some("org-1")), Some(LaneChoice::Experiential));
    a.record("org-1", 1_000_000, 0);
    assert_eq!(choose_lane("deepseek-v4.1-flash", &catalog, &a, Some("org-1")), Some(LaneChoice::Go));
}

#[test]
fn a_go_only_model_ignores_the_el_budget() {
    let catalog = Catalog::merged(vec![m("kimi-k3", Lane::Go)], vec![]);
    let a = Allowance::new(1_000_000, 0.0);
    assert_eq!(choose_lane("kimi-k3", &catalog, &a, None), Some(LaneChoice::Go));
}

#[test]
fn a_shared_model_that_el_rejected_is_still_reachable_on_go() {
    let catalog = Catalog::merged(vec![m("deepseek-v4.1-flash", Lane::Go)], vec![m("deepseek-v4.1-flash", Lane::Experiential)]);
    let a = Allowance::new(1_000_000, 0.0);
    // `None` for el_org means EL is unusable (no key, or the org was refused)
    assert_eq!(choose_lane("deepseek-v4.1-flash", &catalog, &a, None), Some(LaneChoice::Go));
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement.** `el_org: Option<&str>` — `None` means the EL lane is unusable (no EL key configured, or the org was refused), and any model that exists only on EL returns `None`. In `classify_response`: a quota 4xx marks the key exhausted and returns `Retry`; a 5xx records one failure (returning `Retry` only if a different key remains) and must **not** open a breaker when this same body already produced a 5xx on another key; 2xx records success and, for `Provider::Experiential`, adds `usage.total()` to the org's allowance.

- [ ] **Step 4: Run to verify it passes** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): lane choice and retry classification"
```

---

### Task 19: Wire the proxy event loop

**Files:**
- Modify: `crates/cli/src/proxy_server.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: request handling that, per request: reads the body once → inspects `model` → `choose_lane` → selects a key from that lane's pool → `ensure_go_session` on the Go lane → builds headers → fetches upstream → on `Attempt::Retry` picks the next key/lane → streams the response → parses usage → records to `Allowance` and `EventBus`.

- [ ] **Step 1: Write the failing test**

Because this wires I/O, the test is a **decision-path unit test** with a fake fetcher: implement `trait UpstreamClient { async fn send(&self, req) -> Result<Response> }` behind the server, and in the test assert that a first-call 429 on the EL key results in a second call to the **Go** URL with the same body. Keep the fake in `crates/cli/tests/lifecycle.rs`.

```rust
#[tokio::test]
async fn el_quota_falls_over_to_go_with_the_same_body() {
    let fake = FakeUpstream::new(vec![resp(429, r#"{"error":{"code":"free_limit_reached"}}"#), resp(200, "{}")]);
    let out = run_once(&fake, request_for("deepseek-v4.1-flash", "/v1/chat/completions")).await;
    assert_eq!(out.calls.len(), 2);
    assert!(out.calls[0].url.contains("/el/") || out.calls[0].url.contains("api.experientiallabs.ai"));
    assert!(out.calls[1].url.contains("opencode.ai"));
    assert_eq!(out.calls[0].body, out.calls[1].body);
}
```

- [ ] **Step 2: Run to verify it fails** — Run: `cargo test -p ocrouter-cli lifecycle::` — Expected: FAIL.

- [ ] **Step 3: Implement** the loop against the `UpstreamClient` trait; the real impl uses `reqwest` with `bytes_stream()`. Inject `stream_options.include_usage = true` only for streamed chat-completions.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-cli lifecycle::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): proxy request lifecycle with cross-lane failover"
```

---

### Task 20: Control API (token-gated, 127.0.0.1:18904)

**Files:**
- Create: `crates/cli/src/control_api.rs`, `crates/cli/tests/control_api.rs`
- Modify: `crates/cli/src/main.rs:1`

**Interfaces:**
- Produces routes: `GET /healthz`, `GET /api/keys`, `POST /api/keys`, `DELETE /api/keys/:id`, `POST /api/keys/:id/test`, `GET /api/status`, `GET /api/models`, `PUT /api/strategy`, `GET /api/events` (SSE), `GET /api/models/opencode-snippet`. Token read from `~/.config/ocrouter/control.token` (created 0600 on first run); every `/api/*` route requires `Authorization: Bearer <token>`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn api_requires_the_token() {
    let app = test_app();
    assert_eq!(call(&app, "GET", "/api/keys", None).await.status(), 401);
    assert_eq!(call(&app, "GET", "/api/keys", Some("Bearer wrong")).await.status(), 401);
    assert_eq!(call(&app, "GET", "/api/keys", Some(&format!("Bearer {}", app.token()))).await.status(), 200);
}

#[tokio::test]
async fn key_listing_never_leaks_a_secret() {
    let app = test_app_with_key("oc_VERYSECRET1234");
    let body = call(&app, "GET", "/api/keys", Some(&bearer(&app))).await.text().await.unwrap();
    assert!(!body.contains("VERYSECRET"));
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with `axum`; bind `127.0.0.1` only. `GET /healthz` is unauthenticated (it reports only ok/key counts).

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-cli control_api::` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): token-gated local control API"
```

---

### Task 21: CLI, service lifecycle, and live verification

**Files:**
- Create: `crates/cli/src/cli.rs`, `crates/cli/src/service/launchd.rs`, `crates/cli/src/service/mod.rs`
- Modify: `crates/cli/src/main.rs:1`

**Interfaces:**
- Consumes: all.
- Produces subcommands: `ocrouter serve`, `ocrouter key add --provider <p> --label <l> [--secret -]`, `ocrouter key list`, `ocrouter key rm <label>`, `ocrouter models sync [--write]`, `ocrouter status`, `ocrouter strategy <name>`, `ocrouter service install|uninstall|status`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn key_add_then_list_shows_only_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys.toml");
    run(["key","add","--provider","opencode-go","--label","go-1","--secret","oc_abcdef123456","--keys", keys.to_str().unwrap()]);
    let out = run_capture(["key","list","--keys", keys.to_str().unwrap()]);
    assert!(out.contains("go-1"));
    assert!(!out.contains("abcdef"));
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** with `clap` derive. `service install` writes `~/Library/LaunchAgents/com.ocrouter.daemon.plist` with `RunAtLoad` and `KeepAlive`, then `launchctl load`. `models sync` prints by default and writes only with `--write`.

- [ ] **Step 4: Run to verify it passes** — Run: `cargo test -p ocrouter-cli cli::` — Expected: PASS.

- [ ] **Step 5: Live verification (needs the owner's keys)**

```bash
ocrouter key add --provider opencode-go --label go-1 --secret -
ocrouter key add --provider experiential --label el-1 --secret -
ocrouter serve &
curl -s http://127.0.0.1:18904/healthz
ocrouter models sync            # expect Go + EL models, no error
curl -s http://127.0.0.1:18905/v1/models | head
# Go lane, with a synthesised session (client sends none):
curl -s -X POST http://127.0.0.1:18905/go/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"kimi-k3","messages":[{"role":"user","content":"hi"}],"stream":true}'
```

Expected: healthz ok; `models sync` lists EL free-core models (`gpt-6-luna`, `deepseek-v4.1-flash`, `clef`); the Go call returns a streamed 200 (proves Task 17's session synthesis against the real 400 `MissingSessionID`).

- [ ] **Step 6: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): CLI, launchd service, and live verification"
```

---

### Task 22: Docs, parity check, and TypeScript removal

**Files:**
- Modify: `README.md`, `AGENTS.md`, `PRD.md`, `implementation_plan.md`
- Delete (last): `src/`, `dist/`, `package.json`, `package-lock.json`, `.opencode-plugin/`

**Interfaces:** none.

- [ ] **Step 1: Run the parity checklist** from the spec §17 against the running daemon; record each result.
- [ ] **Step 2: Update the docs** to the v2 `providers` shape, `/inference` endpoints, the EL lane, and the free-tier limitation (no bypass).
- [ ] **Step 3: Only after the owner has run the Rust proxy for a day**, remove the TS tree in its own commit, with `chore: remove replaced typescript implementation` as the message.
- [ ] **Step 4: Commit**

```bash
git add README.md AGENTS.md PRD.md implementation_plan.md
git commit -m "docs: describe the rust ocrouter"
```

---

## Plan Self-Review

- **Spec coverage:** intent/goals §3 → Tasks 1–22; endpoint migration §2 → Global Constraints + Task 4; EL-first §9 → Tasks 9, 14, 18; Go session requirement §8 → Task 17; model discovery §10 → Tasks 13–15; file keys §11 → Task 3; strategies/breaker/affinity/state §12 → Tasks 5–8, 11; control API §13 → Task 20; lifecycle §14 → Task 21; migration §15 → Task 22. GPUI (§13, dashboard) is **plan 2** by design.
- **Step scan:** each step names one action with a checkable result; no "handle edge cases" placeholders.
- **Type consistency:** `choose_lane` is defined once (Task 18) with the same signature used in Task 19. `Allowance::record`, `KeyManager::select`, `CircuitBreaker::record_failure`, `generate_snippet`, `write_opencode_config` each appear with one signature. `Lane` (Task 13) is reused by Tasks 14, 16, 18.
- **Review Focus:** item 1 → Tasks 6, 18; item 2 → Task 17; item 3 → Task 9; item 4 → Tasks 14, 18; item 5 → Task 16.
- **Proportion:** deliberately coarse — bodies appear only where the algorithm is not determined by signature + test (quota/e cooldown ordering, lane choice, snippet merge).
