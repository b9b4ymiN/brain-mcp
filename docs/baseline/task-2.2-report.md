# Task 2.2 — Human/agent/generated content ownership Report

Status: **PASS** — Independent Validator confirmed at HEAD `6d41046` on branch `vnext/phase-0` (RED `ae564a2`, GREEN `248cc15`, fix round `6d41046`), after a first pass returned 1 MEDIUM finding (closed), 1 LOW finding (closed), and 1 INFO observation (comment corrected). The validator independently re-ran every gate and re-read every changed file for the re-verification pass rather than trusting the reported numbers, and returned a final **PASS** with 0 CRITICAL/HIGH/MEDIUM open. Clear to close Task 2.2.

## Why this task exists

Task 2.2 closes the Phase 2 GOAL-vNext.md §13 DoD for content ownership: file ownership must be classified as `human-authored`, `agent-proposed`, or `generated`; a human edit must become an authored event while a generated edit must not silently mutate state; and entity rename/merge must preserve stable IDs and backlinks. It also closes the provenance gap left open since Task 1.2: ADR Decision 6 names four provenance variants (`evidence`, `user_assertion`, `mechanical`, `inference`) but only the first two were implemented. Task 2.1's report explicitly deferred "richer typing" to "Task 2.2's ownership model," and this task delivers that model.

The entity model is net-new: prior to this task, `subject` was a free-form `String` with no identity indirection, no rename, and no merge. ADR Decision 3 mandates stable UUIDv7 IDs that never encode the subject string; Task 2.2 introduces the `entities` + `entity_aliases` tables and the resolution/alias machinery that makes rename/merge preserve backlinks.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `dba4456` (Phase 2 Task 2.1 closed).

- **RED — `ae564a2`** ("test(ownership): Task 2.2 RED checkpoint"). `tests/semantic_ownership_v1.rs` (new, 12 tests) + `tests/projection_ownership_v1.rs` (new, 6 tests) written against the not-yet-existing Task 2.2 API. Confirmed RED: `cargo test --no-run` failed with 46 compile errors (missing methods, missing structs, missing enum variants) — all against the target API surface, no incidental type errors against existing signatures.
- **GREEN — `248cc15`** ("feat(ownership): Task 2.2 GREEN"). `src/semantic.rs` (Provenance gains `UserAssertion`/`Mechanical`; `ClaimView` gains `provenance_kind`/`origin`/`entity_id`; `OriginClass` enum; `propose_user_assertion`/`propose_mechanical`; entity model — `entities`/`entity_aliases` tables, `resolve_or_create_entity`/`resolve_entity`/`entity_by_id`/`rename_entity`/`merge_entities`/`claims_for_entity`/`entity_canonical_subjects`; schema split DISK=2/EVENT=1; `validate_database_identity` fail-closed gate) + `src/projection.rs` (`render_claim_page` emits `origin`/`provenance`/`entity_id` frontmatter, `claim_kind → page type` map, canonical-subject projection). All 18 new tests passed on the first run.
- **Fix round — `6d41046`** ("fix(ownership): validator fix round"). Closed all three Independent Validator findings: MEDIUM F-1 (`cargo fmt` reflow on two test files), LOW F-2 (`resolve_or_create_entity` writer-lock ordering before `BEGIN IMMEDIATE`, matching `mutate_once`), INFO F-3 (`merge_entities` comment corrected — the source row is deleted, not kept).

Permitted paths per the Task Brief: `src/semantic.rs` (additive — new variants, new methods, new tables, no change to existing transition logic), `src/projection.rs` (frontmatter extension + canonical-subject projection), `tests/semantic_ownership_v1.rs` (new), `tests/projection_ownership_v1.rs` (new), `docs/baseline/task-2.2-report.md` (this file). The isolation allowlist in `tests/semantic_vertical_slice.rs` was NOT widened — all new code lives in files already authorized to reference `semantic::`.

## DoD verification (all three bullets, with evidence)

### Bullet 1 — file ownership as `human-authored` / `agent-proposed` / `generated`

`OriginClass` enum (`src/semantic.rs`) is derived deterministically from the provenance variant in `build_claim_view`:
- `UserAssertion` / `Mechanical` → `HumanAuthored`
- `Evidence` / `Inference` → `AgentProposed`

