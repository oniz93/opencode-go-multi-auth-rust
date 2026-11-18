# SDD ledger — plan: docs/superpowers/plans/2026-11-18-ocrouter-daemon.md

Spec: docs/superpowers/specs/2026-11-18-ocrouter-rust-design.md
Branch: feat/ocrouter-rust · Worktree: .local/share/opencode/worktree/8bb74c/feat/ocrouter-rust
Setup: executing-plans (inline), TDD loaded, spec read.

## Pre-flight scan

- Task 3 ← Task 4: keys.rs consumes `Provider`, which Task 4 defines later.
  → Ruling: execute Task 4 (provider.rs) before Task 3 (keys.rs). Cost if wrong: one reorder.
- Task 2 ↔ Task 5: Task 2's `Config` needs `RoutingStrategy`; Task 5 defines it.
  → Ruling: define `RoutingStrategy` in `config.rs` (Task 2); Task 5 re-exports it from `router::types` so both paths work. Cost if wrong: a moved definition.
- Task 11 ← Task 12: `state.rs` consumes `EventRecord`, defined in Task 12.
  → Ruling: execute Task 12 (events) before Task 11 (state). Cost if wrong: one reorder.
- Task 5, `ApiKey.id` unspecified.
  → Ruling: `id = "<provider>:<label>"`; stable, human-readable, unique within a key file. Cost if wrong: a rename at most.
- Tasks 16/19/21: test helper names (`run`, `run_capture`, `run_once`, `request_for`, `FakeUpstream`, `test_app`, `call`, `bearer`) appear in tests but not in Interfaces.
  → Ruling: define each helper inside its own task's test module at implementation time. Cost if wrong: local rename.
- Task 13 `Lane` consumed by Tasks 14/16/18; Task 14 `Catalog` consumed by 15/18/19. No conflict.

Execution order: 1, 2, 4, 3, 5, 6, 7, 8, 9, 10, 12, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22.

## Progress

