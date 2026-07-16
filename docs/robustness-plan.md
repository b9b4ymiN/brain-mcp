# Robustness Plan — Audit-First Gap Closure

> สถานะ: **APPROVED — PHASE A IN PROGRESS**
> วันที่อนุมัติ: 2026-07-16
> Base: branch `vnext/phase-0` @ `549c090` (Phase 7 Gate closed — ALL PHASES contract-cleared)
> Master spec: `C:\Programing\AI2.0\jarvis\brain_2nd\GOAL-vNext.md` (§11 System DoD, §12 Engineering Loop, §13 Phase DoD)
> ทิศทางที่ผู้ใช้เลือก: **Audit ก่อน → ปิด gap ตามลำดับความเสี่ยง**
> Environment ที่อนุญาต: Rust local + Docker + Browser/Playwright + external network (**ยิง Z.ai ต้องถามก่อนทุกครั้ง**)

---

## 1. บริบท — ทำไมต้องมีแผนนี้

ทุก Phase ของ GOAL-vNext (-1 ถึง 7) ผ่าน Independent Validator แล้ว แต่ Phase 3–7 ผ่านแบบ
**contract-level** (มี Rust types + state machines + tests แต่ยังไม่ wire เข้าระบบจริง)
งานที่ evidence packets ระบุว่าเปิดค้าง:

| # | Deferred item | มาจาก | ความเสี่ยง |
|---|---|---|---|
| D1 | `brain_*` MCP tools ยังไม่ wire เข้า `SemanticStore` — MCP surface ยังเป็น wiki tools เดิม | Task 3.1/3.2 | ระบบยังใช้เป็น Second Brain จริงไม่ได้ |
| D2 | Auth enforcement ไม่อยู่บน hot path (`AuthPolicy::allows` framework-only) + per-handle capability **HIGH** finding ค้างจาก Phase 1 | Task 1.3/3.3 | ช่องโหว่จริงทันทีที่เปิด transport |
| D3 | Z.ai HTTP adapter จริงยังไม่มี (มีแค่ trait + OutboundPolicy) + §11 adversarial corpus ยังไม่รันกับ adapter จริง | Task 4.1/4.2 | AI ingestion ยังไม่เกิด |
| D4 | React Console + Galaxy 3D + Playwright/browser gates — **Phase 5 Gate ยังเปิด** | Task 5.1–5.3 | ไม่มี UI จริง |
| D5 | `docker compose up` จริง + clean-host restore drill จริง + external security review — **Phase 6 production-closure ยังไม่ปิด** | Task 6.1–6.3 | deploy production ไม่ได้ |
| D6 | `cargo audit` 1 vuln + 3 warnings (เปิดตั้งแต่ Task 0.1) | baseline | ยังไม่รู้ severity จริง |
| D7 | governance 3 fail + eval `byte_lock_passed: false` (9 files mismatched ก่อน Task 2.2) | baseline | eval contract integrity |
| D8 | untracked files ใน working tree: `tests/fixtures/wikis/alt-root/schemas/*.json` ×3 + `.zcode/` | git status | fresh clone อาจ test fail |

---

## 2. กติกาการทำงาน (ทุก Phase)

- ทำ **ทีละ Task** — ไม่มีหลาย implementation Task พร้อมกัน (§12.1)
- ทุก Task: Builder ทำ → **Independent Validator agent (session แยก, read-only)** ตรวจ DoD → PASS ก่อนปิด
- Task ที่แก้ code ใช้ TDD: RED checkpoint → GREEN → refactor, coverage ≥80% ของ code ที่เปลี่ยน
- Error ทุกตัวใช้เป็น feedback แก้ซ้ำจนผ่าน — ห้ามข้าม gate
- เจอ security risk → **แจ้งผู้ใช้ทันที** ไม่รอปิด Task
- จบแต่ละ Task → สรุปรายงาน: ทำอะไร + ผลลัพธ์ + ที่ยังไม่ได้ทำ
- Windows quirk: `cargo test --all-features` ต้องใช้ `-j 2` (pagefile exhaustion — environment ไม่ใช่ code defect)

---

## 3. Phase A — Verification Audit ✅ CLOSED 2026-07-16

