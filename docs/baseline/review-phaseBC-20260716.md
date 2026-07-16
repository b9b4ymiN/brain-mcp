# Independent Review — Phase B + C (user-built)

- Date: 2026-07-16
- Reviewer: Claude (read-only verification of user's commits `d389e70`..`a544d5f`)
- Base verified at: working tree on `vnext/phase-0` (D1 RED `babda7d` present on top; provider.rs/zai_adapter_v1.rs have uncommitted D1 work — out of scope here)

## Verdict: **MOSTLY PASS — 1 material gap (F1) in Task B3**

B1, B2, C1, C2, C3 verified working. Task **B3's stated goal (close S2 — auth enforced on the hot path) is not actually met**: the enforcement code exists but is never activated on the real `serve` path.

---

## What was verified PASS (re-ran, not trusted from commit messages)

| Task | Claim | Verification | Result |
|---|---|---|---|
| B1 | bind default `127.0.0.1`, opt-in for `0.0.0.0` + warning | read `src/server.rs:85-101`, `src/config.rs` (`http_bind_address`/`http_bind_all_interfaces`) | ✅ correct, invalid-addr falls back to loopback |
| B1 | audit 0 vuln | `cargo audit` → 0 vulnerabilities, 1 warning (bincode unmaintained, transitive) | ✅ RUSTSEC-2026-0204 closed |
| B2 | eval byte_lock passes, exit 0 | ran locked runner → `EVAL_EXIT=0` | ✅ |
| B2 | governance 10/10 | `pytest governance/ -q` → 10 passed | ✅ (was 3 fail) |
| C1 | brain_status/search/get dispatch to real SemanticStore | `tests/brain_tools_v1.rs` 10 pass; handlers read `all_claims_current` | ✅ |
| C2 | brain_capture/confirm/supersede mutation tools, operation_id required | handlers.rs:901+ require `operation_id`; write_additive annotations | ✅ |
| C3 | `serve` auto-creates SemanticStore; E2E via real stdio | `mcp/test_brain_tools.py` 2/2 pass on fresh binary | ✅ |
| all | no regression | `cargo test -j 2` default → exit 0, 0 failed | ✅ |

Isolation allowlist extension to `server.rs` (C3) is legitimate — second authorized bridge after `projection.rs`, matches the Task 2.1 precedent.

---

## Findings

### F1 — HIGH — ✅ RESOLVED at `a9cec81` (auth enforcement wired but never activated)

**Fix:** `serve()` now calls `with_auth_policy(AuthPolicy::default(), owner_principal())` — the gate is active on the hot path. The inline check was extracted into `McpServer::check_capability`, and 3 dispatch-level tests in `tests/mcp.rs` drive that exact method: a `Read`-only principal is denied `brain_capture`/`brain_confirm`/`wiki_spaces_remove` but allowed `brain_search`; legacy (no policy) allows all; owner principal allows all (serve path unbroken, confirmed by brain_tools E2E 2/2). S2 closed. Original finding below for the record.


`McpServer::with_auth(manager, policy, principal)` exists (`src/mcp/mod.rs`) and `call_tool` does gate on `policy.allows(...)` **when `auth_policy` is `Some`**. But:

- `grep with_auth src/ tests/` → **only the definition; zero call sites.**
- `serve()` builds the server via `McpServer::new()` / `with_web_refresh()` → both set `auth_policy: None`.
- With `None`, the gate is skipped entirely.

Net effect: on the real serve path, every tool — including the new `brain_capture`/`brain_confirm`/`brain_supersede` mutations from C2 — is reachable with **no capability check**. The `mcp_auth_boundary_v1.rs` negative tests only call `policy.allows()` directly (unit level); they never drive `McpServer::call_tool`, so they pass without proving runtime enforcement.

B3's DoD ("negative tests fail ที่ dispatch จริง ไม่ใช่แค่ policy unit test") is therefore **not satisfied**. S2 remains open.

**Severity in practice:** LOW today because B1 makes the default bind loopback-only, so it's not remotely reachable. But the stated Phase B security goal is incomplete, and it becomes real the moment `http_bind_all_interfaces` is set or a worker identity is introduced (Phase D).

**Fix:** have `serve()` call `with_auth()` with a real principal (bootstrap for local stdio is fine, but the gate must be *on*), and add one test that drives `McpServer::call_tool` with a limited principal and asserts a capability-denied error.

### F2 — MEDIUM (privilege separation not end-to-end)
All `brain_*` handlers use `store.trusted_context()` → the fully-trusted owner identity, regardless of the MCP principal. Acceptable for single-owner local use, but the core GOAL §7.1 rule "proposal-only worker cannot confirm/purge" is not enforceable yet: a worker calling `brain_confirm` would currently succeed. Must be closed before Phase D wires a Z.ai worker identity. Ties into F1 (the MCP principal isn't consulted at all).

### F3 — LOW (search is not the §7.2 contract yet)
`brain_search`/`brain_get` load `all_claims_current` into memory and substring-filter in Rust (handlers.rs:816, 863) — not Tantivy, not time-aware, output-bounded by `top_k` only. Fine as a Phase C stepping stone; note that "hybrid/time-aware search" and unbounded in-memory claim load remain for later.

### F4 — LOW (tree not clean)
`src/provider.rs` + `tests/zai_adapter_v1.rs` have uncommitted D1 work on top of the D1 RED checkpoint. Not a B/C defect; flagged so it isn't lost.

---

## Recommendation
B1, B2, C1–C3 are solid and the brain_* tools genuinely work end-to-end. Before calling Phase B "closed", **reopen B3** to activate the auth gate on the serve path (F1) and add the real-dispatch negative test. F2 can be folded into that or scheduled explicitly for Phase D. F3 is a noted deferral.
