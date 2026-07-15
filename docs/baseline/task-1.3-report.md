# Task 1.3 — Policy and Concurrency (incl. Full Hard-Purge Saga) Report

Status: **PASS** — Independent Validator confirmed at HEAD `a134700` (RED/GREEN across sub-slices A `41d478d`, B `6955bce`, C `5039a62`, fix round `a134700`), after a first validator pass returned 2 MEDIUM findings against `5039a62` and a parallel security-reviewer pass across the whole Phase 1 surface returned one HIGH (correctly scoped out to Phase 3/4, not blocking) and several MEDIUM/LOW/INFO findings. All in-scope findings were closed in `a134700`; the validator independently re-verified every gate and both fixes by reading code and executing the new tests directly, and returned a final **PASS** with zero remaining findings at any severity. Clear to close Task 1.3 and proceed to the Phase 1 Gate.

## Why this task exists

Task 1.3 closes the remaining GOAL-vNext §13 DoD items for Phase 1: scoped latest-user-wins policy semantics (explicit user save, latest scoped correction ordered by server `event_seq`, scope-key validation, trusted-actor authentication) and, per ADR Decision 7, the full deny-first idempotent hard-purge saga — ledger request, registry replication, key revocation, live/object deletion, projection cleanup (the ADR's "Git rewrite / index rebuild" step, realized in Phase 1 as an absence-proof verifier since no projections are wired until Phase 2 Task 2.1), retention invalidation, and completion, each with crash/retry coverage at every step boundary. This task builds directly on Task 1.2b's per-object DEK / per-owner epoch-KEK encryption primitives (`destroy_wrapped_key`, `rotate_epoch_and_rewrap`), which exist specifically so `key_revoked` has a real key to revoke rather than a faked no-op. Closing this task also completes the Phase 1 Gate.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `f3a5565` — a tests-only commit (idempotency/concurrency gaps for reject/retract/supersede/propose_inference), passed unmodified, retroactively mapped into this task per the Task Brief alongside the pre-existing client-capability-model commits (`c994527` RED + `a385702` GREEN: `register_client_scoped`, the shared confirm/reject/retract/supersede choke point, `SemanticError::CapabilityDenied`).

Three sub-slices deliver the remaining scope:

- **Sub-slice A — `41d478d`** ("test: close Task 1.3 policy coverage gaps and gate crash tests by feature"). `tests/semantic_policy_v1.rs` (new, 382 lines, 4 tests) + `tests/semantic_vertical_slice.rs` (14 lines changed). 2 files changed, 393 insertions(+), 3 deletions(-). No `src/semantic.rs` change.
- **Sub-slice B — `6955bce`** ("feat(semantic): add purge registry primitive with deny-first fail-closed reads"). `src/semantic.rs` (+473/-6) + `tests/semantic_purge_v1.rs` (new, 361 lines, 7 tests). 2 files changed, 828 insertions(+), 6 deletions(-).
- **Sub-slice C — `5039a62`** ("feat(semantic): add full hard-purge saga with crash/retry matrix"). `src/semantic.rs` (+546/-10) + `tests/semantic_purge_v1.rs` (+477, growing the suite from 7 to 17 tests). 2 files changed, 1007 insertions(+), 16 deletions(-).
- **Fix round — `a134700`** ("fix(semantic): scope purge backup invalidation, add TOCTOU race test"). Closes the Independent Validator's 2 MEDIUM findings against `5039a62` plus 1 MEDIUM from a parallel security review. `src/semantic.rs` + `tests/semantic_purge_v1.rs`, 2 files changed, 250 insertions(+), 11 deletions(-). `tests/semantic_purge_v1.rs` grows from 17 to 19 tests.

Permitted paths per the Task Brief: `src/semantic.rs`, `src/lib.rs` (only if a module split were needed — not used), `tests/semantic_policy_v1.rs` (new), `tests/semantic_purge_v1.rs` (new), `tests/semantic_vertical_slice.rs` (cfg-gate two crash tests only), `docs/baseline/task-1.3-report.md`, `brain_2nd/GOAL-vNext.md` (decision history at close). Zero new dependencies were required or added at any point, including the fix round.

## Design

**Sub-slice A** made no implementation change: its four tests prove existing, already-correct behavior was previously untested — "a coverage gap, not a behavior gap," matching the pattern already established by `f3a5565`. It also fixed a pre-existing default-features test-suite defect (see "Bug found and fixed" below).

**Sub-slice B — PurgeRegistry primitive.** An append-only, hash-chained purge-registry journal (`purge_registry_entries` + a `purge_denied_ids` projection) is replicated to configured independent target directories with quorum acknowledgement (ADR Decision 7). `append_registry_denial` is deliberately a raw primitive — not capability-gated, not idempotent-by-operation-id, not nonce-bound — mirroring the Task 1.2b precedent where `destroy_wrapped_key`/`rotate_epoch_and_rewrap` were validated standalone before the saga (sub-slice C) wired guarantees around them. Deny-first semantics: the local commit protects this store's own reads immediately and is never rolled back by a replication shortfall (`append_registry_denial` returns `RegistryQuorumFailed` on shortfall, but the denial itself stands). The deny gate lives inside `decrypt_object`, which checks `purge_denied_ids` and a store-wide `registry_sealed` meta flag in the **same connection/transaction** as the plaintext materialization, closing a TOCTOU window where a concurrent `registry_denied` commit could otherwise land between "not denied" and "here is the plaintext." `read_object`, `claim_at`, and `claims_current` — previously autocommit, multi-statement — are restructured to run inside one explicit read transaction each, so every caller (including `object_json`) inherits fail-closed behavior for free. `SemanticStore::open` evaluates the registry against configured replication targets while still holding the writer lock and seals the store (`registry_sealed='true'`, all plaintext reads return `RegistrySealed`) if a reachable quorum is unavailable or reports an epoch/hash this copy hasn't applied. `sync_purge_registry` re-checks, fetches and hash-chain-verifies missing entries from a quorum of targets, and unseals only once the local copy is provably current.

**Sub-slice C — hard-purge saga.** `purge_preview(...)` returns a `PurgePreview{preview_hash, nonce}`; `purge_execute`/`purge_resume` step the saga through `requested → registry_denied → key_revoked → live_deleted → projections_cleaned → retention_pending → completed`, persisted in state so it is resumable. Authorization requires the `purge` capability (a propose-only worker is rejected with `CapabilityDenied`); the nonce is single-use, bound to `preview_hash`, and expires after 60 seconds via the injected `SemanticClock`. Idempotency is bespoke to the saga (not inherited from `mutate_once`): the same `(client_id, operation_id)` with the same `preview_hash` resumes/replays the stored receipt without re-checking the nonce; the same `operation_id` with a different `preview_hash` is an explicit `IDEMPOTENCY_CONFLICT` with zero side effects. Step design: `key_revoked` calls the Task 1.2b primitives directly; `live_deleted` removes the live ciphertext file; `projections_cleaned` — the realization of the ADR's `git_rewritten/index_rebuilt` step — has no wired projections yet (Tantivy/Petgraph/Markdown land in Phase 2 Task 2.1) but still performs a real absence-proof verifier that independently re-checks the object file, wrapped key, and registry-denial record are each gone, failing closed on a stray leftover copy rather than a disguised no-op; `retention_pending` deletes every backup this store tracked as undeleted/possibly-still-decrypting (`backup_consistent` now records each backup into a new `purge_backup_sets` table), then creates and independently reopens/verifies one fresh backup taken after key destruction. `append_registry_denial` gained one addition: calling it again with the exact same normalized ID batch as the current head is now a pure replication retry (no new epoch minted), making saga resume after a registry-replication quorum shortfall idempotent instead of burning an epoch on every retry. The crash/retry matrix uses a dedicated `purge_crash_worker` child-process pattern (mirroring `semantic_vertical_slice`'s existing `crash_worker`) that aborts the process via `crash_at()` right after each of the six step transitions commits; the parent reopens and resumes by replaying the same `purge_execute` call.