> ผลลัพธ์: `docs/baseline/audit-20260716-report.md` — Independent Validator **PASS 10/10 checks, 0 corrections**
> ข้อค้นพบหลัก: ตัวเลขใน evidence packets เดิมตรงความจริงทั้งหมด; พบ environment divergence 2 (GNU toolchain resolution — แก้ด้วย rustup override, CARGO_TARGET_DIR redirect — stale binary ถูกลบ); **S1 HIGH ใหม่: MCP HTTP bind `0.0.0.0` ไม่มี auth (server.rs:85)** — หักล้างสมมติฐานเดิมใน §9 ของแผนนี้ที่ว่า config เป็น loopback-only

> เป้าหมาย: ได้ health report ที่พิสูจน์ด้วยการรันจริง ไม่ใช่เชื่อตัวเลขใน report เก่า

### Task A1 — Repo hygiene + toolchain baseline
สถานะ: ⏳ in progress

ขั้นตอน:
1. ตรวจ untracked files ทั้ง 4 รายการ (D8) — เป็นของจำเป็นที่ลืม commit, ของหลงเหลือ, หรือของ session tool
2. เทียบ toolchain ปัจจุบันกับ `docs/baseline/repository-environment.json`
3. เสนอ disposition ต่อไฟล์: commit / gitignore / ลบ (**ลบต้องถามผู้ใช้ก่อน**)

**DoD:**
- [ ] ทุก untracked file มี disposition ที่ผู้ใช้เห็นชอบ และ working tree สะอาดหรืออธิบายได้ครบ
- [ ] toolchain ตรง baseline หรือ divergence ถูกบันทึกพร้อมผลกระทบ

### Task A2 — รัน gate ทั้งหมดซ้ำ เทียบตัวเลขที่ report อ้าง
สถานะ: ⬜ pending

