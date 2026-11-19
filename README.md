# ocrouter

One local endpoint that fronts several coding-agent accounts, so a session never
stops because one account ran out. It pools **OpenCode Go** API keys and spends
**Experiential Labs**' free daily allowance first.

Rebuilt in Rust as a single binary — no Node, no plugin, no web server.

> The TypeScript implementation in `src/` and `package.json` is the previous
> generation and is kept only until the Rust daemon has been exercised for a
> day. See `docs/superpowers/specs/2026-11-18-ocrouter-rust-design.md`.

## Build

```bash
# The `cargo` on PATH may be Homebrew's x86_64 build; use the rustup one.
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
# binary: target/aarch64-apple-darwin/release/ocrouter
```

### macOS app (dashboard + auto-start)

```bash
./scripts/bundle-macos.sh     # -> dist/Ocrouter.app
open dist/Ocrouter.app
```

`Ocrouter.app` opens the native GPUI dashboard and starts the background
daemon if `127.0.0.1:18904/healthz` is not already answering. Closing the
window does not stop the daemon.

`.cargo/config.toml` pins the native `aarch64-apple-darwin` target so a
Rosetta binary is never produced.

## Setup

1. Add your keys (stored in `~/.config/ocrouter/keys.toml`, mode `0600`):

   ```bash
   ocrouter key add --provider experiential  --label el-1  --secret 'xpl_…'
   ocrouter key add --provider opencode-go   --label go-1  --secret 'oc_…'
   ocrouter key add --provider opencode-go   --label go-2  --secret 'oc_…'
   ocrouter key list          # shows a masked preview only
   ```

2. Start it:

   ```bash
   ocrouter serve             # proxy :18905, control API :18904
   ```

   Or install a login agent:

   ```bash
   ocrouter service install
   ```

3. Point OpenCode at it. `ocrouter models sync --write` writes this block into
   your `opencode.json` (backing the file up first); without `--write` it prints
   the JSON for you to paste:

   ```jsonc
   {
     "providers": {
       "ocrouter": {
         "name": "ocrouter (Experiential then Go)",
         "package": "@opencode/ai/providers/openai-compatible",
         "settings": {
           "baseURL": "http://127.0.0.1:18905/v1",
           "apiKey": "ocrouter-local"
         },
         "models": { "kimi-k3": {}, "deepseek-v4.1-flash": {} }
       }
     }
   }
   ```

   Pick a model with `/models` as `ocrouter/<model-id>`. The `apiKey` is a
   placeholder; the proxy injects the pooled credentials.

## How routing works

Requests arrive on `:18905`. A path prefix picks a lane explicitly; without one
the model decides:

| Path        | Upstream                                   |
|-------------|--------------------------------------------|
| `/v1/...`   | per model: Experiential first, then Go     |
| `/el/...`   | `https://api.experientiallabs.ai/v1`       |
| `/go/...`   | `https://opencode.ai/inference/go`         |
| `/oc/...`   | `https://opencode.ai/inference` (Console)  |

A model both providers serve (for example `deepseek-v4.1-flash`) goes to
Experiential while the org has free allowance, then falls back to Go **with the
same request body**. A Go-only model goes straight to Go.

Failover reacts to what the upstream says, never to a local estimate:

- A quota 4xx (`402`, or `429` carrying `insufficient_quota`,
  `free_limit_reached`, …) cools the key for the reset the upstream names.
- A repeated 5xx on the same body does not open a second key's circuit breaker.
- The proxy synthesises `x-opencode-session` when a client omits it — Go rejects
  the request with `400 MissingSessionID` otherwise.

Prompt-cache headers (`x-opencode-session`, `x-session-id`, `prompt-cache-key`,
`anthropic-beta`, `cache-control`) are forwarded untouched. Bodies are relayed
byte-for-byte; the only mutation is `stream_options.include_usage` on streamed
chat-completions.

## Experiential free allowance

Free core covers **GPT-6 Luna, DeepSeek V4.1 Flash and Clef** on their plain
model ids, sharing **1,000,000 input+output tokens/day per organisation**,
resetting at 00:00 UTC. ocrouter tracks a local counter per org, stops at the
cap minus a 5% safety margin, and routes to Go. It never sends `<slug>:free`,
never rewrites a model id, and never enables "Continue on credits".

To allow paid overflow deliberately, set `el_allow_paid = true` in
`~/.config/ocrouter/config.toml`.

## Control API

`127.0.0.1:18904`, bearer-token guarded. The token lives in
`~/.config/ocrouter/control.token` (mode 0600, created on first start).

```
GET    /healthz                     (public)
GET    /api/keys                    masked list
POST   /api/keys                    add a key
DELETE /api/keys/{id}
GET    /api/status                  counts, strategy, allowance, recent events
GET    /api/models                  merged catalog
GET    /api/models/opencode-snippet
GET    /api/strategy                PUT to change
GET    /api/events                  SSE stream
```

## CLI

```
ocrouter serve
ocrouter key add|list|rm
ocrouter models sync [--write]
ocrouter status
ocrouter strategy [name]
ocrouter service install|uninstall|status
```

## Files

| Path                                   | Contents                          |
|----------------------------------------|-----------------------------------|
| `~/.config/ocrouter/config.toml`       | ports, upstream URLs, strategy    |
| `~/.config/ocrouter/keys.toml`         | API keys (0600)                   |
| `~/.config/ocrouter/control.token`     | control-API bearer token (0600)   |
| `~/.config/ocrouter/state.json`        | key cooldowns + today's EL usage  |
| `~/Library/Logs/ocrouter.*.log`        | launchd agent output              |

The daemon reloads `keys.toml` within a few seconds of a change, so
`ocrouter key add` needs no restart.

## Limitations

- Zen `*-free` models are **not** usable through the proxy: the free tier rejects
  pooled API keys by policy. Use paid Go/Console models.
- One `Engine` guards routing state, so requests take a short lock while a
  decision is made (not for the life of a stream).
- The proxy serves local programs only: requests with a non-loopback `Host` or
  a browser `Origin` get `403`.

## License

MIT.
