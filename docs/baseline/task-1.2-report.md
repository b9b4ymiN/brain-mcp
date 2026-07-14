# Task 1.2 — Bitemporal Claims + State Machine Report

Status: **BUILDER GREEN — awaiting independent validation**

## Scope and TDD evidence

The task started from clean canonical HEAD `7fe6060` on `vnext/phase-0` (Task 1.1 validator PASS).

- RED checkpoint commit: `72fdc19` added `tests/semantic_claims_v1.rs` (11 tests). RED failure was 1 legitimate `E0432` (unresolved import) plus 25 `E0599` contract errors (`propose_inference`, `reject`, `retract`, `supersede`, `claims_current`, `ProposeInferenceCommand`, `RejectCommand`, `RetractCommand`, `SupersedeCommand`, `SemanticError::InvalidTransition`/`UnsupportedInference` all missing); no syntax/config failure.
- GREEN commit: `2e77409` changed `src/semantic.rs` and, during the RED→GREEN loop, one test assertion in `tests/semantic_claims_v1.rs`.
- **Two RED test-assertion defects found and fixed during GREEN** (both disclosed here after an Independent Validator flagged that the first pass under-reported this section):
  1. `supersede_across_mismatched_scope_is_rejected` asserted `diagnostics().events == 4` after five prior mutations (capture-a, propose-a, confirm-a, capture-b, propose-b) plus a rejected supersede attempt; the correct count is 5, since the rejected attempt legitimately adds zero events. The implementation was correct on the first compile; the test's own event-count arithmetic was off by one. Fixed to `5` with an inline comment explaining the count.
  2. `retract_of_unknown_or_already_retracted_claim_is_rejected` originally asserted `Err(SemanticError::InvalidTransition(_))` for retracting a `claim_operation_id` that was never used at all. That is wrong per the store's own existing convention: `stored_outcome` (unchanged, pre-existing helper) returns `MissingDependency` for any operation_id that was never completed, before any state/status check runs — the same behavior `propose`'s `capture_operation_id` lookup has always had since Task 0.3. Fixed the assertion to `Err(SemanticError::MissingDependency(_))`, with a comment distinguishing it from the "already retracted" case immediately below (which legitimately is `InvalidTransition`, since that operation_id resolves to a real, previously-retracted claim).
- **Error-taxonomy inconsistency found by the same validator pass, fixed**: `reject`/`finish_confirmation`'s resolution of `proposal_operation_id` to a `proposal_id` returned `MissingDependency` when the operation resolved successfully but was not actually a `propose`/`propose_inference` operation (e.g., a `capture_operation_id` passed by mistake) — inconsistent with `retract`/`supersede`'s equivalent resolution of a `claim_operation_id`, which correctly used `InvalidTransition` for the same class of caller error ("operation existed and resolved, but to the wrong kind of thing"). Standardized both proposal-id resolution sites on `InvalidTransition`, and added `operation_id_of_the_wrong_kind_is_rejected_as_invalid_transition_not_missing_dependency` to cover the previously-untested edge case for `confirm`, `reject`, and `retract` together.
- One clippy `type_complexity` finding (a five-element tuple query-row binding) was fixed by extracting a named `ClaimScopeRow` struct rather than suppressing the lint.

Permitted paths: `src/semantic.rs`, `tests/semantic_claims_v1.rs`, `docs/baseline/task-1.2-report.md`. No legacy Markdown/Git/MCP/ACP/HTTP/Tantivy/Petgraph runtime file was touched.

## Delivered behavior

- **Proposal state machine.** Every `propose`/`propose_inference` call inserts a `proposal_status` row (`proposed`). `confirm`/`supersede` require that status to still be `proposed` and transition it to `confirmed` (recording the resulting `claim_id`); `reject` requires the same and transitions it to `rejected`. Attempting to confirm an already-confirmed or already-rejected proposal, or reject an already-confirmed proposal, fails with `SemanticError::InvalidTransition` before any new event is written.
- **Retract.** `retract(claim_operation_id)` resolves the claim via the same-client `stored_outcome` lookup used elsewhere in the store, requires the claim to be a known, not-yet-retracted entry in `claim_status`, and stamps `retracted_at_event_seq` with the retracting event's own sequence. History is preserved; nothing is edited in place except the auxiliary status row.
- **Supersede.** `supersede(proposal_operation_id, superseded_claim_operation_ids)` requires a non-empty list (empty is rejected as `InvalidClaim`, forcing callers to use plain `confirm` for a genuinely new/disputed claim), resolves each prior claim, rejects if any is already superseded/retracted or in a different `(domain, subject, predicate)` scope, and — only if every check passes — writes one `claim_confirmed` event whose `supersedes` field lists the prior claim IDs, while stamping each prior claim's `superseded_by_event_seq`.
- **Inference provenance and the unsupported gate.** A new `Provenance::Inference { method, model, prompt_version, evidence, unsupported }` variant is populated by `propose_inference`; `unsupported` is `true` exactly when no evidence-capture operations were supplied. `confirm`/`supersede` reject an unsupported-inference proposal with a new `SemanticError::UnsupportedInference`, whose message states the required remediation (a fresh `user_assertion`/decision claim, not confirming the unsupported one).
- **Scoped, bitemporal query.** `claims_current(ledger_head, world_time, domain, subject, predicate)` returns `CurrentClaims { active, future, past }`. A claim is superseded/retracted "as of" a given `ledger_head` only when the recorded transition's own event sequence is at or before that head — comparing stored sequence numbers, not a mutable status label — so replaying an earlier head correctly excludes supersession/retraction recorded later. `future` holds confirmed, not-yet-superseded/retracted claims whose `valid_from` is still ahead of `world_time`; `past` holds everything superseded, retracted, or past its `valid_to`.