Task 1: complete (commits 35ea7e2..2525edc, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 1: Ruling: Homebrew rustc 1.98 is x86_64-host with no aarch64 std and shadows rustup; all builds use ~/.cargo/bin/cargo (rustup, aarch64 host 1.91.1). .cargo/config.toml keeps the aarch64 target pin. — wrong target = Rosetta binary, caught by `file`.
Task 2: complete (commits 2525edc..5b8dd0e, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 4: Ruling: `#[serde(rename_all="kebab-case")]` renders OpenCodeGo as `open-code-go`, not the spec's `opencode-go`; use explicit `#[serde(rename=...)]` per variant. — cost if wrong: key-file provider tags would not round-trip.
Task 4: complete (commits 5b8dd0e..761532f, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 3: complete (commits 761532f..8568589, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 5: complete (commits 8568589..60660f2, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 6: complete (commits 60660f2..095289d, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 7: complete (commits 095289d..f7ad00c, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 8: Ruling: plan listed `refine_cooldowns` with no defined behavior and no test; omitted (YAGNI). — cost if wrong: one small method added later.
Task 8: complete (commits f7ad00c..b1e4fb2, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 9: complete (commits b1e4fb2..b6f2b12, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 10: complete (commits b6f2b12..94a7be1, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 12: complete (commits 94a7be1..f3ddde2, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 11: complete (commits f3ddde2..e2e3ebb, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 13: complete (commits e2e3ebb..117b5d4, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 13: Rulings: (a) `normalise()` and `fetch_opencode_snippet_models()` from the plan's Interfaces have no test and are covered by parse_model_list/par_el_catalog + fetch_models — omitted (YAGNI); (b) reqwest 0.13 renamed the TLS feature: `rustls-tls` → `rustls`. — cost if wrong: one thin wrapper added later.
Task 14: complete (commits 117b5d4..6123b3c, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 15: complete (commits 6123b3c..4d939e2, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 16: Ruling: `build_upstream_url` and `strip_lane_prefix` live in `ocrouter-core::proxy::routing`, not in the cli binary — binary-crate unit tests are clumsy and the helpers are needed by core routing too. The hyper server itself stays in `crates/cli/src/proxy_server.rs`. — cost if wrong: a move.
Task 16: complete (commits 4d939e2..7aae2ca, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 17: complete (commits 7aae2ca..e6de91b, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 18: Ruling: the plan's positional `classify_response(status, headers, body_peek, key_id, now_ms, cb, km, allowance, org, provider)` gained a `ResponseSignal` struct plus two fields the plan's tests need: `fallback_cooldown_ms` and `count_breaker_failure` (the latter is Review-Focus item 1 — a repeated same-body 5xx must not open a breaker). `LaneChoice` carries a `Console` variant to match `Lane`. — cost if wrong: a signature change in Task 19 only.
Task 18: complete (commits e6de91b..dd5edb9, tests: cargo test -p ocrouter-core → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 19: Ruling: the request pipeline lives in `crates/cli/src/engine.rs` behind an `UpstreamClient` trait, and `ocrouter-cli` becomes lib+bin so an integration test can drive it. `KeyManager` gains `select_for(provider, ...)` (additive). The real reqwest client is `http_client.rs`. — cost if wrong: module moves.
Task 19: complete (commits dd5edb9..3390dc7, tests: cargo test → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 20: complete (commits 3390dc7..23fb022, tests: cargo test → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
- Task 21: code complete (commits 23fb022..HEAD, tests: cargo test → 117 pass). Step 5 (live verification against real Go/EL upstreams) is BLOCKED on the owner's API keys; it is not claimed as verified.
- Task 21: Ruling: `run()` returns `RunOutcome::{Text,Serve}` and `serve()` lives in main.rs; `Engine` state moved to a shared `Shared` struct (Arc<Mutex<…>>) so the control API and proxy observe the same keys and strategy. — cost if wrong: a small refactor.

## Final review (fresh reviewer, whole branch)

- Final: fixed Critical streamed-lane usage — paid-lane SSE tokens were added to the EL bucket; now `tee_org` is `None` unless the provider is Experiential. Test `streamed_go_usage_does_not_touch_the_el_budget` RED→GREEN (verified by reverting the fix), plus `streamed_el_usage_is_recorded_against_the_org`.
- Final: fixed Important daily rollover — `reset_if_new_utc_day` had no production caller, so an exhausted EL allowance stayed dead until restart. Test `an_allowance_exhausted_yesterday_is_usable_after_utc_midnight` RED→GREEN.
- Final: fixed Important unified models route — `GET /v1/models` returned 502. Test `the_unified_models_route_serves_the_merged_catalog` RED→GREEN.
- Final: fixed Important whoami fragility — a failed org lookup disabled the free lane; `ensure_el_org(None)` now falls back to a stable local bucket and logs a warning. Test `an_unresolved_org_falls_back_instead_of_disabling_el`.
- Final: fixed Important `--secret -` — reads one line from stdin so the credential stays out of history/`ps`. Test `secret_dash_reads_one_line_from_stdin`.
- Final: fixed Important `el_allow_paid` — was documented but unread; now bypasses the local cap in `choose_lane`. Test `allow_paid_keeps_using_el_after_the_free_allowance_is_spent`.
- Final: Ruling: `state.json` persistence is NOT wired for v1. Rather than leave a false promise, the README claim was removed and the limitation documented. The reactive 429 keeps the no-credit-spend guarantee. Cost if wrong: a restart re-learns cooldowns and the EL counter.
- Final: minor (deferred): `ensure_go_session`/`PASSTHROUGH` are dead code exercised only by tests.
- Final: minor (deferred): a 401/403 resets a key's breaker via `record_success`.
- Final: minor (deferred): control token compared with `==`, not constant time.
- Final: minor (deferred): `prepare_body` reserializes the whole JSON rather than one field.
- Final: minor (deferred): `x-opencode-session` is also added to EL/Console.
- Final: minor (deferred): a fresh `ReqwestClient` (no shared pool) per connection.
- Final: minor (deferred): `POST /api/keys/{id}/test` reports presence without probing.
- Final: minor (deferred): empty `[workspace.dependencies]`; `rust-version` 1.91.
- Final: minor (deferred): 502/503 bodies carry no `content-type`.
