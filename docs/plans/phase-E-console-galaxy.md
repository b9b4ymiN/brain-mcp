# Phase E — React Console + Galaxy Graph ⬜ pending

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> เป้าหมาย: ปิด **Phase 5 Gate** — Console เรียก application API จริง ไม่มี mock/TODO path + Galaxy 3D + browser/security/a11y/perf gates

## Decisions (2026-07-16)
- **มี Task E0 HTTP JSON/SSE API layer** (ตัวเลือกที่ดีที่สุด — Console ต้องการ session cookie/CSRF/browser auth ที่ MCP protocol ให้ไม่ได้; §9 = application service เดียวกับ MCP expose เป็น HTTPS; decouple UI จาก MCP evolution)

## Decisions (2026-07-17)
- **Frontend stack เปลี่ยนจาก React → Svelte 5 + Vite + TS strict.** เหตุผล: Console เป็น first-party single-user tool, bundle size ไม่ใช่คอขวดจริง — แต่ heap-growth gate (Task 5.2 DoD "heap โต ≤10% หลัง mount/filter 20 รอบ") เป็นความเสี่ยงสูงกว่าถ้าใช้ React wrapper รอบ WebGL (`react-force-graph-3d`) ที่คุม dispose lifecycle ของ three.js ไม่ตรง ถือ `3d-force-graph`/`force-graph` core ตรงๆ ใน Svelte component แล้วเรียก `_destructor()` เองใน `onDestroy` ควบคุมได้แม่นกว่า
- **Deps ใหม่ (ขออนุมัติก่อน install):** Svelte 5 + Vite + TS + `3d-force-graph` (3D renderer) + `force-graph` (2D fallback) + Playwright
- DoD ทุกข้อของ Task 5.1–5.3 เป็น framework-agnostic (grep gate, E2E behavior, benchmark numbers) — ไม่กระทบจาก stack เปลี่ยน

## Task E0 — Console HTTP JSON/SSE API layer ✅ CLOSED (`ebb7701`, `7784260`, `406befe`)
- HTTP endpoint เรียก application service เดียวกับ MCP (ไม่แตะ SQLite/Git/index ตรง) — reuse handler layer ของ brain_*
- Auth: session cookie (HttpOnly/SameSite) + CSRF token; dev = local bootstrap, production = OAuth (Phase F)
- SSE stream สำหรับ jobs/updates

**DoD:**
1. ✅ endpoint ครอบ read + review actions (search/get/timeline/evidence/inbox/approve/reject/supersede) — `src/api.rs`, 8 routes + login/logout + events
2. ✅ auth + CSRF negative tests ผ่าน (no-session → 401, bad-CSRF → 403) — `tests/api_console_v1.rs` (18 tests)
3. ✅ ไม่มี direct storage write; audit link ครบ — ทุก route เรียกผ่าน `SemanticStore` public methods เท่านั้น (grep-verified โดย Validator)

Sub-tasks: E0.1 (`ebb7701`, inbox/timeline/evidence accessors + owner-scoped confirm/reject/supersede ใน `semantic.rs`) · E0.2 (`7784260`, axum `/api/v1` router + session/CSRF) · E0.3 (`406befe`, SSE `/api/v1/events` + static console serve + CSP) — ทุก sub-task ผ่าน Independent Validator (agent แยก, read-only) ก่อน commit

## Task E1 — Console shell + review workflow (Home/Search/Inbox/Entity/Operations)
- Svelte 5 + Vite + TS; 5 หน้าเรียก E0 API จริง (mock-first ตอน dev แต่ integrate ก่อนปิด)
- Inbox: approve/reject/edit/supersede แสดง evidence + diff ก่อน commit; SafeText render (contract Task 5.1)

**DoD (§13 Task 5.1):**
1. 5 หน้าเรียก API จริง ไม่มี mock/TODO path (grep gate ใน production build)
2. approve/reject/edit/supersede แสดง evidence/diff ก่อน commit — Playwright E2E
3. loading/empty/error/permission states + keyboard nav — E2E pass
4. XSS/CSP: inject payload ใน claim/source → escaped, CSP block inline

## Task E2 — Galaxy 3D graph + LOD + fallback
- `3d-force-graph` (core, ไม่ใช่ React wrapper) หลัง `GraphRenderer` interface (contract Task 5.2); `force-graph` 2D/list fallback
- Server ส่ง bounded subgraph/ego network; semantic zoom Far/Mid/Close (≤300/≤2000/ego)
- Label/tooltip ผ่าน escaped textContent

**DoD (§13 Task 5.2 verbatim):**
1. cluster/zoom/click/focus/filter/expand ทำงาน; side panel = current claim/source/timeline/connections
2. edit/add จาก graph ผ่าน API + audit event
3. Benchmark (pinned Playwright Chromium, 1920×1080/DPR1, warm-up 10s, 5 runs, median/p95, baseline 4-core/8GB/iGPU): **1k ≥45 FPS, 5k ≥30 FPS, click p95 <100ms, search-to-focus <300ms**
4. cluster counts/aggregated edges = raw fixture 100%; heap โต ≤10% หลัง mount/filter 20 รอบ
5. no-WebGL/reduced-motion/keyboard/list-2D fallback ใช้งานได้
6. `bench/environment.json` บันทึก OS/browser/Playwright/GPU/CPU/RAM/seed

## Task E3 — Trust/ops views + UAT
- Contradictions/staleness/retrieval-trace/client-activity/jobs/evals/backup-health (contract Task 5.3)
- Entity merge/split + retract = preview/undo; hard purge = preview + recent re-auth + two-step nonce + คำเตือน irreversible
- UAT 3 โดเมน (หุ้น/โปรเจกต์/ความรู้)

**DoD (§13 Task 5.3):**
1. ตอบ "รู้อะไร/มาจากไหน/จริงเมื่อไร/เชื่อมอะไร/client ใดแก้" ได้จาก UI
2. destructive actions มี guard ครบ (preview/reauth/nonce/undo-where-reversible)
3. UAT 3 โดเมน pass; a11y (keyboard/contrast/screen-reader labels) pass

**Phase E Gate = Phase 5 Gate ปิด:** UAT + browser/security/a11y/perf gates ผ่าน → **Independent Validator PASS**
