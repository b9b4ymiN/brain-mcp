# Task 1.1 — Immutable Object Store + Append-only Events Report

Status: **BUILDER GREEN — awaiting independent validation**

Approach: **Promote the Task 0.3 semantic spike** (user-approved). The isolated `SemanticStore` that passed Task 0.3 validation is extended in place rather than rewritten; every Task 0.3 behavior and test is carried forward unchanged.

## Scope and TDD evidence

The task started from clean canonical HEAD `6f38b3c` on `vnext/phase-0`.

- RED checkpoint commit: `3624763` added `tests/semantic_store_v1.rs` (11 tests). RED failure was 19 legitimate `E0599` contract errors (`register_client`, `with_max_object_bytes`, `forge_context_for_test`, `backup_consistent`, `InvalidCapture` all missing); no syntax/config failure.
- GREEN commit: `8ac84e0` changed only `src/semantic.rs` (plus one clippy-driven import gate in the test file).
- User decisions recorded before implementation: multi-client via **client registry** (option C over relaxed validation or deferral), and `max_object_bytes` default **32 MiB**.

Permitted paths: `src/semantic.rs`, `tests/semantic_store_v1.rs`, `docs/baseline/task-1.1-report.md`. No legacy Markdown/Git/MCP/ACP/HTTP/Tantivy/Petgraph runtime file was touched.

## Delivered behavior

- **Client registry.** A `clients` table (server-generated UUIDv7 `client_id`, unique `label`, `created_at`) is created at store initialization and seeded with the bootstrap marker client under the reserved label `__bootstrap__`. `register_client(label)` mints a new identity on first registration and returns the same identity for the same label thereafter — including across process restarts — so a client's idempotency scope is stable. Labels are validated (1–64 bytes of `[a-z0-9._-]`, no `__` prefix).
- **Fail-closed registration check.** `validate_context` still requires marker-bound `store_uuid`/`owner_id`/`actor_id`, but client identity is now verified *inside* every mutation transaction against the `clients` table. A forged context with an unregistered UUID (constructible only through the test-only `forge_context_for_test` hook) is rejected with zero state change. Contexts still cannot cross stores in either direction.
- **Cross-client idempotency isolation.** `(owner_id, client_id, operation_id)` is the uniqueness key in practice, not just in schema: two registered clients may reuse the same `operation_id` with different payloads without conflict, replay returns each client's own stored outcome byte-identically, and same-client/different-payload still returns `IDEMPOTENCY_CONFLICT`.
- **Capture limits.** `SemanticConfig::with_max_object_bytes` (default 32 MiB; zero rejected at create/open) and a lowercase `type/subtype` media-type essence check run before hashing or staging, so an oversized or malformed capture leaves no operation row, event, staging file, or object.
- **Dedup made explicit.** Identical bytes captured under different operations produce two events referencing one content-addressed object and exactly one file on disk.
- **Consistent backup.** `backup_consistent(target)` snapshots the store into a fresh sibling root under the same allowed parent while holding the exclusive maintenance lock (no mutation, projection, or recovery can interleave). The SQLite database is copied with `VACUUM INTO` (then restored to WAL journal mode), followed by marker, projection snapshot, and object files. The backup reopens as a valid store with identical event/operation counts and ledger/projection checksums, and later writes to the source do not leak into it. Targets that already exist, escape the allowed parent, or lack an accessible parent are rejected.

## DoD mapping

