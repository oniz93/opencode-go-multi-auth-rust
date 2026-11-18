# ocrouter GPUI Dashboard + macOS .app Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A native GPUI window that shows ocrouter's accounts, allowance, strategy and live events, auto-starting the daemon, shipped as `Ocrouter.app`.

**Architecture:** New `crates/gui` binary in the same workspace. It never links into `ocrouter-cli`. A background std thread polls the existing control API (`127.0.0.1:18904`) into an `Arc<Mutex<UiState>>`; the GPUI view wakes every 500 ms, reads the snapshot, and repaints. The daemon is started once at launch if `GET /healthz` fails.

**Tech Stack:** `gpui-kit` 0.10 (pulls GPUI + components), `smol` (Timer), `reqwest` blocking + rustls, `serde_json`.

**Spec:** `docs/superpowers/specs/2026-11-18-ocrouter-rust-design.md` §13.

## Global Constraints

- `crates/gui` must never be a dependency of `ocrouter-cli` or `ocrouter-core`.
- The GUI is a **client of the control API**; it must not touch `keys.toml` or the routing engine directly.
- Closing the window must not stop the daemon; the daemon is a detached child.
- The control token is read from `~/.config/ocrouter/control.token`; it is never displayed.
- Build target stays `aarch64-apple-darwin`; the `.app` is ad-hoc signed (`codesign -s -`).

## Files

- `crates/gui/Cargo.toml` — gpui-kit, smol, reqwest(blocking,rustls,json), serde_json, ocrouter-core (for `find_config_path`/paths only)
- `crates/gui/src/main.rs` — app entry, window
- `crates/gui/src/state.rs` — `UiState`, token discovery, snapshot parsing
- `crates/gui/src/client.rs` — `ensure_daemon`, `poll_once`, the polling thread
- `crates/gui/src/view.rs` — `Dashboard` view + `Render`
- `scripts/bundle-macos.sh` — assemble and sign `Ocrouter.app`
- `Cargo.toml` — add `crates/gui` to members

## Task 1: GUI crate skeleton and snapshot types

**Files:** `crates/gui/Cargo.toml`, `src/state.rs`, workspace `Cargo.toml`
**Produces:** `UiState { connected, strategy, keys: Vec<KeyRow>, allowance: Vec<AllowanceRow>, events: Vec<String>, error: Option<String> }`, `parse_status(json, keys_json) -> UiState`, `control_token() -> Option<String>`.

- [ ] Step 1: failing test — `parse_status` reads strategy, key labels, allowance rows, and never a secret.
- [ ] Step 2: run it, expect FAIL (module missing).
- [ ] Step 3: implement `UiState` + `parse_status` + `control_token`.
- [ ] Step 4: run it, expect PASS.
- [ ] Step 5: commit.

## Task 2: Daemon ensure + polling thread

**Files:** `src/client.rs`
**Produces:** `ensure_daemon(bin: &Path) -> bool`, `spawn_poller(state: Arc<Mutex<UiState>>, base: String, token: Option<String>)`.

- [ ] Step 1: failing test — `is_healthy` returns false for a closed port and true for a served one (test spins a tiny TCP listener returning 200 on `/healthz`).
- [ ] Step 2: run it, expect FAIL.
- [ ] Step 3: implement `is_healthy`, `ensure_daemon` (spawn sibling `ocrouter serve`, wait up to 5 s), and the polling thread.
- [ ] Step 4: run it, expect PASS.
- [ ] Step 5: commit.

## Task 3: GPUI view

**Files:** `src/view.rs`, `src/main.rs`
**Produces:** `Dashboard` entity with `Render`; rows for keys, allowance bars, event list, strategy label, connection state.

- [ ] Step 1: `cargo build -p ocrouter-gui` compiles clean.
- [ ] Step 2: run the binary; a window opens showing connection state.
- [ ] Step 3: commit.

## Task 4: `.app` bundle

**Files:** `scripts/bundle-macos.sh`
**Produces:** `dist/Ocrouter.app/Contents/{MacOS/ocrouter-gui,MacOS/ocrouter,Info.plist,Resources}`.

- [ ] Step 1: script builds both bins for `aarch64-apple-darwin`, assembles the bundle, ad-hoc signs it.
- [ ] Step 2: `open dist/Ocrouter.app` opens the window and starts the daemon if down.
- [ ] Step 3: `unzip -l`/`ls` shows no keys, no token, no target dir inside the bundle.
- [ ] Step 4: commit.

## Self-review

- Spec §13 coverage: accounts list, EL daily bar, strategy selector, live events, "sync models" action → Tasks 1–3 (sync action may be a button that shells to `ocrouter models sync`).
- Review focus: (a) GUI must not leak the token into the UI, (b) closing the window must not kill the daemon, (c) the daemon must not be spawned twice, (d) a daemon that dies must show disconnected rather than freezing.
