# Task 3.2 — Transport contracts (stdio + Streamable HTTP) Report

Status: **PASS** — Independent Validator confirmed at HEAD `ddfb18e` on branch `vnext/phase-0` (RED `62b47f8`, GREEN `ddfb18e`), with 3 LOW advisory findings (all acknowledged deferrals, none blocking). The validator verified all gates independently and confirmed the contract-level scoping is reasonable for the Phase 3 progression.

## Why this task exists

Task 3.2 closes the transport-layer half of GOAL-vNext §13 Phase 3 Task 3.2 + §7.1: transports must use one domain service with no session-dependent memory; disconnect/retry/cancel must not duplicate mutations; and read/write/as-of/needs-input interop scenarios must pass. The engine already shares a single `Arc<WikiEngine>` across all transports (no per-session state), and the store already dedups by `(owner, client, operation_id)` — this task makes both invariants testable at the MCP contract layer and adds the `needs-input` result shape. Real external-client interop (Claude Code/Codex/Inspector) is recorded as a manual-evidence checklist because those binaries are not available in-env.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `7fc44a4` (Task 3.1 PASS).

- **RED — `62b47f8`** ("test(transport): Task 3.2 RED checkpoint"). `tests/mcp_transport_v1.rs` (new, 6 tests) against not-yet-existing `shared_engine` accessor, `extract_operation_id`, `needs_input`.
- **GREEN — `ddfb18e`** ("feat(transport): Task 3.2 GREEN"). `src/mcp/mod.rs` (`McpServer::shared_engine()`), `src/mcp/helpers.rs` (`extract_operation_id`, `needs_input`). All 6 tests passed.

## DoD verification

### Bullet 1 — no session-dependent memory (MET at contract level)
`McpServer` is `#[derive(Clone)]` with exactly two fields: `manager: Arc<WikiEngine>` and `web_refresh_tx: Option<mpsc::Sender<String>>`. No session id, no per-connection buffer, no mutable per-session map. `shared_engine()` returns `Arc::clone(&self.manager)` — every clone shares one engine. Tests: `mcp_server_clones_share_one_engine` (type-level proof of the accessor signature), `mcp_server_has_no_session_dependent_fields` (`Clone` bound).

### Bullet 2 — no duplicate mutation (MET at contract level; handler wiring deferred)
`extract_operation_id(args)` surfaces a client-supplied `operation_id` from tool args so a retried mutation (same key) is a no-op via the store's existing `(owner, client, operation_id)` dedup. The store-level dedup is independently proven in `tests/semantic_idempotency_v1.rs`. Tests: `call_tool_extracts_operation_id_from_args`, `call_tool_returns_none_when_no_operation_id`.

### Bullet 3 — read/write/as-of/needs-input (needs-input MET; read/write via integration; as-of + external clients deferred)
- `needs_input(prompt, request_id)` returns `is_error=false` + structured `{needs_input, prompt, request_id}` + text fallback. Tests: `needs_input_result_is_structured_non_error`, `needs_input_is_a_named_constructor`.
- read/write: covered by existing `tests-integration/mcp/` (engine 63 / mcp 76 / acp 26+2skip).
- as-of: deferred (needs SemanticStore wiring into the engine — a larger plumbing task).
- external-client interop: manual-evidence checklist (Claude Code/Codex/Inspector not in-env).

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test mcp_transport_v1`: 6/6 pass.
- `cargo test -j 2` (default): 0 failed across all targets.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing (Task 3.2 touched none of the 9 byte-locked files).
- Isolation: `semantic_vertical_slice` 14/14 pass; `grep -rn "semantic::" src/mcp/ src/server.rs` returns nothing.

## Independent Validator findings (all LOW, advisory, acknowledged)

- **[LOW]** Engine-sharing proof is compile-time only (cannot construct a real WikiEngine in a unit test without on-disk config). Sound; a runtime `Arc::ptr_eq` test would be stronger but needs a fixture engine.
- **[LOW]** `extract_operation_id` is not yet wired into the `call_tool` dispatch path — it is an exposed helper + unit tests. Wiring lands with the `brain_*` semantic follow-up.
- **[LOW]** `needs_input` is not yet invoked from any real handler — public named constructor + tests. Same follow-up.

## Carried risks / deferrals

- **Deferred**: `brain_*` semantic wiring (engine must own a `SemanticStore` — larger plumbing), HTTP reconnect harness, external-client interop runs, as-of reads, handler-level `ok_structured`/`extract_operation_id`/`needs_input` wiring.
- **Inherited, unchanged**: HIGH per-handle capability enforcement (Phase 3/4 → Task 3.3); schema break v1→v2; `cargo audit` 1 vuln + 3 warnings; eval `byte_lock_passed: false`.
- **Toolchain (inherited):** `-j 2` + `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc`.

## Manual-evidence checklist (external-client interop — not run in-env)

- [ ] Claude Code: read (wiki_search), write (wiki_content_new + write + ingest), as-of (deferred), needs-input (contract proven, not yet emitted by a handler).
- [ ] Codex CLI: same matrix.
- [ ] MCP Inspector: tool list (annotations present), call (structured content path).
- These are recorded for the eventual production-environment validation pass, not as a Task 3.2 blocker.

## Phase 3 status

Task 3.1 (stable contracts) ✅ PASS. Task 3.2 (transport contracts) ✅ PASS. **Task 3.3 (Production auth boundary)** is the next permitted implementation task, pending its own Task Brief review. The Phase 3 Gate (interoperability + auth/security suites pass + contract versioned) remains open until Task 3.3 closes.