Commands (ตามลำดับ):
1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings` (และรอบ `--all-features`)
3. `cargo test -j 2` (default features)
4. `cargo test --all-features -j 2`
5. eval v1: `python evals/v1/run.py` (คาด 126/126, ตรวจ byte_lock)
6. Python integration suites (engine/mcp/acp — คาด 10/63/76/26+2skip)
7. governance suite (คาด 3 fail pre-existing — ยืนยันว่าไม่งอกใหม่)
8. `cargo audit` (คาด 1 vuln + 3 warnings — ระบุ CVE + severity จริง)

**DoD:**
- [ ] ตารางเทียบ reported vs actual ครบทุก gate พร้อม exit code
- [ ] discrepancy ทุกตัวจำแนกเป็น product defect / environment / stale-doc
- [ ] CVE ใน D6 ถูกระบุชื่อ + severity + affected path จริง

### Task A3 — Gap matrix + Security risk report
สถานะ: ⬜ pending

ขั้นตอน:
1. Map §11 System-level DoD ทุก bullet → สถานะจริง: `done` / `contract-only` / `missing` พร้อมหลักฐาน
2. Map deferred items D1–D8 → confirm/update จากผล A1+A2
3. Security-focused review: per-handle HIGH, auth hot path, HTTP bind policy ปัจจุบัน, secret handling
4. จัดลำดับ backlog ตามความเสี่ยง → เสนอผู้ใช้ตัดสินลำดับ Phase B–F

**DoD:**
- [ ] gap matrix ครบทุก §11 bullet ไม่มีข้อไหนไม่มีสถานะ
- [ ] security risk report แยกระดับ CRITICAL/HIGH/MEDIUM/LOW พร้อม evidence path
- [ ] risk-ranked backlog ที่ผู้ใช้ review และเลือกลำดับ Phase ถัดไปแล้ว

**Phase A Gate:** A1–A3 DoD ครบ + health report ส่งผู้ใช้ + ผู้ใช้ยืนยันลำดับ Phase B–F

---

## 4. Phase B — Security Hardening ⬜ pending (ลำดับปรับตามผล audit)

> ก่อน Phase C เสมอ — เปิด transport โดย auth ไม่ enforce = ช่องโหว่จริง (S1+S2)

### Task B1 — Quick wins: loopback bind guard + dependency bumps (S1 + S4/S5)
- แก้ MCP HTTP bind default `0.0.0.0` → `127.0.0.1` (`src/server.rs:85`); non-loopback ต้อง explicit opt-in + คำเตือน
- `cargo update -p crossbeam-epoch` ปิด RUSTSEC-2026-0204; bump anyhow/memmap2 ถ้าเวอร์ชัน fix มี
- **DoD:** test พิสูจน์ default bind = loopback; `cargo audit` เหลือ 0 vuln (warnings มี written acceptance); full regression เขียว

### Task B2 — Eval contract re-lock (S3 / D7)
- ตรวจ git history ว่าการแก้ 9 locked files หลัง Task 0.2 ผ่าน validator จริง → regenerate manifest hashes → governance เขียว
- **DoD:** eval runner exit 0 (byte_lock ผ่าน); governance 10/10; rationale บันทึกใน decision history

### Task B3 — Wire auth enforcement เข้า hot path (S2 / D2)
- `AuthPolicy::allows(principal, tool)` enforce จริงใน `call_tool` dispatch
- ปิด per-handle capability HIGH: code ที่ถือ `&SemanticStore` ต้องไม่ bypass capability check ได้
- **DoD:** negative tests (proposal-only worker เรียก confirm/purge/admin) fail ที่ dispatch จริง ไม่ใช่แค่ policy unit test; per-handle exploit path ปิดพร้อม test พิสูจน์; full regression เขียว

**Phase B Gate:** security reviewer agent (session แยก) ไม่มี CRITICAL/HIGH unresolved

---

## 5. Phase C — `brain_*` Semantic Wiring ⬜ pending

> ทำให้ contract กลายเป็นระบบจริง: Claude Code เรียก `brain_search` ได้จริง (D1)

### Task C1 — Engine owns SemanticStore + read tools
- `WikiEngine` (หรือ composition layer) ถือ `SemanticStore`; wire `brain_search`, `brain_get`, `brain_get_timeline`, `brain_get_evidence`, `brain_explain`, `brain_status`
- **DoD:** read tools ครบ 6 ตัวตาม §7.2 พร้อม structured content + text fallback + pagination + bounded output; ผ่าน schema tests

### Task C2 — Mutation tools + capability enforcement
- `brain_capture`, `brain_propose`, `brain_confirm`, `brain_supersede`, `brain_ingest_source`, `brain_merge_entity`, `brain_split_entity`, `brain_purge`
- ทุก mutation: `operation_id` + capability check (จาก Phase B) + audit link
- **DoD:** idempotency/retry tests ผ่านผ่าน MCP layer จริง; `brain_purge` มี recent re-auth + two-step nonce; §13 Task 3.1 mutation DoD ปิดจริง

### Task C3 — Client interop verification
- stdio + Streamable HTTP (loopback) กับ Claude Code + MCP Inspector จริง
- **DoD:** §13 Task 3.2 DoD ปิดด้วย runtime evidence: read/write/as-of/needs-input scenarios ผ่านจาก client จริงอย่างน้อย 2 ตัว; disconnect/retry ไม่ duplicate mutation

**Phase C Gate:** GULF temporal scenario (§11) ตอบถูกจาก Claude Code จริง end-to-end

---

## 6. Phase D — Z.ai Adapter จริง ⬜ pending

> ยิง network ไป api.z.ai ต้องถามผู้ใช้ก่อนทุกครั้ง; API key ผ่าน secret manager/env เท่านั้น ห้ามลง repo

### Task D1 — Concrete HTTP adapter
- implement `AiProvider` สำหรับ Z.ai OpenAI-compatible endpoint หลัง `ProviderConfig` + kill switch
- **DoD:** timeout/quota/429/5xx/invalid-JSON/partial-stream tests ผ่านกับ mock server; live smoke test (ขออนุมัติก่อน) ผ่าน

### Task D2 — Extraction round-trip + adversarial corpus
- source → spans → typed proposals กับ adapter จริง; รัน §11 adversarial corpus (30 cases)
- **DoD:** prompt-injection corpus ไม่ทำให้ worker ข้าม policy; local-only/secret negative corpus ผ่าน 100% ทุก path (normal/repair/retry/failure); intercepted outbound ยืนยันไม่มี secret egress

**Phase D Gate:** §13 Phase 4 Gate ปิดแบบ runtime (ไม่ใช่ contract-only) + compliance record ครบ

---

## 7. Phase E — Console + Galaxy จริง ⬜ pending

### Task E1 — React Console shell + review workflow (Home/Search/Inbox/Entity/Operations)
- **DoD:** §13 Task 5.1 DoD เดิมทั้งหมด — API จริงไม่มี mock path, E2E states, XSS/CSP tests ผ่าน
### Task E2 — Galaxy 3D + LOD + fallback
- **DoD:** §13 Task 5.2 DoD เดิม — benchmark fixtures 1k/5k/20k, FPS/latency targets, no-WebGL fallback
### Task E3 — Trust/ops views + UAT
- **DoD:** §13 Task 5.3 DoD เดิม + **Phase 5 Gate ปิด**: UAT หุ้น/โปรเจกต์/ความรู้ + browser/security/a11y/perf gates

---

## 8. Phase F — Production Closure ⬜ pending

### Task F1 — `docker compose up` จริงบน clean host (amd64; arm64 ตาม availability)
- **DoD:** §13 Task 6.1 DoD — smoke/health/MCP/Console checks ผ่าน; secrets ไม่ bake ใน image/repo
### Task F2 — Clean-host restore drill จริง + upgrade/rollback rehearsal
- **DoD:** §13 Task 6.3 DoD — restore drill ผ่านพร้อม composite checksum + purge registry sync fail-closed; RPO/RTO บันทึก
### Task F3 — External security review
- **DoD:** **Phase 6 Gate ปิด**: ไม่มี critical/high unresolved

---

## 9. Risk Register (แจ้งผู้ใช้แล้ว ณ วันอนุมัติแผน)

| Risk | ระดับ | Mitigation ในแผน |
|---|---|---|
| per-handle capability + auth ไม่ enforce บน hot path | **HIGH** | Phase B ก่อน Phase C เสมอ — ห้ามเปิด transport ก่อน B2 ปิด |
| `cargo audit` vuln ยังไม่รู้ severity | ? → รู้ที่ A2 | ถ้า CRITICAL/HIGH บน attack path จริง → เลื่อน B1 ขึ้นทันที |
| MCP HTTP bind `0.0.0.0` ไม่มี auth (**ยืนยันแล้วโดย A3** — server.rs:85, Host-header allowlist อย่างเดียวซึ่ง spoof ได้) | **HIGH (S1)** | ห้ามรัน `serve --http` บนเครื่องที่ expose ต่อ network จนกว่า B1 ปิด; B1 = loopback default |
| eval byte_lock mismatch → eval ที่รันอาจไม่ใช่ contract ที่ hash-lock ไว้ | MEDIUM | B3 ก่อนใช้ eval เป็น promotion gate ใน Phase D+ |
| Z.ai Coding Plan terms risk (GOAL §2.3) | KNOWN | compliance record + kill switch + ถามก่อนยิงทุกครั้ง |

---

## 10. Progress Log

| วันที่ | เหตุการณ์ |
|---|---|
| 2026-07-16 | แผนอนุมัติ; เริ่ม Phase A Task A1 |
| 2026-07-16 | A1 ปิด: hygiene commit `3a21daf` (schema fixtures committed, `.zcode/` ignored); toolchain GNU/MSVC divergence พบ+แก้ (rustup override); CARGO_TARGET_DIR footgun documented |
| 2026-07-16 | A2 ปิด: gates ทั้งหมดรันซ้ำ — ตรง report เดิมทุกตัว (~803/~810 tests 0 fail, python 63/76/26+2s, eval cases 126/126 แต่ gate exit 1 จาก byte_lock, governance 3 fail root cause เดียวกัน, audit vuln = crossbeam-epoch RUSTSEC-2026-0204) |
| 2026-07-16 | A3 ปิด + **Phase A Gate: Independent Validator PASS 10/10** — report: `docs/baseline/audit-20260716-report.md`; S1 HIGH ใหม่ (0.0.0.0 bind ไม่มี auth); Phase B re-ordered เป็น B1 bind+deps → B2 eval re-lock → B3 auth wiring |
| 2026-07-16 | **ผู้ใช้ทำ B1–B2–C1–C2–C3 เอง** (commits `d389e70`..`a544d5f`) |
| 2026-07-16 | **Review Phase B+C** (`docs/baseline/review-phaseBC-20260716.md`): B1/B2/C1/C2/C3 PASS (verified re-run: audit 0 vuln, eval exit 0, governance 10/10, brain E2E 2/2, regression 0 fail) — **แต่ F1 HIGH: B3 auth gate เขียนแล้วแต่ `serve()` ไม่เปิดใช้ (`with_auth` ไม่ถูกเรียกที่ไหนเลย) → S2 ยังไม่ปิดจริง**. ต้อง reopen B3 |
