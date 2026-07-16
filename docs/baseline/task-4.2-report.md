# Task 4.2 — Evidence-linked extraction pipeline Report

Status: **PASS** — Independent Validator confirmed (after fix-round) on branch `vnext/phase-0` (RED `606317c`, GREEN `0114b6b`, fix-round this commit). The first pass returned 0 CRITICAL/HIGH findings and 3 informational: F1 (quote_hash mechanical proof — deferred to integration), F2 (schema validation not wired — deferred to integration), F3 (build-then-check ordering — closed in this fix-round by adding `OutboundPolicy::check_text` and flipping `build_provider_request` to check-then-build). The validator confirmed the prompt-injection resistance and no-commit guarantees are structural, not merely policy.

## Why this task exists

Task 4.2 is the domain contract for turning a captured source into typed semantic proposals (GOAL-vNext §13 Phase 4 Task 4.2, §6.2 claim/provenance, §4 rules 5/8). The pipeline yields `ExtractionProposal`s that either link exact evidence spans or are flagged `unsupported` (and rejected by the policy before reaching the confirm path). The worker physically cannot commit — `ExtractionOutcome` has no commit/confirm/side-effect field (§4 rule 5: AI proposes, policy commits). Prompt-injection resistance is structural: source text becomes prompt *data*, never an instruction; the worker has no tools to execute.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `2fbd1e7` (Task 4.1 PASS).

- **RED — `606317c`** ("test(extraction): Task 4.2 RED checkpoint"). `tests/extraction_pipeline_v1.rs` (new, 12 tests) against not-yet-existing `llm_wiki::extraction` module.
- **GREEN — `0114b6b`** ("feat(extraction): Task 4.2 GREEN"). `src/extraction.rs` (new: EvidenceSpan, ExtractionProposal, ExtractionAudit, ExtractionOutcome, ExtractionPolicy, ProposeError) + `src/lib.rs`. All 12 tests passed.
- **Fix-round — this commit.** Closed F3: added `OutboundPolicy::check_text(text, local_only)` (gates raw text without allocating a ProviderRequest) and flipped `ExtractionPolicy::build_provider_request` to check-then-build, so a denied secret never lands in a `ProviderRequest.prompt` even momentarily.

## DoD verification

### Bullet 1 — typed proposals + provenance + schema validation (MET; schema-validation wiring deferred)
`ExtractionProposal` carries subject/predicate/value/claim_kind/domain/confidence + evidence + unsupported. `ProposeError::SchemaInvalid` is defined for the integration task that wires a real JSON-schema validator into `validate()`.

### Bullet 2 — exact evidence / unsupported (MET; quote-hash mechanical proof deferred)
`EvidenceSpan` carries rendition_id + quote_hash + half-open byte range; `is_well_formed()` rejects `byte_end <= byte_start`. `validate()` rejects any unsupported proposal (no evidence) — this is the prompt-injection guarantee: even if injected text "says" to confirm, a proposal with no evidence cannot pass. The integration task will add mechanical quote-hash verification against rendition bytes.

### Bullet 3 — audit + no commit tools (MET)
`ExtractionAudit` records prompt_version/model/schema_version/adapter_name. `ExtractionOutcome` has only `proposals + audit` — NO commit/confirm/purge field (grep-verified). The worker physically cannot commit.

### Bullet 4 — prompt-injection (MET, structural)
Resistance is STRUCTURAL: (a) `ExtractionOutcome` has no side-effect field; (b) `ProviderRequest` has no tools/function-calling channel; (c) `validate()` derives pass/fail solely from `evidence.is_empty()` and span shape, never from text semantics. Test: `prompt_injection_does_not_force_confirm_or_clear_unsupported`.

### Bullet 5 — local-only/secret 100% (MET)
`build_provider_request` returns None for local_only (test `local_only_source_never_egresses`) and for secret-bearing source (`secret_in_source_is_refused`); clean source yields a request (`clean_source_produces_provider_request`). After F3 fix, the gate runs before the source is copied into the request struct.

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test extraction_pipeline_v1`: 12/12 pass.
- `cargo test -j 2` (default): 0 failed across all binaries.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing.
- No new dependency: `Cargo.toml`/`Cargo.lock` unchanged.
- Isolation: `semantic_vertical_slice` 14/14; `grep -rn "semantic::" src/extraction.rs` empty.

## Independent Validator findings (all closed or acknowledged deferrals)

- **[MEDIUM F1, deferred]** quote_hash mechanical proof — `validate()` checks span shape only, not hash-vs-rendition-bytes. Deferral: the integration task that has rendition bytes in scope will add the re-slice + hash compare. Tracked as explicit acceptance criteria.
- **[LOW F2, deferred]** schema validation not wired — `ProposeError::SchemaInvalid` defined but unused. Same integration-task deferral.
- **[INFO F3, fixed]** build-then-check ordering — closed by `check_text` + check-then-build.

## Carried risks / deferrals

- **Integration task (explicit acceptance criteria)**: wire real rendition bytes into `validate()` for quote-hash mechanical proof; wire a JSON-schema validator for `proposal.value`; run the §11 adversarial prompt-injection corpus + local-only/secret negative corpus against the real adapter.
- **Deferred**: concrete Z.ai HTTP adapter (Task 4.1 deployment follow-up), `brain_*` semantic wiring.
- **Inherited, unchanged**: HIGH per-handle capability enforcement (Phase 6); schema break v1→v2; `cargo audit` 1 vuln + 3 warnings; eval `byte_lock_passed: false`.

## Phase 4 status

Task 4.1 (provider adapter) ✅ PASS. Task 4.2 (extraction pipeline) ✅ PASS. **Task 4.3 (Consolidation + domain evals)** is the next permitted implementation task, pending its own Task Brief review. The Phase 4 Gate (AI creates traceable proposals, no direct truth mutation, no critical unsupported-confirm case in eval) remains open until Task 4.3 closes.