The `generated` class is a *file* property of the projection output, modeled by the pre-existing `OWNERSHIP_MARKER` (`.projection-owned`) directory guard from Task 2.1 — not a claim origin. ADR Decision 1 defines generated wiki as a "replaceable materialized view," distinct from claim provenance.

Projection frontmatter (`src/projection.rs::render_claim_page`) emits `origin` (`human-authored`/`agent-proposed`), `provenance` (the variant name), and `entity_id`. Tests: `generated_page_carries_origin_provenance_and_entity_id_for_a_user_assertion`, `generated_page_carries_agent_proposed_origin_for_an_evidence_backed_claim`, `generated_page_type_reflects_claim_kind_not_a_hardcoded_entity`.

### Bullet 2 — human edit becomes authored event; generated edit does not silently mutate state

Human path: `propose_user_assertion` mints a `claim_proposed` event with `Provenance::UserAssertion`, capturing the utterance bytes as a content-addressed object inside the same transaction. This is a real ledger event, not a silent write — `confirm` then promotes it to `claim_confirmed` like any other proposal. Test: `user_assertion_propose_and_confirm_produces_a_user_assertion_claim`, `user_assertion_proposal_does_not_require_a_prior_capture`.

Generated-wipe: `rebuild_projection` `remove_dir_all`s the `.projection-owned` generated root on every call (after verifying the ownership marker), so a hand-edited generated page is discarded on the next rebuild — it never becomes semantic state. Test: `hand_editing_a_generated_page_does_not_survive_the_next_rebuild` (asserts the tampered page is gone AND the composite checksum is stable across the rebuild).

### Bullet 3 — entity rename/merge preserves stable IDs and backlinks

`rename_entity` updates `entities.canonical_subject` only; the entity_id UUIDv7 is untouched. The old subject is recorded as a `former_subject` alias, so `resolve_entity` keeps resolving it. Test: `rename_entity_preserves_id_and_keeps_old_subject_as_backlink`.

`merge_entities` rewrites every `claim_status.entity_id` from source onto target (no orphaned claims), folds the source's canonical subject + prior aliases onto the target as `former_subject` aliases (no alias deleted), and removes the source's canonical row. Resolution via `resolve_entity_in_tx` consults `entity_aliases` ordered by `aliased_at_event_seq DESC`, so the target-pointing rows always win. Tests: `merge_entities_moves_claims_and_keeps_aliases_as_backlinks`, `entity_id_is_stable_across_a_rename_visible_in_projection`.

Edge cases: self-merge and self-rename are rejected as `InvalidTransition` (`merge_entity_into_itself_is_rejected`); renaming onto an existing subject in the same domain is rejected — that is a merge (`rename_to_an_existing_subject_in_same_domain_is_rejected`); rename is idempotent under `operation_id` replay (`rename_entity_is_idempotent_under_operation_id_replay`).

## Design

**Provenance gap closure.** `Provenance` gains `UserAssertion { actor_id, utterance_object_id, utterance_byte_start, utterance_byte_end }` and `Mechanical { method, method_version, input_hashes, output_hash }`, matching ADR Decision 6's four-variant contract. Both new variants pass the `finish_confirmation` unsupported-inference gate (they are not `Inference { unsupported: true }`), so they can be confirmed. The existing `Evidence` and `Inference` paths are unchanged.

**Schema split (DISK=2, EVENT=1).** The on-disk DDL version bumped 1→2 (new `entities`/`entity_aliases` tables + `claim_status.entity_id` column). The event *wire* version stays at 1 — `EventEnvelope.schema_version` is unchanged because the JSON shape of an event did not change. This keeps the hash-locked `evals/v1/contracts/event-schema-v1.json` (which has `"schema_version": {"const": 1}`) valid. `validate_database_identity` now compares the marker's schema_version against `CURRENT_DISK_SCHEMA_VERSION` and fails closed on a mismatch (pre-production break, recorded like Task 1.1's clients-table precedent — no migration path exists yet).

**Origin derivation, not storage.** `origin` and `provenance_kind` are derived in `build_claim_view` from the decrypted `ClaimRecord.provenance`, not stored as separate columns. This keeps the claim payload the single source of truth for provenance and avoids a dual-source-of-truth risk. `entity_id` IS stored (on `claim_status`) because it is a structural reference, not a derived property.

