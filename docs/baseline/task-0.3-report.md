# Task 0.3 — Semantic Vertical Slice Report

Status: **ROUND 2 HARDENING FIXED — awaiting independent revalidation**

Outcome: **EXTEND**

## Round 2 — evidence guard and recovery/mutation queuing

RED checkpoint commit: `876838f` added two tests against clean HEAD `1502b0a` (this task's prior GREEN commit) covering gaps not exercised by round 1:

- `empty_or_invalid_utf8_capture_cannot_become_a_confirmed_evidence_claim`: an empty or invalid-UTF-8 captured rendition must not become a confirmed evidence-linked claim; `propose` now rejects it with `InvalidClaim` before an object is published, and the downstream `confirm` call correctly fails with `MissingDependency` since no proposal exists. A valid Thai-language UTF-8 rendition still produces the correct `byte_start`/`byte_end`/`quote_hash`/`object_id` evidence span and confirms normally.
- `recovery_queued_during_committed_mutation_does_not_deadlock`: a mutation paused after its database commit (via a new `#[cfg(feature = "semantic-test-failpoints")]` test-only pause hook) must not deadlock a concurrently issued `recover()` call; recovery blocks on the maintenance write lock until the paused mutation releases, then both converge with one event, one operation, zero pending outbox rows, and matching ledger/projection checksums.

RED command: `cargo +1.95-x86_64-pc-windows-msvc test --features semantic-test-failpoints --test semantic_vertical_slice -j 1`; RED failure was legitimate `E0599` (`pause_after_commit_for_test`/`recovery_blocked_for_test` not found on `SemanticStore`), not a syntax/config error.

GREEN implementation added to `src/semantic.rs` only:

- Evidence-span guard: `propose` rejects an empty or non-UTF-8 captured rendition with `InvalidClaim` before any proposal object is published.
- Test-only pause primitive (`SemanticTestPause`/`PauseState`, `pause_after_commit_for_test`, `recovery_blocked_for_test`) gated behind `semantic-test-failpoints`, using a `parking_lot::Condvar` to park a mutation after its commit and signal test observers.
- `recover()` now increments/decrements a `recovery_waiting` counter around acquiring the maintenance write lock so tests can observe queuing without a busy-wait on internal state.
- `project_and_ack` switched its maintenance guard from `read()` to `read_recursive()`: the mutation path already holds a maintenance read guard when it calls `project_and_ack`, and parking_lot read locks queue behind a waiting writer by default, so a second plain `read()` on the same thread while `recover()`'s writer is queued would deadlock. `read_recursive()` is safe here because both guards are held by the same call stack on the same thread, never across threads.

All permitted paths remain the same as round 1 (`src/semantic.rs`, `tests/semantic_vertical_slice.rs`, `docs/baseline/task-0.3-report.md`); no legacy Markdown/Git/MCP/ACP/HTTP/Tantivy/Petgraph runtime files were touched.

### Round 2 verification results

| Gate | Result |
|---|---|
| Focused semantic suite (`--features semantic-test-failpoints --test semantic_vertical_slice`) | PASS — 18 passed, including both new tests |
| Rust format (`cargo fmt --check`) | PASS |
| Rust clippy all targets/features (`-D warnings`) | PASS |
| Rust all targets/features (`cargo test --all-targets --all-features`) | PASS — 583 passed, 0 failed, 0 ignored (581 round-1 baseline + 2 new) |
| Semantic per-file coverage (`cargo-llvm-cov 0.8.6`) | PASS — 88.58% lines (1,148/1,296 instrumented lines), minimum 80% |
| Locked eval (`evals/v1`, pinned `uv run --python 3.14.4`) | PASS — 126/126 cases, `environment_passed=true`, `thresholds_passed=true`, zero threshold failures |
| Python governance (`tests-integration/governance`) | PASS — 10/10 |
| Python engine (`tests-integration/engine`) | PASS — 63/63 |
| Python MCP (`tests-integration/mcp`) | PASS — 76/76 |
| Python ACP (`tests-integration/acp`) | PASS WITH KNOWN SKIPS — 26 passed, 2 skipped |
| Dependency audit comparison (`scripts/compare_cargo_audit.ps1` vs `task-0.3-audit-after.json`) | PASS — before 4, after 4, zero new findings |

Known carried risk observed during this round: running the `engine`/`spaces` Python integration suite writes generated default schema files (`procedure.json`, `profile.json`, `semantic.json`) into the tracked `tests/fixtures/wikis/alt-root/schemas/` fixture directory as an untracked side effect. This is the same "test fixture pollution" risk already carried from round 1; it was left untouched and not committed, since cleaning or gitignoring it is outside Task 0.3's permitted paths.

## Scope and TDD evidence

The task started from clean canonical HEAD `9521f87e88fd57a2f1a254cd6474e883426814d5` on `vnext/phase-0`.

- Audit-before was captured before dependency edits: 444 locked dependencies and four existing findings.
- RED checkpoint commit: `9218c98fa77051e2a6a0625ca9c869d0d8063f54`.
- RED command: `cargo test --features semantic-test-failpoints --test semantic_vertical_slice -j 1` through the pinned MSVC 1.95 toolchain.
- Legitimate RED: Rust `E0432`, `could not find semantic in llm_wiki`. There was no syntax/config failure.
- The repository-pinned GNU host first failed on missing `dlltool.exe`; that environment error was not counted as RED. MSVC produced the required contract failure.

Only Task 0.3 permitted paths changed. Legacy Markdown/Git/MCP/ACP/HTTP/Tantivy/Petgraph runtime files were not modified.

## Delivered behavior

- Disabled-by-default `SemanticConfig` fails before any filesystem access.
- Isolated `SemanticApplicationCore` implements capture, propose, and confirm over a SQLite event ledger.
- UUIDv7 owner/event/actor/client/source/rendition/evidence/proposal/claim identities are store-generated. Deserialized capture/propose/claim-draft/confirm commands use `deny_unknown_fields`; executable tests reject identity, status, provenance, supersede, and retract injection.
- Proposal objects carry tagged evidence provenance with source/rendition/evidence IDs, immutable object ID, byte span, and quote hash. Confirmation objects persist the ADR claim kind/status/domain/confidence/privacy/bitemporal/provenance/supersede/retract fields and bind the claim to its recorded event ID/sequence.
- Privacy remains `local_only` until a future policy event authorizes release. Caller attempts to elevate it fail closed before a proposal event is written, and tampered/evidence-less proposal objects cannot be confirmed.
- RFC 8785 canonical request/event hashing includes an RFC number/string vector and fixed event-chain hash `47750a496d582f4c10c374ff0e35552250bc63b9b81ab2623e30ab96ea4ae584`.
- Every event validates against the locked `additionalProperties=false` schema and exposes content only as an object reference.
- SQLite uses WAL, `synchronous=FULL`, foreign keys, 5-second busy timeout, and `BEGIN IMMEDIATE`. Event, outbox, operation request hash, and stored outcome commit atomically.
- Content is SHA-256 addressed, flushed to staging, and published while the write transaction is held. Recovery removes staging and zero-reference objects without deleting shared referenced objects.
- Projection output plus head/checksum is published by atomic replacement. On Windows this uses `MoveFileExW` with replace-existing and write-through flags. Outbox acknowledgement is a later transaction, making replay effectively once.
- `claim_at(ledger_head, world_time)` applies authoritative transaction sequence and half-open `[valid_from,valid_to)` world time. Tests cover future/boundary/as-of behavior and clock rollback.
- A process-global root coordinator separates shared normal maintenance, store-open/ledger validation and SQLite writer arbitration, serialized projection publication, exclusive recovery, and non-blocking capability rollback.
- SQLite BUSY/LOCKED/PROTOCOL contention is retried only at the complete idempotent transaction boundary with bounded backoff/deadline. Semantic conflicts are never retried.

## Crash, concurrency, and rollback evidence

Abrupt child processes cover all nine required failpoints:

1. after idempotency reservation;
2. after object temp flush;
3. after object rename;
4. after event insert;
5. after outbox insert;
6. after stored response;
7. after database commit before projection;
8. after projection temp flush;
9. after snapshot rename before outbox acknowledgement.

Every pre-commit boundary reopens with zero partial event/outbox/operation state. Every post-commit boundary replays the byte-identical stored response and converges to one event, one operation, zero pending outbox rows, and matching ledger/projection checksums.

Concurrency uses independent handles/connections: 20 unique simultaneous writes, same-operation/same-hash, and same-operation/different-hash. Independent validation reproduced SQLite `locking protocol` despite the first transaction-boundary retry. The final fix coordinates store open/ledger validation and WAL's single-writer region per canonical root while retaining bounded full-transaction retry for external contention. The race then passed 30 consecutive stress runs and the complete all-target suite.

Rollback tests reject live handles, an active transaction after its store handle is dropped, missing markers, structurally valid copied markers with wrong UUID/nonce, outside-parent paths, filesystem/repository/`.git` roots, Unix links, and Windows root/interior junctions. The capability accepts no arbitrary path, is not serializable/cloneable, redacts its path/UUID/nonce from `Debug`, and repeated cleanup is safe. A real Git fixture's complete file manifest and HEAD remain identical.

## Verification results

| Gate | Result |
|---|---|
| Focused semantic suite | PASS — 16 passed, including abrupt child processes |
| Independent writer stress | PASS — 30/30 repeated 20-writer runs |
| Per-file semantic coverage | PASS — 87.99% lines (1,084/1,232 instrumented lines), minimum 80% |
| Rust format | PASS |
| Rust clippy all targets/features | PASS with `-D warnings` after final bounded-retry change |
| Rust all targets/features | PASS — 581 passed, 0 failed, 0 ignored |
| Python governance | PASS — 10 |
| Python engine | PASS — 63 |
| Python MCP | PASS — 76 |
| Python ACP | PASS WITH KNOWN SKIPS — 26 passed, 2 skipped |
| Locked Task 0.2 eval | PASS — 126/126 and all thresholds/environment checks |
| Dependency audit comparison | PASS — before 4, after 4, zero new findings |
| CodeGraph coupling | PASS — refreshed 140 files/2,401 nodes/6,754 edges; only semantic entry points call private writer |

Pinned coverage tool: `cargo-llvm-cov 0.8.6`, installed with `--version 0.8.6 --locked` through the MSVC toolchain after GNU installation failed on missing `dlltool.exe`.

## Dependency and security result

New direct dependencies are `rusqlite 0.40.1` with bundled SQLite and `serde_jcs 0.2.0`; UUID enables v7/serde and Windows directly declares the already-locked `windows-sys 0.61.2` API needed for atomic replacement. Lockfile dependency count changed from 444 to 453.

Normalized `cargo audit 0.22.2` findings remain exactly four:

- vulnerable `crossbeam-epoch 0.9.18` / RUSTSEC-2026-0204;
- unsound `anyhow 1.0.102` / RUSTSEC-2026-0190;
- unsound `memmap2 0.9.10` / RUSTSEC-2026-0186;
- unmaintained `bincode 2.0.1` / RUSTSEC-2025-0141.

No finding is suppressed. These remain Phase 0/1 remediation gates. Other carried risks are mutable CI action tags, ignored integration `uv.lock`, Windows pagefile sensitivity, test fixture pollution, and unauthenticated HTTP exposure. Task 0.3 adds no remote endpoint, credential, provider call, or production-data access.

## Architecture decision

`EXTEND` is selected. Common gates pass, executable tests and refreshed CodeGraph prove same-crate semantic isolation, and `SemanticApplicationCore` supplies the future routing seam without dual writing. See `docs/architecture/task-0.3-coupling-decision.md`.