## What the 23 new tests prove

**Sub-slice A (`tests/semantic_policy_v1.rs`, 4 tests):**
- Explicit user save is attributed to one authenticated `actor_id` across distinct `client_id`s.
- Latest scoped correction (supersede) orders by server `event_seq` even when the injected clock moves backward, not by `recorded_at`.
- Scope key fields (domain/subject/predicate) reject whitespace-only values before any event is appended.
- A forged, unregistered client is rejected fail-closed by confirm/reject/retract/supersede, extending Task 1.1's capture-only coverage of the shared `mutate_once` choke point.

All four pass against the existing implementation unmodified.

**Sub-slice B (`tests/semantic_purge_v1.rs`, 7 tests)** cover: the 2-target replication minimum with quorum acknowledgement; deny-first holding even on a replication quorum shortfall; denial fail-closed via `object_json`; denial fail-closed via `claims_current`; registry epoch monotonicity; seal-at-open when a quorum is unreachable; seal-at-open when the local copy is behind quorum, together with hash-chain rejection of a tampered replicated entry.

**Sub-slice C (`tests/semantic_purge_v1.rs` grows from 7 to 17 tests, 10 new)** cover: the crash/retry matrix — one test per each of the six step-transition boundaries (`requested`, `registry_denied`, `key_revoked`, `live_deleted`, `projections_cleaned`, `retention_pending`/`completed`), each proving the saga recovers after an abrupt child-process abort and resume reaches `completed`, with denial already fail-closed as of `registry_denied` regardless of which later boundary crashed; a dedicated negative test seeding a stray leftover object-file copy between `live_deleted` and `projections_cleaned`, proving the absence-proof step rejects it (`CorruptLedger`) until the leftover is removed, then completes normally; a dedicated `IDEMPOTENCY_CONFLICT` test for the same `operation_id` with a different `preview_hash`, proving zero side effects; an authorization test proving a propose-only worker is rejected with `CapabilityDenied` on `purge_execute`; and a resume/replay test proving the same `(client_id, operation_id)` + same `preview_hash` returns the stored receipt without re-checking the (already-consumed) nonce.

