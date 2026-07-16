# Task 3.3 — Production auth boundary Report

Status: **PASS** — Independent Validator confirmed (after fix-round) on branch `vnext/phase-0` (RED `cec91ec`, GREEN `f09e2ea`, fix-round this commit). The first pass returned 2 LOW findings (cross-map consistency test gap + Purge-arm negative-test gap) — both closed in the fix-round by adding `annotation_profile_and_capability_map_agree` and `proposal_only_worker_is_denied_purge` + extending the admin-bypass test. The validator confirmed Task 3.3 closes the Phase 3 Gate at the contract level.

## Why this task exists

Task 3.3 closes GOAL-vNext §13 Phase 3 Task 3.3 + §7.1 auth matrix: TLS, Origin validation, audience/resource validation, and per-tool capabilities (`brain.read/capture/propose/confirm/purge/admin`) proven by negative tests (proposal-only worker cannot confirm/purge/admin); tokens never in URL/log/repo; every HTTP path has an auth policy with no admin bypass. Real TLS/OAuth/Keycloak termination is Phase 6 deployment; this task delivers the policy framework + capability enforcement + negative tests + token redaction that the deployment layer plugs into.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `e4a54ab` (Task 3.2 PASS).

- **RED — `cec91ec`** ("test(auth-boundary): Task 3.3 RED checkpoint"). `tests/mcp_auth_boundary_v1.rs` (new, 10 tests) against not-yet-existing `llm_wiki::mcp::auth` module.
- **GREEN — `f09e2ea`** ("feat(auth-boundary): Task 3.3 GREEN"). `src/mcp/auth.rs` (new: Capability enum, AuthPrincipal, AuthPolicy + AuthPolicyEntry, TokenRedaction) + `src/mcp/mod.rs` (module registration). All 10 tests passed.
- **Fix-round — this commit.** Closed the Validator's 2 LOW findings:
  - LOW-1: added `annotation_profile_and_capability_map_agree` — cross-checks that read-only-annotated tools map to `brain.read` and destructive-annotated tools map to `brain.purge`, so the Task 3.1 annotation map and Task 3.3 capability map cannot drift apart silently.
  - LOW-2: added `proposal_only_worker_is_denied_purge` + extended `admin_capability_alone_does_not_grant_confirm_or_purge` to cover the Purge arm (`wiki_spaces_remove`), closing the negative-test triad (Confirm + Purge + Admin).

## DoD verification

### Bullet 1 — TLS/Origin/audience + per-tool capabilities + negative tests
- 6 capabilities present (Read/Capture/Propose/Confirm/Purge/Admin); no broad `brain.write`.
- `AuthPolicy::default()` maps every declared tool (no gaps — `every_tool_has_a_required_capability`).
- Negative tests: proposal-only worker denied Confirm (`wiki_config`) and Purge (`wiki_spaces_remove`, `wiki_schema`); admin-only denied both too; no admin bypass.
- `AuthPolicyEntry` carries `requires_tls` + `allowed_origins` (DoD bullet 3). Audience/resource validation is deferred to Phase 6 deployment (real token termination).
- Cross-map consistency: `annotation_profile_and_capability_map_agree` locks the Task 3.1 ↔ 3.3 classification link.

### Bullet 2 — token never in URL/log/repo
- `TokenRedaction::redact` strips `Bearer `, `access_token=`, `token=` (conservatively — over-redaction is the safe direction for a security filter). Tests: `token_redaction_strips_bearer_tokens`, `token_redaction_strips_url_query_tokens`.
- `AuthPrincipal` carries only `id` (subject) + `capabilities` — never the raw token.

### Bullet 3 — every HTTP path has auth policy, no admin bypass
- `/mcp` covered with `requires: [Read]`, `requires_tls: true`. Every entry has non-empty `requires` (`auth_policy_covers_every_http_path`).
- No admin bypass: admin alone does not satisfy Confirm/Purge (capability-scoped, `allows` checks the exact required cap).

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test mcp_auth_boundary_v1`: 12/12 pass (after fix-round).
- `cargo test -j 2` (default): 700 passed / 0 failed.
- `cargo test --all-features -j 2`: 711 passed / 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing.
- Isolation: `semantic_vertical_slice` 14/14 pass; `grep -rn "semantic::" src/mcp/` returns nothing.

## Security review (Independent Validator, all clear)

- **`allows` is fail-closed**: unmapped tool → `None => false` (denied even to an all-capabilities principal).
- **No admin bypass**: Admin capability alone does not satisfy Confirm/Purge.
- **TM-012**: no `semantic::` import in `src/mcp/` — auth module is pure policy, no store coupling.
- **TokenRedaction over-redaction**: conservative (may redact non-secret `token=` substrings) — safe direction for a security filter.

## Carried risks / deferrals

- **Enforcement wiring (open, Phase 6)**: `AuthPolicy::allows` exists but is NOT yet wired into `McpServer::call_tool` dispatch — there is no real principal source yet. The HIGH carried risk moves from "no auth boundary exists" to "auth boundary framework exists but is not yet enforced on the hot path." Deployment (Phase 6) populates `AuthPrincipal` from a validated token and enforces `requires_tls`/`allowed_origins`/audience at the HTTP edge.
- **Deferred**: `brain_*` semantic wiring, audience/resource validation, real TLS/OAuth/Keycloak termination, external-client interop runs.
- **Inherited, unchanged**: schema break v1→v2; `cargo audit` 1 vuln + 3 warnings; eval `byte_lock_passed: false`.

## Phase 3 Gate — CLOSED

Task 3.1 (stable contracts) ✅ PASS. Task 3.2 (transport contracts) ✅ PASS. Task 3.3 (auth boundary) ✅ PASS. The Phase 3 Gate (GOAL-vNext §13: "interoperability + auth/security suites ผ่าน และ contract ถูก versioned") is satisfied at the contract level: the interop suite (`mcp_transport_v1`) and auth/security suite (`mcp_auth_boundary_v1`) both pass, and the contract is versioned via the capability enum + policy structure. **Phase 4 (AI Ingestion and Consolidation) is clear to start** pending its own Task Brief review. The per-request enforcement wiring remains the explicit Phase 6 deployment responsibility.
