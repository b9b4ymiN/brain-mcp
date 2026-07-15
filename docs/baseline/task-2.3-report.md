# Task 2.3 — Backfill and cutover Report

Status: **PASS** — Independent Validator confirmed at HEAD (this commit) on branch `vnext/phase-0` (RED `c56e2e3`, GREEN `b890793`, doc-fix this commit), after a first pass returned 0 CRITICAL/HIGH/MEDIUM findings and 2 LOW informational notes (one acted on — rustdoc corrected to match the resolve-only implementation; one deferred — ungated test helpers, acceptable for pre-production). The validator independently re-ran every gate and confirmed **Task 2.3 closes the Phase 2 Gate.**

## Why this task exists

Task 2.3 closes the Phase 2 GOAL-vNext.md §13 DoD for backfill: a migration must report migrated/skipped/ambiguous/error for every record, LLM-backfilled claims must start as proposed (not confirmed), old/new read parity must hold before cutover, and rollback + rerun must be idempotent. Task 2.2 left `claim_status.entity_id` nullable precisely so a legacy store (or a row that lost its entity binding) could be backfilled without a destructive migration; this task delivers that backfill path.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `839c388` (Phase 2 Task 2.2 closed).

- **RED — `c56e2e3`** ("test(migration): Task 2.3 RED checkpoint"). `tests/semantic_migration_v1.rs` (new, 7 tests) written against the not-yet-existing backfill API. Confirmed RED: compile failure on `backfill_entity_ids`, `MigrationReport`, and three test helpers.
- **GREEN — `b890793`** ("feat(migration): Task 2.3 GREEN"). `src/semantic.rs` adds `MigrationRecord` + `MigrationReport` structs, `backfill_entity_ids(context, dry_run)` method (resolve-only, transactional, idempotent), and three test-only helpers. All 7 new tests passed.
- **Doc fix — this commit.** Closed the Validator's LOW note: `backfill_entity_ids` rustdoc corrected from "resolved (or lazily created)" to "resolve-only ... does not lazily create entities" to match the implementation.

Permitted paths: `src/semantic.rs` (additive — new structs + method + test helpers, no change to existing transition logic or DDL), `tests/semantic_migration_v1.rs` (new), `docs/baseline/task-2.3-report.md` (this file). No new src files, no allowlist change, no ADR/eval/schema touch.

## DoD verification (all four bullets, with evidence)

### Bullet 1 — migration report shows migrated/skipped/ambiguous/error for every record

`MigrationReport` carries four counts + a `records: Vec<MigrationRecord>` with one entry per examined `claim_status` row. The loop in `backfill_entity_ids` pushes exactly one record per row across four branches:
- `error` — `claim_id` failed to parse as a UUID (corrupt row).
- `skipped` — row already had a non-NULL `entity_id`.
- `migrated` — NULL `entity_id` resolved to an existing entity via `entity_aliases`; binding written (unless `dry_run`).
- `ambiguous` — NULL `entity_id` with no matching entity; backfill does not guess.

Tests: `dry_run_classifies_already_bound_rows_as_skipped`, `apply_migrates_legacy_null_entity_id_rows`, `unresolvable_row_is_ambiguous_not_error`.

### Bullet 2 — LLM-backfilled claims start as proposed, not confirmed

`backfill_entity_ids` is **resolve-only**: it calls `resolve_entity_in_tx` (a SELECT), never `resolve_or_create_entity_in_tx`, `propose`, `confirm`, or `mutate`. The only write is `UPDATE claim_status SET entity_id=?` on already-confirmed rows. An unconfirmed LLM proposal is not in `claim_status` at all, so it is invisible to backfill. LLM-derived claims must still enter the store via `propose_inference`, which emits `claim_proposed` (status `proposed`) — §5 Memory Policy, enforced at the API surface (the `UnsupportedInference` gate still blocks confirm). Test: `backfill_never_promotes_an_llm_claim_to_confirmed`.

### Bullet 3 — old/new read parity

`read_parity_holds_across_backfill` confirms that `all_claims_current` before and after a backfill returns identical `subject`/`predicate`/`value`/`claim_kind`/`provenance_kind`/`origin` for every claim; only `entity_id` goes None→Some. Backfill is a metadata fix, not a semantic mutation.