| DoD item | Evidence |
|---|---|
| content hash/dedup | `identical_bytes_deduplicate_to_one_object` (new); content addressing carried from Task 0.3 |
| size/MIME limits | `capture_rejects_oversized_bytes_fail_closed`, `capture_default_limit_is_32_mib`, `capture_rejects_malformed_media_types` (new) |
| provenance ทำงาน | Carried Task 0.3: evidence-span provenance suite incl. UTF-8 guard round 2 |
| quarantine ทำงาน | Carried Task 0.3: staging is invisible until commit; abrupt-crash matrix proves no partial object is readable and recovery removes staging |
| event มี actor/client/operation/schema version | Carried Task 0.3 schema-validation test; `registered_clients_have_stable_server_generated_identity` additionally proves per-client `client_id` stamping (new) |
| uniqueness constraints | `(owner_id,client_id,operation_id)` primary key exercised across clients (new) |
| idempotency: same-key/same-payload | Replay assertions in `same_operation_id_is_isolated_per_client_and_conflicts_within_client` (new) + carried Task 0.3 |
| idempotency: same-key/different-tool-or-payload | `IDEMPOTENCY_CONFLICT` assertion (new) + carried Task 0.3 request-hash tests |
| idempotency: cross-client/owner isolation | `same_operation_id_is_isolated_per_client...`, `contexts_do_not_cross_stores_in_either_direction`, `forged_unregistered_client_is_rejected_fail_closed` (new) |
| append/replay tests | Carried Task 0.3 ledger/projection checksum + crash-recovery suites; re-run green |
| backup tests | `consistent_backup_restores_identical_state_and_stays_isolated`, `backup_rejects_existing_or_outside_targets` (new) |
| orphan staging/object cleanup ปลอดภัย | Carried Task 0.3: `recovery_preserves_shared_objects_and_is_checksum_idempotent` + failpoint matrix; re-run green |
| mutation ไม่มี in-place history edit | Carried Task 0.3: append-only schema + hash chain; no write path modifies an existing event row |

## Verification results

| Gate | Result |
|---|---|
| New suite (`--test semantic_store_v1`) | PASS — 11 passed |
| Task 0.3 suite (`--test semantic_vertical_slice`) | PASS — 18 passed, no regression |
| Rust format | PASS |
| Rust clippy all targets/features (`-D warnings`) | PASS |
| Rust clippy all targets, default features (`-D warnings`) | PASS (after gating a test-only `Uuid` import) |
| Rust all targets/features | PASS — 594 passed, 0 failed (583 baseline + 11 new) |
| Semantic per-file coverage (`cargo-llvm-cov 0.8.6`, both semantic suites) | PASS — 88.75% lines (1,333/1,502), minimum 80% |
| Locked eval (pinned `uv run --python 3.14.4`) | PASS — 126/126, `environment_passed=true`, `thresholds_passed=true` |
| Python governance / engine / mcp / acp | PASS — 10 / 63 / 76 / 26+2 known skips |
| Dependency audit comparison vs `task-0.3-audit-after.json` | PASS — before 4, after 4, zero new findings |

## Notes and carried risks

- **Schema change before production.** `initialize_schema` now creates the `clients` table and seeds the bootstrap client. No production store exists yet (every prior store is a test fixture), so no migration path is provided; a store created before this commit cannot be opened by code that queries `clients`. This is recorded as an accepted pre-production break.
- **Backup shares `store_uuid` by design.** A backup is the same trust domain as its source, so contexts minted by the source validate against the backup. Cross-host/clean-host restore, encryption, and purge-registry synchronization are Phase 6 (Task 6.3) scope.
- **Python-suite binary freshness.** Earlier gate runs (including Task 0.3 round 2) invoked `tests-integration` with `LLM_WIKI_BIN=../target/debug/llm-wiki`, which resolved to a stale July 13 binary because this environment sets `CARGO_TARGET_DIR` elsewhere. Results were unaffected — the semantic module is not wired into the CLI — but this task's Python gates were re-run against the freshly built MSVC binary in `CARGO_TARGET_DIR`, and future runs should do the same.
- **Toolchain.** The repository-pinned bare `cargo` resolves to the GNU host and fails on missing `dlltool.exe`; all gates run through `cargo +1.95-x86_64-pc-windows-msvc`, consistent with Tasks 0.1–0.3.
- **MIME strictness.** Media types are accepted only as lowercase essences; uppercase input is rejected rather than normalized. Client-side normalization is deferred to the transport layer (Phase 3).
- Existing carried risks (4 RUSTSEC findings, CI action tags, fixture pollution, Windows pagefile sensitivity) remain unchanged and unsuppressed.
