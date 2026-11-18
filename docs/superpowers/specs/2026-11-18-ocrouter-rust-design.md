# ocrouter — Rust rewrite of opencode-go-multi-auth

**Date:** 2026-11-18 (rev 2 — review findings resolved)
**Status:** Draft for approval
**Supersedes:** the TypeScript/Node implementation in this repository (`src/**`, `package.json`, `dist/**`)

## 1. Intent

One local endpoint that fronts several coding-agent accounts, so a session never
stops because one account ran out. The existing TypeScript project already does
this for OpenCode Go API keys. We are rebuilding it as a **single Rust binary**
for three reasons the owner stated:

1. No Node/TypeScript runtime process sitting on the machine.
2. Lightweight, low-friction to run (start on login, forget about it).
3. Add a second upstream — **Experiential Labs** — and spend its free daily
   allowance *before* touching a paid Go account.

A native **GPUI** dashboard replaces the browser dashboard: keys, health, live
logs, and per-account allowances in a resizable window, no web server to poke at.

## 2. Why now

- The Go/Console documentation moved. The proxy forwards to
  `https://opencode.ai/zen/go/v1` and `https://opencode.ai/zen/v1`; the
  [console docs](https://opencode.ai/docs/console) now publish
  `https://opencode.ai/inference/go` and `https://opencode.ai/inference`.
  `/zen/*` currently still answers (verified: identical payloads), but it is no
  longer the documented home.
- OpenCode is on **v2.3.2** here; its config surface changed. The repo's docs
  still describe the deprecated v1 shape (`provider` + `npm` + `options`). V2
  uses `providers` + `package`/`canonical` + `settings` (the singular `provider`
  schema key is marked `"deprecated": true`).
- Experiential Labs is a new required upstream.
- The Go upstream **now hard-requires `x-opencode-session`** (verified: a
  request without it returns `400 MissingSessionID`).

## 3. Goals

- Pool N OpenCode **Go** API keys behind one base URL, with automatic
  failover and session stickiness. *(owner's primary goal)*
- **Try Experiential Labs first** and fall through to Go automatically, so the
  free daily allowance is spent before a paid account is touched.
  *(owner's additional goal)*
- Discover available models automatically from every upstream and keep the
  OpenCode config in sync without hand-editing model lists.
- One self-contained, fast binary. No Node. Start-on-login.
- A small native dashboard (GPUI) for keys, status, and logs.
- Preserve the cache behaviour that makes the proxy worth using: never break
  prompt-cache headers, never turn a cache hit into a miss gratuitously.

## 4. Non-goals

- **No free-tier bypass.** Zen `*-free` models reject pooled API keys by
  policy (upstream issue #11; the project owner explicitly refuses this). We
  surface the upstream error clearly and do not attempt to defeat it.
- **No auto-login / credential scraping.** Keys are pasted by the owner.
- **No npm/OpenCode plugin.** OpenCode integration is config-only plus an
  `ocrouter` CLI. (The JS plugin entrypoint in the TS version is dropped.)
- **No remote notifications / webhooks.** Local logs only.
- **No web dashboard.** GPUI replaces it. A small local JSON control API
  remains, for the CLI and for scripting.
- **No spending the owner's Experiential credits.** The proxy stops at the free
  daily allowance and falls back to Go; it never enables or relies on
  "Continue on credits".

## 5. Anti-goals (would make this a failure even if it works)

- A binary that is harder to install than `npm i -g`. One step to build/launch.
- A proxy that corrupts or buffers streaming responses, or that parses
  tool-call payloads to make routing decisions.
- Predicting quota from a local cost accumulator. §9 — we react to the
  upstream, we do not predict it.
- A GPUI dependency that makes the *daemon* fail to build. The daemon must
  build and run with zero GUI dependencies.
- A request that starts free and silently ends up billed to a credit card.

## 6. Constraints

- macOS (darwin/aarch64) is the first target; Linux should follow. Rust 1.98,
  Xcode present.
- **Pin the native target.** The `cargo`/`rustc` on `PATH` is Homebrew's
  **x86_64** build (`host: x86_64-apple-darwin`) even though the machine is
  arm64 and the rustup default toolchain is `aarch64-apple-darwin`. Without
  pinning, every build is a Rosetta binary. A `.cargo/config.toml` with
  `[build] target = "aarch64-apple-darwin"` is mandatory.
- Ports stay **18905** (proxy) and **18904** (control API) so existing
  `opencode.json` base URLs keep working.
- The proxy is a **byte-for-byte pass-through** of request and response bodies.
  The only body mutation permitted is `stream_options.include_usage = true` on
  streamed OpenAI chat-completions requests, matching the current code.
  *(Model IDs are never rewritten — see §9.)*
- **Headers may be added** where the upstream requires it to route at all: the
  Go lane must carry `x-opencode-session` (§8).
- No hard-coded upstream wall-clock timeout. Optional, env-gated, default off.
- Secrets never appear in logs, the control API, or the repo.

## 7. Decisions already made

**Owner's calls:**

- Language: **Rust**.
- Dashboard: **GPUI**, if Rust. (Small.)
- Routing: **Experiential Labs first**, then Go.
- Purpose: juggle multiple OpenCode Go accounts.

**My calls — flag any that are wrong:**

- **Unified lane is the default client surface.** One provider in
  `opencode.json` pointing at `http://127.0.0.1:18905/v1`, serving a **merged
  model catalog**, with the lane chosen **per model**: a model that Experiential
  Labs serves goes to EL first (free allowance), everything else — and
  everything once EL's allowance is spent — goes to Go. This is the only shape
  in which "Experiential goes first" is real rather than a manual provider
  switch. Reading `model` from the body for lane selection is the one
  body-peek allowed; the router still never parses messages/tools.
  The explicit `/go/*` and `/el/*` lanes remain as escape hatches and for
  testing.
- **Workspace, not a rewrite-in-place of `src/`.** New Rust workspace; the
  TypeScript sources stay runnable until Rust reaches parity (§17), then are
  removed. The repo keeps its name.
- **Personal single-machine tool**, not a distributable artifact. Built with
  `cargo build --release`; installed at a stable path.
- **Keys live in a plain file**, not the OS keychain, so the setup ports to
  Linux and Windows by copying one file (§11). Default is plaintext at `0600`;
  passphrase encryption is opt-in.
- **`launchd` agent** installed by `ocrouter service install` so it starts at
  login. Not a shell alias.
- **Control API on 18904** binds `127.0.0.1` only and requires a token stored in
  the config dir (0600). The GPUI app and CLI both use it.
- **OpenCode model catalog is inherited, not regenerated, for the Go half**
  (via `canonical` — see §10). The EL half is generated.

## 8. Architecture

Cargo workspace:

```
ocrouter/
├── crates/
│   ├── core/    ocrouter-core   — config, key store, router, quota, upstreams,
│   │                              model catalog. No HTTP, no UI.
│   ├── daemon/  ocrouterd       — proxy + control API binary. NO gpui dep.
│   └── gui/     ocrouter-gui    — GPUI dashboard binary. Depends on gpui-kit.
└── scripts/                     — model-sync helper, release packaging
```

Why split the daemon from the GUI: `gpui-kit` pulls ~600 crates and is young.
The daemon must never be coupled to that.

**Runtime:** `tokio` + `hyper`. The proxy uses hyper directly (manual body
streaming, no framework buffering). The control API uses `axum` (hyper-based).

**Request flow:**

```
OpenCode ──▶ 127.0.0.1:18905/v1/<path>     (unified lane; /go and /el also exist)
                │
                ├─ read body once (unmodified) — only `model` is inspected
                ├─ choosing lane: if model ∈ EL catalog and EL budget remains → EL
                │                   else → Go   (allowance gone → Go)
                ├─ select key  (strategy + session affinity + cooldown filter)
                ├─ ensure x-opencode-session on the Go lane (synthesize if absent)
                ├─ build upstream URL + headers (inject key, preserve cache headers)
                ├─ fetch upstream  (abort on client disconnect)
                ├─ on 402/429 quota → cooldown key, retry next key / next lane
                ├─ on repeated 5xx  → circuit breaker, retry next key
                └─ stream response back verbatim; record usage for display
```

Invariants carried over from the TS proxy (do not regress):

- Both `Authorization: Bearer …` **and** `x-api-key: …` are sent upstream.
- `x-opencode-session`, `x-session-id`, `prompt-cache-key`, and cache headers
  are forwarded unmodified when present.
- **The Go lane always carries `x-opencode-session`.** If the client omits it,
  the proxy derives a stable id from `prompt-cache-key` when available, else
  generates a per-conversation id, and feeds that same id into session
  affinity. (This *is* inventing one — Go requires it; the old "never invent"
  rule is superseded.) EL/Console requests pass it through when present.
- Quota cooldown is taken from the upstream's own 4xx signals, in order:
  `Retry-After` → `x-codex-*-reset-*` → `error.retry_after_ms` /
  `error.retry_after` / `error.resets_at` → "retry after N m/h" text →
  fallback constant (5h).
- Only genuine quota errors trigger cooldown; a plain rate limit does not.
- The first 5xx of a request may open a breaker; a repeated 5xx (same body,
  different key) must not open every breaker.
- A cold-start after failover logs a "cache miss on new key" warning.
- Client disconnect aborts the upstream fetch.

## 9. Providers, upstreams, and the free allowance

| Lane   | Prefix | Base URL (default)                     | Auth | Model list |
|--------|--------|----------------------------------------|------|-----------|
| Go     | `/go`  | `https://opencode.ai/inference/go`     | Bearer + `x-api-key` | `GET /v1/models` |
| Console| `/oc`  | `https://opencode.ai/inference`        | Bearer + `x-api-key` | `GET /v1/models` |
| EL     | `/el`  | `https://api.experientiallabs.ai/v1`   | Bearer only | `GET /v1/models` (key) · `GET /api/models` (keyless catalog) |

Console (`/oc`) is optional and off unless the owner adds a Console key; the
primary pair is Go + EL.

**Experiential free core (verified against `/llms.txt`).** Free core covers
**GPT-6 Luna, DeepSeek V4.1 Flash and Clef** on their **plain** model ids
(no `:free` suffix), at 10 req/min and 250k input tokens/min per model, sharing
**1,000,000 input+output tokens/day**, reset at **00:00 UTC**. Past it the
gateway answers `429 free_limit_reached`; separate promotions use
`<slug>:free`, which never overflows to paid.

Consequences for the proxy:

- **Send the plain slug. Never rewrite a model id.** (`:free` is for separate
  promotions, not free core, and rewriting would break §6.)
- Track a local **daily** token counter and stop at a **safety margin** below
  1M (start at 95%), then route to Go. This is what keeps the "no credit spend"
  promise when the local count drifts from the org's.
- **Counter is grouped by organisation**, not by key. Resolve the org with
  `GET {el-management}/api/whoami` (`org_id`) and key the counter on it, so two
  keys from one org cannot each claim the full 1M.
- React to `free_limit_reached` / `insufficient_quota` (`429`) by cooling the
  key until the next UTC midnight or the reset the upstream names.
- **Never enable "Continue on credits."** That is the owner's account setting;
  we document it and never touch it.
- If the owner *wants* to spend credits past the free tier, that is an explicit
  config switch (`el.allow_paid = true`), off by default.

**Cost / usage.** Upstream `cost` (when present) and locally estimated cost
(published rate card) are **display-only**. They never drive routing. Go
responses may carry `opencode.allowance`; record it for display.

## 10. Model discovery and OpenCode integration

OpenCode v2 does **not** auto-discover models for a generic OpenAI-compatible
provider — you list them, or you inherit a catalog. The unified lane needs one
merged catalog, which we generate and keep fresh:

1. **Go half — inherit the catalog** where possible: a `canonical:
   "opencode-go"` provider with `settings.baseURL` overridden. *(Verify in a
   spike; fall back to generating the Go half too if `canonical` + `baseURL`
   does not route.)*
2. **EL half — generate.** `ocrouter models sync` reads the EL catalog
   (`GET /api/models` keyless for metadata, `GET /v1/models` per key for the
   callable set) and maps each model to an OpenCode entry (`modelID`, `name`,
   `limit`, `cost`, `capabilities`, `modalities`, `compatibility.cacheFormat`).

Because the client surface is unified, the generated provider block lists the
**merged** catalog. `ocrouter models sync` writes
`~/.config/opencode/opencode.json` (v2 `providers` shape) with a timestamped
backup, or prints the JSON. Writing is opt-in (`--write`); printing is default.

The proxy also serves `GET /v1/models` (merged, all lanes) for tooling and the
dashboard.

**Unified provider skeleton:**

```jsonc
{
  "providers": {
    "ocrouter": {
      "name": "ocrouter (EL → Go)",
      "package": "@opencode/ai/providers/openai-compatible",
      "settings": {
        "baseURL": "http://127.0.0.1:18905/v1",
        "apiKey": "ocrouter-local"
      },
      "models": { /* merged, generated */ }
    }
  }
}
```

The client `apiKey` is irrelevant — the proxy overrides credentials; it exists
only to satisfy the provider package. If the `canonical` spike succeeds we can
also offer a Go-only variant that inherits the built-in catalog and needs no
generated list.

## 11. Key storage

Keys live in a **plain file** so the setup ports across macOS, Linux and
Windows by copying one file: `~/.config/ocrouter/keys.toml`, mode `0600`, one
entry per key.

```toml
[[key]]
provider = "opencode-go"     # or "experiential" / "opencode-console"
label    = "go-1"
secret   = "oc_..."
priority = 1
weight   = 1
enabled  = true
```

- **Plaintext by default.** The owner chose portability over at-rest encryption
  and accepts the file is readable by their user account. Mode is enforced
  (`0600`), and the file is never written into logs, the control-API response,
  or the repo (`.gitignore` it).
- **Encryption is opt-in.** `ocrouter key encrypt` re-wraps the file under a
  passphrase (`age` or `chacha20poly1305`), for anyone who wants it. Once
  encrypted, `ocrouter` needs the passphrase at start (env
  `OCROUTER_PASSPHRASE` or a prompt) — a hurdle, so it is never the default.
- `OCROUTER_KEYS` overrides the path (containers, CI).
- Non-secret state (health, cooldowns, daily counters) lives in `state.json`
  and never contains secrets.
- Import from the TS store (`~/.opencode/router-keys.enc`) is a nice-to-have.

**Target deployment for v1:** 1 Experiential key, 3 OpenCode Go keys. The EL
daily counter is therefore a single org bucket, and the Go pool is a 3-way
failover. The code must still handle N of each.

## 12. Routing, health, and state

**Lane choice:** unified lane picks EL or Go per model (§7/§8). Within a lane:

**Strategies:** `priority_failover`, `round_robin`, `session_affinity`. The
EL-first behaviour is a property of the unified lane, not a strategy, so it
holds no matter which intra-lane strategy is selected.

**Circuit breaker:** per key, N consecutive 5xx opens it; half-open probe after
a cooldown. One request cannot open every breaker.

**Session affinity:** a conversation's session id maps to the key that warmed
its cache; sticky until that key is unavailable. Per lane, so Go and EL
preferences do not collide.

**State:** `~/.local/state/ocrouter/state.json` — per-key health, cooldowns,
per-org daily counters, recent events. Atomic writes (temp + rename), debounced.

**Config:** `~/.config/ocrouter/config.toml` — ports, upstream base URLs (all
overridable), EL daily cap + safety margin + `allow_paid`, strategy, per-provider
settings, log level.

## 13. Control API and GPUI dashboard

**Control API (127.0.0.1:18904, token-gated):**

- `GET /healthz`
- `GET /api/keys` (secrets masked) · `POST /api/keys` · `DELETE /api/keys/:id`
- `POST /api/keys/:id/test`
- `GET /api/status` — health, cooldowns, per-org EL allowance, last error,
  current lane split
- `GET /api/models` — merged catalog
- `PUT /api/strategy`
- `GET /api/events` — SSE stream of routing events
- `GET /api/models/opencode-snippet` — the JSON to paste

**GPUI dashboard** (`ocrouter-gui`, talks to the control API):

- Accounts list: provider badge, masked key, health dot, cooldown timer,
  EL daily bar (`used / 1M`, per org), tokens today.
- Strategy selector.
- Live log/event pane (SSE into a virtualized list).
- Add-key dialog (paste secret, pick provider, label).
- "Copy OpenCode snippet" / "Sync models" action.

The GUI is a client of the daemon; closing it never stops the daemon.

## 14. Service lifecycle and risks

**Lifecycle:** `ocrouter service install|uninstall|status` writes a `launchd`
plist (`~/Library/LaunchAgents/…`), `KeepAlive` on crash, `RunAtLoad` true.
`ocrouter` with no args starts the daemon if down, then opens the GUI.

**Risks / spikes (cheapest first):**

1. ~~**GPUI build**~~ — **CLEARED.** `gpui-kit` 0.10.0 compiled and linked a
   working binary from the hello-world in **5 m 45 s** (802 packages in
   `Cargo.lock`, ~105 MB debug artifact). Confirms a window can open here.
   The daemon still must not depend on it.
2. **`canonical` + `baseURL` routing** — confirm OpenCode v2 sends the Go
   catalog through an overridden base URL. If not, generate the Go half too.
3. **EL callable list + org id** — confirm `GET /el/v1/models` and
   `GET /api/whoami` with one real key.
4. **Byte-fidelity** — same-body double-5xx must not open every breaker;
   streaming must not buffer.
5. **Free-allowance accuracy** — local counter vs the org's real usage.

## 15. Migration and docs

- Keep the TS implementation runnable during development; cut over at parity
  (§17), then remove `src/`, `dist/`, `package.json`, and the plugin manifest
  from the tree (history keeps them).
- Update `README.md`, `AGENTS.md`, `PRD.md`, `implementation_plan.md` to
  describe the Rust design, the v2 `providers` config, `/inference` endpoints,
  and the Experiential lane.
- Adopt upstream **PR #12**'s idea (per-account usage visibility) natively in
  the dashboard rather than merging that PR into a codebase we are replacing.

## 16. Left to the builder

- HTTP crate choices inside hyper (`axum` vs raw), exact SSE framing, log
  rotation.
- The precise TOML schema, as long as it is hand-editable.
- Whether the GUI is one window or a tabbed window.
- Crate-level module layout, error types, test strategy (unit tests for the
  router and quota detector; end-to-end needs live keys).

## 17. Parity checklist before the TS tree is removed

1. Same ports, same passthrough invariants.
2. Go pool failover + circuit breaker + session affinity.
3. EL-first unified lane with daily allowance accounting.
4. Model sync writes a working v2 `providers` block.
5. GPUI dashboard covers key add/remove, status, and live logs.
6. `ocrouter service install` survives a reboot.
7. The owner's OpenCode session runs on the Rust proxy for a day without a
   fallback to the TS daemon.