### Bullet 4 — rollback + rerun idempotent

- Idempotent: `rerun_backfill_is_idempotent` — second run reports `migrated=0, skipped=1` (the previously-migrated row is now skipped, no duplicate work, no error).
- Rollback: `rollback_to_null_bindings_is_lossless_and_recoverable` — nulling `entity_id` back leaves the claim fully readable (subject/value intact); re-running backfill restores the binding to the **same** entity_id (stability across rollback/recover).

## Design

**Resolve-only, not lazy-create.** The key safety property: backfill resolves `(domain, subject)` against existing `entity_aliases` only. It never mints a new entity. A real confirmed claim always resolves because Task 2.2's confirm path created its entity; only orphan rows (from a partial import, with no matching entity) fail as `ambiguous`. This matches ADR Decision 1 (no silent writes, no dual source of truth) — backfill does not invent entity identity, it only repairs a missing reference to an identity that already exists.

**Transactional + writer-locked.** `backfill_entity_ids` takes the in-process writer lock before opening the Immediate transaction (matching `mutate_once`'s ordering, per the Task 2.2 fix-round F-2 precedent). All row examinations + writes happen in one transaction; `dry_run` skips the commit.

**Metadata-only.** The only UPDATE is `SET entity_id=?`. No claim payload field (subject/predicate/value/provenance) is touched, so §4 rule 1 (no silent overwrite) holds and read parity is structural.

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test semantic_migration_v1`: 7/7 pass.
- `cargo test -j 2` (default features): 671 passed / 0 failed across 40 binaries.
- `cargo test --all-features -j 2`: 682 passed / 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` is pre-existing baseline (Task 2.3 touched none of the 9 byte-locked files).
- `cargo audit`: 1 vuln + 3 warnings (== baseline, no new dependencies).

## Security review (Independent Validator, all clear)

- **TM-002 (Critical):** backfill cannot promote any proposal to confirmed. The method body contains exactly one write — `UPDATE claim_status SET entity_id=?` — gated by `if !dry_run`. No INSERT, no DELETE, no touch of `proposal_status`/`events`/`entities`/`entity_aliases`. `validate_context` at entry rejects untrusted contexts.
- **ADR Decision 1 (no dual truth):** resolve-only; an orphan row is `ambiguous`, never silently bound to a fresh entity.
- **§4 rule 1 (no silent overwrite):** only `entity_id` is written; payload fields untouched.
- **§4 rule 6 (idempotent):** bound rows are skipped on rerun, no error.

## Carried risks

- **LOW (deferred):** the three `*_for_test` helpers (`null_entity_id_for_test`, `null_all_entity_ids_for_test`, `insert_orphan_claim_status_for_test`) are `pub` with no feature gate, by necessity (integration tests in `tests/` cannot reach `#[cfg(test)]` items). Documented test-only; each takes the writer lock and panics on misuse rather than corrupting silently. Pre-release hardening ticket: gate behind `#[cfg(any(test, feature = "test-fixture"))]`.
- **Inherited, unchanged:** HIGH per-handle capability enforcement (Phase 3/4); schema break v1→v2 with no migration path (pre-production); `cargo audit` 1 vuln + 3 warnings (open since Task 0.1); eval `byte_lock_passed: false` (pre-existing).
- **Toolchain (inherited):** `-j 2` + `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc` required on this Windows host.

## Phase 2 Gate — CLOSED

Task 2.1 (projection adapters) ✅ PASS. Task 2.2 (ownership + entity) ✅ PASS. Task 2.3 (backfill + cutover) ✅ PASS. The Phase 2 Gate (production-like copy migrate/rebuild/rollback with no data loss and no MCP read regression) is satisfied: backfill is a transactional, idempotent, resolve-only, metadata-only UPDATE; rollback is lossless (`git revert` + nulling the column both recover); read parity is structural; no MCP tool surface changed in Phase 2. **Phase 3 (MCP vNext and Client Interoperability) is clear to start** pending its own Task Brief review.
