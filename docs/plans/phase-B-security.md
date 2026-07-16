# Phase B — Security Hardening ✅ CLOSED 2026-07-16

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> Review: [`../baseline/review-phaseBC-20260716.md`](../baseline/review-phaseBC-20260716.md)
> ก่อน Phase C เสมอ — เปิด transport โดย auth ไม่ enforce = ช่องโหว่จริง (S1+S2)

สรุป: B1 (`d389e70`) + B2 (`261c4a8`) + B3 (`cbe3182`) ผู้ใช้ทำเอง; review พบ **F1 gap** (B3 gate เขียนแต่ `serve()` ไม่เปิด) → แก้ที่ **`a9cec81`** (F1 fix). S2 ปิดจริงแล้ว.
**ค้าง → ยกไป phase อื่น:** F2 (brain_* ใช้ owner context เสมอ) → [Phase D Task D2](phase-D-zai-adapter.md); F3 (search substring in-memory) → [Phase E](phase-E-console-galaxy.md)/retrieval.

## Task B1 — Loopback bind guard + dependency bumps (S1 + S4/S5) ✅ `d389e70`
- MCP HTTP bind default `0.0.0.0` → `127.0.0.1` (`src/server.rs`); non-loopback = explicit opt-in + คำเตือน
- `crossbeam-epoch` → 0.9.20 (ปิด RUSTSEC-2026-0204); anyhow/memmap2 bumped

**DoD:**
- [x] test พิสูจน์ default bind = loopback
- [x] `cargo audit` เหลือ 0 vuln (bincode warning = transitive, accepted)
- [x] full regression เขียว

## Task B2 — Eval contract re-lock (S3 / D7) ✅ `261c4a8`
- regenerate manifest hashes ของ 9 locked files; fix governance runner ให้ใช้ python 3.14.4

**DoD:**
- [x] eval runner exit 0 (byte_lock ผ่าน)
- [x] governance 10/10
- [x] rationale บันทึกใน commit

## Task B3 — Wire auth enforcement เข้า hot path (S2) ✅ `cbe3182` + F1 fix `a9cec81`
- `AuthPolicy::allows` enforce ใน `call_tool` dispatch
- **F1 fix:** `serve()` เรียก `with_auth_policy(AuthPolicy::default(), owner_principal())`; extract `check_capability`; 3 dispatch-level tests

**DoD:**
- [x] negative tests (restricted principal เรียก capture/confirm/purge) fail ที่ **dispatch จริง** ไม่ใช่แค่ policy unit test
- [x] owner path ไม่พัง (brain E2E stdio 2/2)
- [x] full regression เขียว

**Phase B Gate ✅:** S1+S2+S3 ปิด; audit 0 vuln; F1 แก้แล้ว