**Canonical-subject projection.** A claim's payload records the subject string it was confirmed under (an immutable historical fact). The generated wiki, however, is a materialized view of the entity's current identity (ADR Decision 1). `rebuild_projection` reads `entity_canonical_subjects_owned()` and projects the live canonical subject onto each claim's page title, while the claim's own `subject` field stays in the body for provenance. This is why a renamed entity's page title follows the rename but its `entity_id` frontmatter stays constant.

## Gates (re-verified independently by Validator)

- `cargo fmt --check`: clean (after fix round).
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo test -j 2` (default features): 39 test binaries, 0 failed (~664 tests including the 18 new).
- `cargo test --all-features -j 2`: 0 failed.
- Python integration: engine 63 pass, mcp 76 pass, acp 26 pass + 2 skip (== baseline).
- Eval v1: 126/126 cases pass, hard invariants pass. `byte_lock_passed: false` is a **pre-existing baseline state** (9 files mismatched since before Task 2.2 — Task 2.2 did not touch any byte-locked file: ADR, threat model, event-schema, metrics, run.py, and the 4 case files are all untouched).
- `cargo audit`: 1 vulnerability + 3 warnings (== baseline, no new dependencies added in Task 2.2).

## Security review (Independent Validator, all clear)

- **TM-002 (Critical):** `finish_confirmation`'s unsupported-inference gate still fires before any claim row is written. The new `propose_user_assertion`/`propose_mechanical` paths produce `UserAssertion`/`Mechanical` provenance — never `Inference { unsupported: true }` — so there is no laundering path.
- **TM-009 (High):** The new `entity_id`/`origin`/`provenance_kind` fields are derived *after* bitemporal bucketing; the WHERE clauses and active/future/past logic are unchanged.
- **TM-010 (High):** Merge rewrites only `claim_status.entity_id`; it never touches supersede/retract sequences or claim values. A disputed scope stays disputed.
- **TM-012 (Critical):** `rebuild_projection` only reads from `SemanticStore` and writes solely to the `.projection-owned` generated dir — zero mutation calls (grep-verified).
- **TM-024 (Medium):** `actor_id` is recorded in the event envelope and `UserAssertion` provenance but is NOT surfaced on `ClaimView`. `origin`/`provenance_kind` describe HOW, not WHO. No person inference introduced.

## Carried risks (unchanged or newly documented)

- **HIGH (inherited, unchanged):** capability enforcement is per-method not per-handle (Task 1.3 security review). Still scoped to Phase 3/4 auth-boundary wiring; `rename_entity`/`merge_entities` correctly require the `confirm` capability via `mutate()`.
- **Pre-existing (not introduced by Task 2.2):** `cargo audit` 1 vuln (`crossbeam-epoch` RUSTSEC-2026-0204... actually `RUSTSEC-2026-0190` per current audit output) + 3 warnings (`anyhow`, `memmap2`, `bincode`). Open since Task 0.1.
- **Pre-existing (not introduced by Task 2.2):** eval v1 `byte_lock_passed: false` — 9 files mismatched the hash-locked manifest before this task. Task 2.2 touched none of them.
- **Toolchain (inherited):** `cargo test --all-features` requires `-j 2` on this Windows host (pagefile exhaustion at full parallelism); `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc` required (default gnu host lacks `dlltool.exe`).
- **Schema break (accepted, pre-production):** stores created under DISK schema v1 fail closed under the v2 binary. No migration path exists. All existing stores are fixtures (Task 1.1 precedent).

## Out of scope (deferred)

- `brain_split_entity` — not in Task 2.2 DoD; deferred to Task 5.3 (Console UI).
- File-system watcher (Markdown → auto event) — violates ADR Decision 1; Task 2.2 provides the service method only.
- Backfill of legacy `claim_status.entity_id` for pre-existing rows — Task 2.3 territory.
- MCP tool surface (`brain_*` tools) — Phase 3.
- Per-handle capability enforcement (HIGH) — Phase 3/4.

## Phase 2 status

Task 2.1 (projection adapters) ✅ PASS. Task 2.2 (ownership + entity) ✅ PASS. **Task 2.3 (Backfill and cutover)** is the next permitted implementation task, pending Task Brief review. The Phase 2 Gate (production-like copy migrate/rebuild/rollback with no data loss and no MCP read regression) remains open until Task 2.3 closes.
