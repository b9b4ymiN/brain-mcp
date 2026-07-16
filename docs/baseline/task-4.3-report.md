# Task 4.3 — Consolidation + domain evals Report

Status: **PASS** — Independent Validator confirmed (after doc fix) on branch `vnext/phase-0` (RED `263e8eb`, GREEN `b500a7d`, doc fix this commit). The validator verified all gates independently, returned PASS, and confirmed Task 4.3 closes the Phase 4 Gate at the contract level. The single LOW finding (doc-clarity on the no-confidence `is_auto_approved` variant) was closed by tightening the rustdoc to state explicitly it is a pre-check, not an approval.

## Why this task exists

Task 4.3 closes GOAL-vNext §13 Phase 4 Task 4.3 + §8 consolidation cycle + §11 eval contract: duplicate/contradiction/stale candidates go to a review queue (never auto-applied), the domain eval covers stocks/projects/knowledge across update/time/provenance/abstention with abstention as a hard invariant, auto-approve is off by default and gated per-type with a confidence threshold, and a model/prompt regression report blocks promotion if any metric regresses beyond bound.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `5f26fb0` (Task 4.2 PASS).

- **RED — `263e8eb`** ("test(consolidation): Task 4.3 RED checkpoint"). `tests/consolidation_v1.rs` (new, 9 tests) against not-yet-existing `llm_wiki::consolidation` module.
- **GREEN — `b500a7d`** ("feat(consolidation): Task 4.3 GREEN"). `src/consolidation.rs` (new) + `src/lib.rs`. All 9 tests passed.
- **Doc fix — this commit.** Closed the Validator's LOW: tightened `is_auto_approved` rustdoc to state it is a pre-check, not an approval; the actual gate is `is_auto_approved_with_confidence`.

## DoD verification

### Bullet 1 — dedupe/contradiction/stale → review queue (MET)
`ConsolidationKind` (Duplicate/Contradiction/Stale, distinct). `ConsolidationReport` has ONLY `candidates` — NO `applied` field, so auto-merge is structurally forbidden (§8.1).

### Bullet 2 — domain eval 3 domains × 4 dimensions (MET)
`DomainEvalReport` carries stocks_update / projects_time / knowledge_provenance / adversarial_abstention. `abstention_passed_100()` enforces 100% (hard invariant — §11).

### Bullet 3 — auto-approve off default + threshold (MET)
`AutoApprovePolicy::default()` off for all. `enable(type, source, threshold)`. `is_auto_approved_with_confidence` is the full gate (enabled AND confidence ≥ threshold); below-threshold and unenabled-source denied.

### Bullet 4 — regression report blocks promote (MET)
`RegressionReport::is_promotable()` requires every delta within `max_allowed_regression` on all three metrics (recall@10 / nDCG@10 / abstention); any single out-of-bound metric blocks.

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test consolidation_v1`: 9/9 pass.
- `cargo test -j 2` (default): 0 failed across all binaries.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing.
- `cargo audit`: 1 vuln + 3 warnings (== baseline, NO new dependency).
- Isolation: `semantic_vertical_slice` 14/14; `grep -rn "semantic::" src/consolidation.rs` empty.

## Phase 4 Gate — CLOSED

Task 4.1 (provider adapter) ✅ PASS. Task 4.2 (extraction pipeline) ✅ PASS. Task 4.3 (consolidation + evals) ✅ PASS. The Phase 4 Gate (GOAL-vNext §13: "AI สร้าง proposal ที่ตรวจย้อนกลับได้, ไม่มี direct truth mutation และ eval ไม่มี critical unsupported-confirm case") is satisfied at the contract level:
1. AI creates traceable proposals (ExtractionProposal links exact evidence; ConsolidationCandidate carries kind + claim_ids + detail).
2. No direct truth mutation (ExtractionOutcome has no commit field; ConsolidationReport has no applied field; auto-merge structurally forbidden).
3. No critical unsupported-confirm case (ExtractionPolicy rejects unsupported proposals; DomainEvalReport enforces abstention 100%).

**Phase 5 (Smart Console + Galaxy Graph) is clear to start** pending its own Task Brief review.

## Carried risks / deferrals

- **Integration task (explicit acceptance criteria)**: real LLM-backed detectors for dedupe/contradiction/stale; real rendition-byte quote-hash verification (Task 4.2 F1); real JSON-schema validation wiring (Task 4.2 F2); §11 adversarial corpus run against the real adapter.
- **Deferred**: concrete Z.ai HTTP adapter, `brain_*` semantic wiring, Console/Galaxy UI.
- **Inherited, unchanged**: HIGH per-handle capability enforcement (Phase 6); schema break v1→v2; `cargo audit` 1 vuln + 3 warnings; eval `byte_lock_passed: false`.
