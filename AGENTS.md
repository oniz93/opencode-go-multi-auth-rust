# AGENTS.md

> Repo-specific notes for OpenCode sessions working in `ocrouter`. Keep this
> file short.

## What this repo is

A Rust workspace that pools multiple OpenCode Go API keys and an Experiential
Labs key behind one local proxy, with a token-gated control API. The previous
TypeScript implementation still sits in `src/` and `package.json`; it is kept
only until the Rust daemon has been exercised for a day, then deleted.

Design: `docs/superpowers/specs/2026-11-18-ocrouter-rust-design.md`
Plan: `docs/superpowers/plans/2026-11-18-ocrouter-daemon.md`

## Build, test, run

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # rustup cargo, not Homebrew's x86_64 one
cargo build --release
cargo test
cargo run -- serve
```

`.cargo/config.toml` pins `aarch64-apple-darwin`. Verify a binary with
`file target/aarch64-apple-darwin/debug/ocrouter` — it must say `arm64`.
No linter or formatter config exists beyond rustfmt defaults.

## Layout

```
crates/core   ocrouter-core — config, keys, router, quota, models. No HTTP, no UI.
crates/cli    ocrouter-cli  — lib (engine, control_api, proxy_server, cli) + bin `ocrouter`.
crates/gui    ocrouter-gui  — GPUI dashboard. A control-API client + `.app` bundle.
```

`crates/core` must never depend on HTTP-server or UI crates. `crates/gui` must
never be a dependency of `ocrouter-cli`; it only talks to the control API over
HTTP. The GUI polls on a std thread because gpui's executor is not tokio.

The macOS bundle is assembled by `scripts/bundle-macos.sh` into
`dist/Ocrouter.app` (gitignored). It contains both binaries; the GUI starts the
sibling `ocrouter serve` when `18904/healthz` does not answer.

Toolchain note: `gpui-pre` needs a stable rustc with `slice_as_array` (1.98+),
so `crates/gui` declares `rust-version = "1.98"` while the daemon still builds
on 1.91.

## Invariants — do not regress

- **Byte-for-byte bodies.** The only permitted mutations are
  `stream_options.include_usage = true` on streamed chat-completions
  (`engine::prepare_body`) and, for a `[[routing.route]]` step, `model` set to
  that step's upstream id (`engine::with_model`). Nothing else rewrites ids.
- **Routes** (`config.toml` `[routing]`, edited in the GUI) walk their steps in
  order; any 429/402 cools that key and moves on. Cooled keys and a spent EL
  org allowance are skipped without a request. `expose` filters `/v1/models`.
- **Route on path and `model` only.** Never read `tools` / `messages` /
  `tool_use`.
- **Go requires `x-opencode-session`.** Synthesise it when the client omits it
  (once per request in `Engine::handle`, so retries share it). Missing it
  returns `400 MissingSessionID`.
- **Both auth headers** go upstream for Go/Console (`Authorization` + `x-api-key`);
  Experiential gets `Authorization` only.
- **React, do not predict.** Cooldown comes from the upstream 4xx
  (`proxy::quota`). Never add a local quota accumulator to routing.
- **Only a request's first failure (5xx or transport error) counts against a
  breaker** (`ResponseSignal::count_breaker_failure`). Breakers open for
  `breaker_open_ms` (60 s), not the 5 h quota `cooldown_ms`.
- **Never forward `accept-encoding`.** The proxy has no decompressor and must
  read error bodies for quota markers.
- **The proxy is local-only** (`proxy_server::is_local_request`): no browser
  `Origin`, loopback `Host` only.
- **No wall-clock upstream timeout** by default.
- **The Experiential allowance is per org**, keyed on the `org_id` from
  `GET /api/whoami`, reset 00:00 UTC (`router::allowance`).

## Upstreams

| Lane   | Base URL                              | Auth                  |
|--------|---------------------------------------|-----------------------|
| Go     | `https://opencode.ai/inference/go`    | Bearer + `x-api-key`  |
| Console| `https://opencode.ai/inference`       | Bearer + `x-api-key`  |
| EL     | `https://api.experientiallabs.ai/v1`  | Bearer                |

`/zen/*` still answers as an alias today but is not the documented home.

## Secrets

Keys live in `~/.config/ocrouter/keys.toml` (0600). Never log, print, or commit
a secret. `ApiKey` deliberately does not implement `Serialize`; `KeyHealth`
(which is persisted) has no secret field.

## Testing

`cargo test` runs unit + integration tests. The engine is exercised through the
`UpstreamClient` trait with a scripted fake (`crates/cli/tests/lifecycle.rs`), so
no network is needed. Live verification against the real upstreams needs the
owner's keys and is not part of `cargo test`.
