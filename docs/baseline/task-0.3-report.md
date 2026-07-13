# Task 0.3 — Semantic Vertical Slice Report

Status: **VALIDATOR FEEDBACK FIXED — awaiting independent revalidation**

Outcome: **EXTEND**

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
