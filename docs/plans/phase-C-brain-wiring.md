# Phase C — `brain_*` Semantic Wiring ✅ CLOSED 2026-07-16

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> Review: [`../baseline/review-phaseBC-20260716.md`](../baseline/review-phaseBC-20260716.md)
> เป้าหมาย: contract กลายเป็นระบบจริง — Claude Code เรียก `brain_search` ได้จริง

สรุป: C1 (`dec92a3`) + C2 (`208fed1`) + C3 (`a544d5f`) ผู้ใช้ทำเอง. brain_* ใช้ได้จริงจาก Claude Code ผ่าน stdio (E2E 2/2).
**หมายเหตุ scope:** read tools ที่ wire จริง = status/search/get (timeline/evidence/explain ยังไม่ wire → ยกไป Phase E retrieval); ingest_source/merge/split/purge ยกไป Phase D/E ตามที่ต้องใช้.

## Task C1 — SemanticStore wiring + read tools ✅ `dec92a3`
- `McpServer` ถือ `semantic_store: Option<Arc<SemanticStore>>`; wire `brain_status`/`brain_search`/`brain_get`
- **DoD:** [x] read tools dispatch เข้า SemanticStore จริง; structured + text; schema tests ผ่าน (brain_tools_v1 pass)

## Task C2 — Mutation tools ✅ `208fed1`
- `brain_capture`/`brain_confirm`/`brain_supersede` → propose_user_assertion/confirm/supersede; operation_id required
- **DoD:** [x] capture→confirm→supersede cycle ผ่าน MCP layer จริง; write_additive annotations; capability map ครบ

## Task C3 — Client interop (stdio) ✅ `a544d5f`
- `serve()` auto-creates SemanticStore ที่ `state_dir/semantic-store`; isolation allowlist +server.rs
- **DoD:** [x] E2E ผ่าน real stdio MCP: brain_status + capture→confirm→search→get round-trip (2/2)

**Phase C Gate ✅:** brain_* end-to-end จาก Claude Code ผ่าน stdio จริง
**ค้าง (ยกไป Phase D/E):** timeline/evidence/explain wiring · hybrid/time-aware search (F3) · worker privilege separation (F2)
