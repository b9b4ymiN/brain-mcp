# Robustness Plan — Index

> สถานะ: **Phase A/B/C CLOSED · Phase D = NEXT**
> Base: branch `vnext/phase-0` (Phase 7 Gate closed — ALL PHASES contract-cleared)
> Master spec: `C:\Programing\AI2.0\jarvis\brain_2nd\GOAL-vNext.md` (§11 System DoD, §12 Engineering Loop, §13 Phase DoD)
> ทิศทาง: **Audit ก่อน → ปิด gap ตามลำดับความเสี่ยง**
> Environment: Rust local + Docker + Browser/Playwright + external network (**ยิง Z.ai ต้องถามก่อนทุกครั้ง**)

รายละเอียดแต่ละ Phase แยกเป็นไฟล์ใน [`plans/`](plans/) — ไฟล์นี้เป็น index + บริบท/กติกา/risk/progress ที่ใช้ร่วมกัน

## Phases

| Phase | ไฟล์ | สถานะ | Gate |
|---|---|---|---|
| A — Verification Audit | [`plans/phase-A-audit.md`](plans/phase-A-audit.md) | ✅ CLOSED | Validator PASS 10/10 |
| B — Security Hardening | [`plans/phase-B-security.md`](plans/phase-B-security.md) | ✅ CLOSED | S1+S2+S3 ปิด, audit 0 vuln |
| C — `brain_*` Semantic Wiring | [`plans/phase-C-brain-wiring.md`](plans/phase-C-brain-wiring.md) | ✅ CLOSED | brain_* E2E จาก Claude Code |
| D — Z.ai Adapter จริง | [`plans/phase-D-zai-adapter.md`](plans/phase-D-zai-adapter.md) | ⬜ **NEXT** | Phase 4 Gate runtime |
| E — React Console + Galaxy | [`plans/phase-E-console-galaxy.md`](plans/phase-E-console-galaxy.md) | ⬜ pending | Phase 5 Gate |
| F — Production (Docker) | [`plans/phase-F-production.md`](plans/phase-F-production.md) | ⬜ pending | Phase 6 Gate |

Evidence: [`baseline/audit-20260716-report.md`](baseline/audit-20260716-report.md) · [`baseline/review-phaseBC-20260716.md`](baseline/review-phaseBC-20260716.md) · [`final-report-20260716.md`](final-report-20260716.md)

---

## 1. บริบท — ทำไมต้องมีแผนนี้

ทุก Phase ของ GOAL-vNext (-1 ถึง 7) ผ่าน Independent Validator แล้ว แต่ Phase 3–7 ผ่านแบบ **contract-level** (มี Rust types + tests แต่ยังไม่ wire เข้าระบบจริง). Deferred items ที่เป็นที่มาของแผนนี้:

| # | Deferred item | มาจาก | สถานะ |
|---|---|---|---|
| D1 | `brain_*` MCP tools ยังไม่ wire เข้า `SemanticStore` | Task 3.1/3.2 | ✅ ปิด Phase C (บางส่วน; timeline/evidence/explain ยกไป E) |
| D2 | Auth ไม่อยู่บน hot path + per-handle HIGH | Task 1.3/3.3 | ✅ ปิด Phase B (F1 fix `a9cec81`) |
| D3 | Z.ai HTTP adapter จริง + adversarial corpus | Task 4.1/4.2 | ⬜ Phase D |
| D4 | React Console + Galaxy + Playwright gates | Task 5.1–5.3 | ⬜ Phase E |
| D5 | `docker compose up` + restore drill + external review | Task 6.1–6.3 | ⬜ Phase F |
| D6 | `cargo audit` vuln | baseline | ✅ ปิด Phase B (crossbeam-epoch 0.9.20) |
| D7 | governance fail + eval byte_lock | baseline | ✅ ปิด Phase B |
| D8 | untracked fixtures + `.zcode/` | git status | ✅ ปิด Phase A (`3a21daf`) |

---

## 2. กติกาการทำงาน (ทุก Phase)

- ทำ **ทีละ Task** — ไม่มีหลาย implementation Task พร้อมกัน (§12.1)
- ทุก Task: Builder ทำ → **Independent Validator agent (session แยก, read-only)** ตรวจ DoD → PASS ก่อนปิด
- Task ที่แก้ code ใช้ TDD: RED checkpoint → GREEN → refactor, coverage ≥80% ของ code ที่เปลี่ยน
- Error ทุกตัวใช้เป็น feedback แก้ซ้ำจนผ่าน — ห้ามข้าม gate
- เจอ security risk → **แจ้งผู้ใช้ทันที** ไม่รอปิด Task
- จบแต่ละ Task → สรุปรายงาน: ทำอะไร + ผลลัพธ์ + ที่ยังไม่ได้ทำ
- Windows quirk: `cargo test --all-features` ต้องใช้ `-j 2` (pagefile exhaustion); repo ต้อง `rustup override 1.95-x86_64-pc-windows-msvc`

---

## 3. Risk Register

| Risk | ระดับ | สถานะ |
|---|---|---|
| per-handle capability + auth ไม่ enforce บน hot path | HIGH | ✅ ปิด Phase B (F1 fix) |
| MCP HTTP bind `0.0.0.0` ไม่มี auth (S1) | HIGH | ✅ ปิด Phase B (loopback default) |
| `cargo audit` vuln (crossbeam-epoch RUSTSEC-2026-0204) | LOW-MED | ✅ ปิด (0.9.20) |
| eval byte_lock mismatch | MEDIUM | ✅ ปิด Phase B |
| brain_* worker privilege separation ยังไม่ end-to-end (F2) | MEDIUM | ⬜ Phase D Task D2 |
| brain_search substring in-memory ไม่ใช่ hybrid/time-aware (F3) | LOW | ⬜ Phase E/retrieval |
| Z.ai Coding Plan terms risk (GOAL §2.3) | KNOWN | Phase D: compliance record + kill switch + ถามก่อนยิง |

---

## 4. Progress Log

| วันที่ | เหตุการณ์ |
|---|---|
| 2026-07-16 | แผนอนุมัติ; Phase A เริ่ม |
| 2026-07-16 | **Phase A CLOSED** — Validator PASS 10/10; hygiene `3a21daf`; S1 HIGH ใหม่พบ |
| 2026-07-16 | ผู้ใช้ทำ B1–B2–C1–C2–C3 เอง (`d389e70`..`a544d5f`) |
| 2026-07-16 | **Review B+C** — B1/B2/C1/C2/C3 PASS แต่พบ F1 HIGH (B3 gate ไม่เปิดที่ serve) |
| 2026-07-16 | **F1 fix `a9cec81`** — serve เปิด auth gate + dispatch-level tests; **Phase B/C CLOSED** |
| 2026-07-16 | **Phase D/E/F detailed plan** + 3 decisions ล็อค (no-mock / E0 API layer / docker multi-arch) |
| 2026-07-16 | **แยกแผนเป็นไฟล์ต่อ Phase** ใน `plans/`; ไฟล์นี้เป็น index |