**Fix round (`tests/semantic_purge_v1.rs` grows from 17 to 19 tests, 2 new)** — added in response to the Independent Validator's review, see the dedicated section below: `retention_pending_spares_a_backup_that_cannot_decrypt_the_target` proves an unrelated, pre-dating backup survives a purge while the fresh post-purge backup is still created/verified; `claims_current_uses_one_snapshot_across_all_rows_despite_a_mid_query_denial` proves the promised TOCTOU race property using a new `pause_read_after_deny_check_for_test` hook.

## Independent Validator and security review findings, and the fix round

**Independent Validator, round 1 (against `5039a62`).** Re-ran every gate independently (all clean, matching this report's numbers) and re-confirmed 4 of the 6 prior Task-Brief-review findings as closed by reading code/tests directly, not the commit messages. Raised two new findings against the implementation:

1. **[MEDIUM]** `advance_purge_retention_pending` invalidated (deleted) *every* backup this store had tracked and not yet invalidated, with no check that the backup could actually decrypt a purge target — contradicting both the approved brief ("marks every registered backup that can still decrypt the targets") and ADR Decision 7 ("invalidate backups capable of decryption"). No test exercised an unrelated/pre-dating backup.
2. **[MEDIUM]** The approved brief committed to a concurrent race test proving the `decrypt_object` TOCTOU fix using the Task 0.3 pause-hook pattern; that test was never added, even though the underlying transaction-boundary fix itself was verified correct by static reading.

Also noted, non-blocking: `docs/baseline/task-1.3-report.md` didn't exist yet (this document); and a content-addressed-dedup reference-counting gap inherited from Task 0.3/1.1 (a purge target byte-identical to another still-valid claim's evidence would destroy that evidence too) — not introduced or regressed by this task, carried forward as an informational note.

**Security reviewer, parallel pass (whole Phase 1 surface: `src/semantic.rs`, Tasks 1.1–1.3, plus all eight semantic test files).** No CRITICAL finding, and no HIGH finding reachable in the code as it stands today (SQL injection, path traversal, nonce reuse, and "denied ID still returns plaintext" were specifically hunted for and not found — see full review for the clean-bill list). Findings, most severe first:

