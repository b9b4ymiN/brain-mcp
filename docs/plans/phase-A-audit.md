# Phase A — Verification Audit ✅ CLOSED 2026-07-16

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> ผลลัพธ์: [`../baseline/audit-20260716-report.md`](../baseline/audit-20260716-report.md) — Independent Validator **PASS 10/10 checks, 0 corrections**
> เป้าหมาย: health report ที่พิสูจน์ด้วยการรันจริง ไม่เชื่อตัวเลขใน report เก่า

ข้อค้นพบหลัก: ตัวเลขใน evidence packets เดิมตรงความจริงทั้งหมด; พบ environment divergence 2 (GNU toolchain resolution — แก้ด้วย rustup override, CARGO_TARGET_DIR redirect — stale binary ถูกลบ); **S1 HIGH ใหม่: MCP HTTP bind `0.0.0.0` ไม่มี auth (server.rs:85)**

## Task A1 — Repo hygiene + toolchain baseline ✅
1. ตรวจ untracked files 4 รายการ (D8) → disposition
2. เทียบ toolchain กับ `../baseline/repository-environment.json`
3. disposition: commit / gitignore / ลบ (ลบต้องถามก่อน)

**DoD:**
- [x] ทุก untracked file มี disposition ที่ผู้ใช้เห็นชอบ; working tree สะอาด (`3a21daf`)
- [x] toolchain ตรง baseline หรือ divergence บันทึกพร้อมผลกระทบ (GNU/MSVC → rustup override)

## Task A2 — รัน gate ทั้งหมดซ้ำ เทียบตัวเลขที่ report อ้าง ✅
Commands: `cargo fmt --check` · `cargo clippy --all-targets [--all-features] -- -D warnings` · `cargo test -j 2` (default + `--all-features`) · eval v1 · Python engine/mcp/acp · governance · `cargo audit`

**DoD:**
- [x] ตารางเทียบ reported vs actual ครบทุก gate พร้อม exit code
- [x] discrepancy จำแนก product / environment / stale-doc
- [x] CVE ระบุจริง: crossbeam-epoch 0.9.18 RUSTSEC-2026-0204 (fix ≥0.9.20)

## Task A3 — Gap matrix + Security risk report ✅
1. Map §11 System DoD ทุก bullet → done/contract-only/missing
2. Map deferred D1–D8 → confirm/update
3. Security review: per-handle HIGH, auth hot path, HTTP bind, secret handling
4. risk-ranked backlog → ผู้ใช้เลือกลำดับ Phase B–F

**DoD:**
- [x] gap matrix ครบทุก §11 bullet
- [x] security report แยก CRITICAL/HIGH/MEDIUM/LOW พร้อม evidence path
- [x] risk-ranked backlog ที่ผู้ใช้ review แล้ว

**Phase A Gate ✅:** A1–A3 DoD ครบ + Validator PASS 10/10 + ผู้ใช้ยืนยันลำดับ Phase B–F
