# Task 4.1 — Provider adapter + Z.ai compliance gate Report

Status: **PASS** — Independent Validator confirmed (after fix-round) on branch `vnext/phase-0` (RED `79b5738`, GREEN `5e0089b`, fix-round this commit). The first pass returned 1 MEDIUM finding (`detect_secret` `sk-` branch logic inverted — counted substring occurrences instead of key-material length, so a single realistic `sk-...` key slipped through) — closed by rewriting the `sk-` detection to measure key-material length (≥16 alphanumeric/`-`/`_` chars after the prefix) and adding three regression tests (single-key denial across realistic shapes + denial-reason-never-contains-the-secret). Two secondary observations (api_key_ref raw-key guard by convention only, destination-audit deferred) are acknowledged follow-ups for the deployment adapter, not Task 4.1 blockers.

## Why this task exists

Task 4.1 is the domain boundary for AI extraction/synthesis/consolidation (GOAL-vNext §8.1, §13 Phase 4 Task 4.1). The domain core must depend on a provider-agnostic trait — no Z.ai-specific field/model id baked in — with the endpoint/key/models in config (not durable schema), a kill switch, a deny-by-default outbound policy (§8.2), full error coverage, and an auditable compliance record. Real HTTP termination (the concrete Z.ai adapter) is a deployment follow-up; this task delivers the contract a deployment adapter implements against.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `db69ecf` (Phase 3 Gate closed).

- **RED — `79b5738`** ("test(provider): Task 4.1 RED checkpoint"). `tests/provider_compliance_v1.rs` (new, 12 tests) against not-yet-existing `llm_wiki::provider` module.
- **GREEN — `5e0089b`** ("feat(provider): Task 4.1 GREEN"). `src/provider.rs` (new: AiProvider trait, ProviderRequest, ProviderConfig, OutboundPolicy, ProviderError, ComplianceRecord) + `src/lib.rs` (module registration). All 12 tests passed.
- **Fix-round — this commit.** Closed the Validator's MEDIUM:
  - Rewrote `detect_secret`'s `sk-` branch to measure key-material length (the chars after `sk-` that are alphanumeric/`-`/`_`, ≥16 triggers a match) instead of the inverted substring-count arithmetic that missed a single key.
  - Added `outbound_policy_denies_a_single_realistic_sk_key` (3 realistic shapes: `sk-proj-...`, `sk-<32hex>`, `key=sk-...`).
  - Added `denial_reason_never_contains_the_secret` (locks the reason-safety invariant that was correct but untested).

## DoD verification

### Bullet 1 — no Z.ai field in domain core (MET)
`ProviderRequest` serializes to `{prompt, max_tokens, temperature, local_only}` — no `zai_*`/`glm_*`/`provider` key (test `provider_request_is_provider_agnostic`). All `grep -i "z\.ai\|glm"` hits in `src/` are doc comments. `AiProvider` is a trait the domain depends on (`ai_provider_is_a_trait`).

### Bullet 2 — config not durable + compliance record (MET)
`ProviderConfig` holds `base_url`/`api_key_ref` (secret-manager REFERENCE, not raw key)/models/`kill_switch` as plain config. `ComplianceRecord` carries all §8.2 fields (`compliance_record_carries_required_fields`).

### Bullet 3 — kill switch + swap without migration (MET)
`is_disabled()` returns `kill_switch` (`provider_config_has_kill_switch`, `provider_config_enabled_when_kill_switch_off`). base_url/models are swappable Strings; provider module has zero coupling to semantic store (`grep -rn "semantic::" src/provider.rs` empty).

### Bullet 4 — error coverage (MET)
All 7+1 failure modes: Timeout, QuotaExhausted, RateLimited (429), ServerError(5xx), InvalidJson, PartialStream, Outage, Disabled. `is_retryable()` correct for each (`provider_error_covers_all_failure_modes`, `provider_error_retryable_classification`).

### Bullet 5 — secret handling + destination audit (MET after fix)
`OutboundPolicy` denies `local_only` unconditionally + detected secrets (Bearer, sk- key ≥16 material, access_token=, api_key=). Conservative over-detection. Destination audit deferred to deployment adapter (the policy struct is the hook).

### Bullet 6 — intercepted outbound (MET after fix)
`outbound_policy_denies_by_default_when_local_only`, `outbound_policy_denies_detected_secrets`, `outbound_policy_denies_a_single_realistic_sk_key`, `denial_reason_never_contains_the_secret`.

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test provider_compliance_v1`: 14/14 pass (after fix-round).
- `cargo test -j 2` (default): 0 failed across all binaries.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing.
- `cargo audit`: 1 vuln + 3 warnings (== baseline, NO new dependency — `Cargo.toml`/`Cargo.lock` unchanged).
- Isolation: `semantic_vertical_slice` 14/14 pass; `grep -rn "semantic::" src/provider.rs` empty.

## Security review (Independent Validator, all clear after fix)

- **`detect_secret`**: Bearer/access_token=/api_key= correct from the start; `sk-` branch fixed (key-material length, not substring count). Conservative over-detection is the safe direction.
- **Reason-safety**: `OutboundDecision.reason` carries only a label (`bearer`/`sk-key`/`access_token`), never the secret — now explicitly tested.
- **api_key_ref**: raw-key guard is by convention (a `String` field); the deployment adapter that resolves the ref should add a refuse-if-it-looks-like-a-raw-key guard. Follow-up.
- **TM-012**: no `semantic::` import — provider is pure domain contract.

## Carried risks / deferrals

- **Deferred to deployment adapter**: concrete Z.ai HTTP call, destination-allow-list audit, `api_key_ref` raw-key refuse guard, log-redaction of `InvalidJson`/`ServerError` payloads, §11 adversarial 30-case no-egress corpus (Phase 4 exit gate).
- **Inherited, unchanged**: HIGH per-handle capability enforcement (Phase 6); schema break v1→v2; `cargo audit` 1 vuln + 3 warnings; eval `byte_lock_passed: false`.

## Phase 4 status

Task 4.1 (provider adapter + compliance gate) ✅ PASS. **Task 4.2 (Evidence-linked extraction pipeline)** is the next permitted implementation task, pending its own Task Brief review. The Phase 4 Gate (AI creates traceable proposals, no direct truth mutation, no critical unsupported-confirm case in eval) remains open until Tasks 4.2 and 4.3 close.