- **[HIGH, not currently exploitable]** Capability enforcement is per-method, not per-handle: `register_client_scoped`, `destroy_wrapped_key`, `rotate_epoch_and_rewrap`, `append_registry_denial`, `sync_purge_registry`, `backup_consistent`, `recover`, `purge_resume`, `purge_status` all take no `TrustedContext`/capability parameter, so any code holding a raw `&SemanticStore` (not just a `TrustedContext` value) could call them directly. Not exploitable today because no transport/tool layer in this repo hands a `SemanticStore` reference to any less-trusted component yet; this is exactly Phase 3/4's "production auth boundary" scope (ADR Decision 8), and most of these primitives (`register_client_scoped`, `destroy_wrapped_key`, `rotate_epoch_and_rewrap`, `backup_consistent`, `recover`) predate Task 1.3 (Tasks 1.1/1.2b). **Not fixed in this task** — recorded as a carried risk that must close before Phase 3/4 wiring; see "Notes and carried risks."
- **[MEDIUM, fixed in `a134700`]** A malformed purge target (e.g. a claim ID passed where an object ID was expected) wasn't rejected until `live_deleted`/`projections_cleaned`, by which point `registry_denied`/`key_revoked` had already committed for the *other*, well-formed targets in the same batch — permanently stalling the saga with no way to complete it. Fixed by validating every target's `sha256:<64-hex>` format eagerly in `purge_preview`, before anything irreversible starts.
- **[MEDIUM, disclosed/deferred]** No automatic content-linkage expansion from a claim ID to every object that carries its plaintext (proposal/confirmation/source objects can each hold a copy) — already self-documented as the caller's job in `purge_preview`'s own doc comment; re-flagged here as a tracked, not-yet-closed gap for whenever hard purge is relied on for a real deletion request.
- **[MEDIUM/LOW, disclosed/deferred]** `propose_inference`'s `evidence_capture_operation_ids` and `supersede`'s `superseded_claim_operation_ids` have no length cap (unlike `capture`'s `max_object_bytes`), and `propose_inference` requires no capability at all; an arbitrarily large vector could stall the single writer mutex for its duration. Pre-existing Task 1.2 code, outside Task 1.3's permitted paths — **not fixed here**.
- **[LOW, disclosed/deferred]** `purge_resume`/`purge_status` take a bare `purge_id` with no per-caller ownership/ authorization check, unlike `purge_execute`. Low impact in this single-owner system; **not fixed here**, same Phase 3 scope as the HIGH finding above.
- **[INFO]** Read-path methods (`claim_at`, `claims_current`, `object_json`, etc.) take no capability at all — consistent with ADR Decision 8 assigning `brain.read` enforcement to the not-yet-built transport layer.
- **[INFO]** `claim_at`/`claims_current`/`read_object` don't take `coordinator.maintenance.read()` before opening their own transaction, unlike mutating paths — robustness-only, not a confidentiality issue; the deny/seal check was independently confirmed co-located with plaintext materialization in every traced call site.

**Fix round (`a134700`)** closes both validator MEDIUM findings and the one security MEDIUM judged in-scope and cheap to fix now:

- `advance_purge_retention_pending` now takes the saga's `targets` and opens each tracked backup's own database, invalidating (deleting) it only if its own `wrapped_keys` table has a live row for at least one target. New test: `retention_pending_spares_a_backup_that_cannot_decrypt_the_target` — creates an unrelated backup before the purge target is even captured, asserts it survives the purge while the fresh post-purge backup is still created and independently reopened/verified.
- A new pause hook, `pause_read_after_deny_check_for_test`, is armed inside `decrypt_object` itself, right after its deny/seal check and before it touches ciphertext. Because `decrypt_object` is a free function with no `&self`, the hook lives on the shared `RootCoordinator` (reachable via `coordinator_for(root)`) rather than on `SemanticStore`, mirroring the existing `pause_after_commit` mechanism's `PauseState`/`SemanticTestPause` types. New test: `claims_current_uses_one_snapshot_across_all_rows_despite_a_mid_query_denial` — two independent claims in one scope, a real `append_registry_denial` commit is raced against an in-flight `claims_current` call paused mid-loop, and the whole call is proven to return both claims from one consistent pre-denial snapshot (not a torn mix), with the denial fully durable for the store's next, new call.
- `purge_preview` now validates every target's `sha256:<64-hex>` format via `object_path()` eagerly, closing the security review's malformed-target-stalls-the-saga finding.

## Bug found and fixed

Sub-slice A fixed a pre-existing default-features test-suite defect (not a production-code bug): the two child-process crash tests in `tests/semantic_vertical_slice.rs` — `abrupt_failpoint_matrix_has_atomic_recovery_and_effectively_once_projection` and `recovery_preserves_shared_objects_and_is_checksum_idempotent` — call `crash_at()` failpoints that are a no-op without the `semantic-test-failpoints` feature, so a plain `cargo test` (default features) was failing on this branch, observed 2026-07-15. **Fix**: gated both tests, their `run_crash_child` helper, and the now-conditionally-unused imports/statics behind that feature, matching the existing gate already applied to `recovery_queued_during_committed_mutation_does_not_deadlock`. No assertion was weakened — attribute gating only. Verification: `cargo test --test semantic_vertical_slice` went from 14 passed/2 failed to 14/14 with default features; 18/18 with `--all-features` (unchanged).

## Verification results

| Gate | Result |
|---|---|
| New suite (`--test semantic_policy_v1`, sub-slice A) | PASS — 4 new tests, all pass, no `src/semantic.rs` change |
| `semantic_vertical_slice` default-features regression (sub-slice A fix) | PASS — 14/14 (was 14 passed, 2 failed pre-fix); 18/18 unchanged with `--all-features` |
| Semantic suites after sub-slice A (`--all-features`) | PASS — 64/64 across 7 suites |
| New suite (`--test semantic_purge_v1`, sub-slice B) | PASS — 7 new tests, all pass |
| Semantic suites after sub-slice B (`--all-features`) | PASS — 71/71 across 8 suites |
| `semantic_purge_v1` after sub-slice C | PASS — grew to 17 tests total (13 pass with default features; 4 failpoint-only tests, including the full crash matrix and the absence-proof negative test, require `--all-features`) |
| Semantic suites after sub-slice C (`--all-features`) | PASS — 81/81 across 8 suites |
| `semantic_purge_v1` after fix round (`a134700`) | PASS — grew to 19 tests total (both new tests — backup-invalidation scope, TOCTOU race — pass; the race test requires `--all-features`) |
| Semantic suites after fix round (`--all-features`) | PASS — 83/83 across 8 suites |
| Rust format (`cargo fmt --check`) | PASS — clean at every sub-slice and the fix round |
| Rust clippy (`--all-targets`, default features, `-D warnings`) | PASS — clean at every sub-slice and the fix round |
| Rust clippy (`--all-targets --all-features`, `-D warnings`) | PASS — clean at every sub-slice and the fix round |
| Full-repo regression (`cargo test --all-targets --all-features`) | PASS — 0 failed across every binary (35 test binaries), at every sub-slice and the fix round |
| Dependency audit (`cargo audit` vs `docs/baseline/task-0.3-audit-after.json`) | PASS — 4 findings, identical, zero new findings throughout (no new dependency added at any point) |
| Semantic coverage (`cargo-llvm-cov` on `src/semantic.rs`) | PASS — 89.29% baseline → 89.85% (sub-slice B) → 89.63% (sub-slice C) → 89.74% (fix round, final); minimum 80% throughout |
| Locked eval (`evals/v1`, pinned `uv run --python 3.14.4`, fresh binary in an isolated `CARGO_TARGET_DIR`) | PASS — 126/126 cases, `environment_passed=true`, `thresholds_passed=true`, zero threshold failures |
| Python governance (`tests-integration/governance`) | PASS — 10/10 |
| Python engine (`tests-integration/engine`) | PASS — 63/63 |
| Python mcp (`tests-integration/mcp`) | PASS — 76/76 |
| Python acp (`tests-integration/acp`) | PASS — 26/26, 2 skipped (known: session-cap enforcement) |

Note: the locked eval and Python suites were run once, against the fix-round HEAD (`a134700`), using a fresh `llm-wiki` binary built in an isolated `CARGO_TARGET_DIR` per the established convention (not the stale in-repo `target/`). They were not re-run separately per sub-slice; the Rust-side gates (fmt/clippy/test/audit/coverage) were re-verified at every sub-slice and the fix round.

## Disclosed scope limits and accepted tradeoffs

Per the sub-slice C (`5039a62`) commit message:

- Claim-ID-to-object-ID resolution is the caller's job, matching Task 1.2's documented context-dimension deferral.
- Recent-re-auth/OIDC ceremony for `purge_execute` is out of scope here — it is Task 3.3 scope.
- `rotate_epoch_and_rewrap` is not itself idempotent, so a crash between it and recording `key_revoked` can rotate an extra epoch on resume. This is harmless (surviving objects stay readable) but wasteful — the same category of accepted tradeoff as Task 1.2b's dedup-hit re-encrypt cost.
- `retention_pending` only addresses backups this store itself created and tracked, matching the ADR's own backup model (external copies remain operator responsibility).

## DoD checklist mapped to evidence

Verbatim mapping from GOAL-vNext §13 Task 1.3, per the Task Brief:

1. **Explicit user save / latest scoped correction / AI proposal policies as tests** → Sub-slice A (`41d478d`) for the first two. "AI proposal policies" is already closed by pre-existing tests: `propose_inference_without_evidence_is_unsupported_and_cannot_be_confirmed` + `propose_inference_with_evidence_is_supported_and_confirmable` (`tests/semantic_claims_v1.rs`, Task 1.2), and the worker-capability gating tests in `tests/semantic_capability_v1.rs` (`c994527`/`a385702`: a propose-only client can capture/propose but not confirm).
2. **Trusted actor authenticated, ordering uses server `event_seq`, scope key validated** → Sub-slice A (`41d478d`).
3. **SQLite WAL does not lose updates under concurrent clients** → `f3a5565` (pre-existing, retroactively mapped) + the saga's own transactional step boundaries in sub-slice C (`5039a62`).
4. **Repeated `operation_id` returns the same result without duplication** → `f3a5565` (pre-existing) + the purge replay test in sub-slice C (`5039a62`).
5. **Same `operation_id`, different payload ⇒ `IDEMPOTENCY_CONFLICT`, no state change** → `f3a5565` (pre-existing) + the purge variant in sub-slice C (`5039a62`).
6. **Worker credential cannot confirm/purge** → `a385702` (confirm half, pre-existing) + Sub-slice C (`5039a62`) closes the purge half: `purge_execute` requires the `purge` capability; a propose-only worker is rejected with `CapabilityDenied`.
7. **Hard-purge saga crash/retry complete at every step (ledger request, registry replication, key revocation, live/object deletion, projection cleanup, retention, completion); after `registry_denied` every read/decrypt fails closed** → Sub-slices B (`6955bce`) + C (`5039a62`).

## Notes and carried risks

- Toolchain and Python-suite invocation follow the established conventions from prior tasks: `cargo +1.95-x86_64-pc-windows-msvc` for every Rust gate; the locked eval and Python suites were run against a fresh `llm-wiki` binary built in an isolated `CARGO_TARGET_DIR`, not the stale in-repo `target/`.
- The builder-role transfer to Claude (superseding the §12.1 Codex role, approved by the user 2026-07-15) still has no Decision History entry in `brain_2nd/GOAL-vNext.md` as of this revision; a numbered entry is to be added at or before task close, matching the convention of every prior role change.
- Existing carried risks from prior tasks (4 RUSTSEC findings, CI action tags, fixture pollution, Windows pagefile sensitivity) remain unchanged and unsuppressed; the audit comparison above confirms no new findings were introduced by this task (zero new dependencies).
- `rotate_epoch_and_rewrap`'s non-idempotency under crash (see "Disclosed scope limits" above) is a known, accepted gap, not a silent one.
- **New carried risk from the security review, required before Phase 3/4 auth-boundary wiring (ADR Decision 8):** capability enforcement across the semantic module is per-method, not per-handle — `register_client_scoped`, `destroy_wrapped_key`, `rotate_epoch_and_rewrap`, `append_registry_denial`, `sync_purge_registry`, `backup_consistent`, `recover`, `purge_resume`, and `purge_status` take no `TrustedContext`/capability parameter, so any code holding a raw `&SemanticStore` could call them directly. Not exploitable today (no transport/tool layer exists yet to hand such a reference to less-trusted code) and mostly pre-existing (Tasks 1.1/1.2b), but must close before any Phase 3/4 component shares a `SemanticStore` handle with less-trusted code.
- **New carried risk from the security review, not fixed in this task:** no automatic claim-ID-to-object-ID expansion when building a purge target list (a claim's plaintext can be duplicated across its proposal/confirmation/source objects); `evidence_capture_operation_ids`/`superseded_claim_operation_ids` have no length cap and `propose_inference` requires no capability, a single-writer stall risk; `purge_resume`/`purge_status` have no per-caller ownership check. All three are either pre-existing Task 1.2 code outside this task's permitted paths, or low-impact/same Phase 3 scope as the capability-enforcement risk above.

## Independent Validator final result

Round 1 (against `5039a62`): FINDINGS — 2 MEDIUM (backup-invalidation scope; missing TOCTOU race test), both fixed in `a134700`; 4 of 6 prior Task-Brief-review findings re-confirmed closed.

Round 2 (against `a134700`): **PASS.** The validator independently re-ran every gate (fmt, both clippy configs, all 8 semantic suites `--all-features` at 83/83, full-repo regression at 0 failed, `cargo audit` at 4 unchanged findings, `cargo-llvm-cov` at 89.74%), re-read the diff for both fixes, and executed both new tests directly rather than trusting this report or the commit message. It also explicitly reviewed and agreed with the scope decision to carry the security review's HIGH finding (per-method rather than per-handle capability enforcement) forward to Phase 3/4 rather than fix it in this task. Zero remaining findings at any severity.

---

**Independent Validator: PASS. Task 1.3 closed.**