## DoD mapping

| DoD item | Evidence |
|---|---|
| proposed/confirmed/rejected/superseded/retracted transitions ถูกบังคับ | `reject_transitions_a_proposal_without_creating_a_claim`, `double_confirm_of_same_proposal_is_rejected_as_invalid_transition`, `retract_marks_a_confirmed_claim_as_no_longer_current`, `retract_of_unknown_or_already_retracted_claim_is_rejected`, `supersede_replaces_the_prior_claim_and_as_of_ledger_head_ignores_later_supersession`, `supersede_across_mismatched_scope_is_rejected`, `supersede_requires_at_least_one_prior_claim` |
| disputed transitions | `disputed_external_facts_coexist_without_superseding` (no transition at all — two independently confirmed claims coexist; see below) |
| valid time และ recorded time query แยกกันได้ | `claims_current(ledger_head, world_time, ...)` signature; every golden test below passes both independently |
| future-valid golden test | `future_valid_claim_is_known_but_not_current_until_valid_from` |
| correction golden test | `supersede_replaces_the_prior_claim_and_as_of_ledger_head_ignores_later_supersession` (GULF-style: 58 superseded by 62) |
| contradiction golden test | `disputed_external_facts_coexist_without_superseding` (two brokers' targets coexist as `disputed` — both in `active`) |
| as-of golden test | Same supersede test: `claims_current` at the pre-supersede `ledger_head` returns the original claim; at the post-supersede head it returns the new one |
| invalid transition rejected พร้อม actionable error | All `InvalidTransition` messages above name the claim/proposal ID and the specific reason (already confirmed/rejected/retracted/superseded, or scope mismatch) |
| evidence-less derived confirm ถูก reject | `propose_inference_without_evidence_is_unsupported_and_cannot_be_confirmed` |
| unsupported inference confirm/auto-approve ถูก reject | Same test: `confirm` on the unsupported proposal returns `SemanticError::UnsupportedInference` |
| user acceptance สร้าง user_assertion/decision ใหม่ | Same test: after the rejection, a fresh unrelated `propose`(claim_kind=`decision`)+`confirm` succeeds as a new claim, proving acceptance is a new event, not a mutation |
| (supporting) inference with evidence is confirmable | `propose_inference_with_evidence_is_supported_and_confirmable` |

## Scoped simplifications (recorded, not silently assumed)

- **Scope key omits `context`.** ADR Decision 6's full scope key is `(owner_id, domain, subject_id, predicate, normalized_context)`. `ClaimDraft` has no `context` field yet, so Task 1.2 treats scope as `(domain, subject, predicate)` under a single owner. This is a deliberate, documented simplification consistent with the task's approved design summary, not an oversight; adding `context` is deferred to whichever task first needs to disambiguate same-subject claims across different contexts (e.g., multiple portfolios).
- **Same-client operation_id resolution.** `retract`/`supersede`/`reject` resolve their target proposal/claim via `stored_outcome`, which is scoped to the calling context's `client_id` — identical to how `propose`'s `capture_operation_id` has always resolved since Task 0.3. A different registered client cannot yet retract/supersede a claim confirmed under another client's operation_id by string alone; stable cross-client reference by `claim_id` is Phase 3's `brain_get`-style read-tool surface, not a new gap introduced here.
- **`proposal_status`/`claim_status` are same-transaction authoritative tables**, following the precedent already accepted for Task 1.1's `clients` table: they are written atomically with the event that creates them and are not a replay-rebuildable projection. This keeps the change bounded; a replay-based rebuild path for these tables (if ever needed) is Phase 2 projection-adapter territory.
- **`RejectionObject`/`RetractionObject` mint no new claim.** Rejection references only `proposal_id`; retraction references only `claim_id`. Neither creates a `ClaimRecord`, keeping the append-only ledger free of a claim object for a value that was never (or is no longer) asserted.

## Verification results

| Gate | Result |
|---|---|
| New suite (`--test semantic_claims_v1`) | PASS — 11 passed |
| Task 0.3 suite (`--test semantic_vertical_slice`) | PASS — 18 passed, no regression |
| Task 1.1 suite (`--test semantic_store_v1`) | PASS — 11 passed, no regression |
| Rust format | PASS |
| Rust clippy all targets/features (`-D warnings`) | PASS |
| Rust clippy all targets, default features (`-D warnings`) | PASS (after extracting `ClaimScopeRow`) |
| Rust all targets/features | PASS — 605 passed, 0 failed (583 baseline + 11 + 11) |
| Semantic per-file coverage (`cargo-llvm-cov 0.8.6`, all three semantic suites) | PASS — 88.50% lines (1,701/1,922), minimum 80% |
| Locked eval (pinned `uv run --python 3.14.4`) | PASS — 126/126, `environment_passed=true`, `thresholds_passed=true` |
| Python governance / engine / mcp / acp | PASS — 10 / 63 / 76 / 26+2 known skips |
| Dependency audit comparison vs `task-0.3-audit-after.json` | PASS — before 4, after 4, zero new findings |

## Notes and carried risks

- Toolchain and Python-suite invocation follow the same conventions established in Tasks 0.3/1.1: `cargo +1.95-x86_64-pc-windows-msvc` for every Rust gate; `tests-integration` suites run with CWD = `tests-integration/` and `LLM_WIKI_BIN` pointed at the fresh `CARGO_TARGET_DIR` binary, not the stale in-repo `target/`.
- Existing carried risks (4 RUSTSEC findings, CI action tags, fixture pollution, Windows pagefile sensitivity) remain unchanged and unsuppressed.
