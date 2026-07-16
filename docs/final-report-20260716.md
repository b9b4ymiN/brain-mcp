# Final Report & Forward Plan — brain-mcp-vnext

> วันที่: 2026-07-16
> Base: `vnext/phase-0` @ `6f7cba7`
> เอกสารประกอบ: `docs/robustness-plan.md` (living plan) · `docs/baseline/audit-20260716-report.md` (audit evidence, Validator PASS 10/10)
> Master spec: `brain_2nd/GOAL-vNext.md`

---

## 1. Executive Summary

**โปรเจคอยู่ในสภาพดีมากในระดับ core แต่ยังใช้งานจริงไม่ได้** — ทุก Phase ของ GOAL-vNext (-1 ถึง 7)
ผ่าน Independent Validator แล้ว และการ audit รอบนี้ (รัน gate ทั้งหมดซ้ำจากศูนย์) ยืนยันว่า
**ตัวเลขใน evidence packets เดิมตรงความจริงทั้งหมด ไม่มี fabrication**:

- ~810 Rust tests / 0 fail (default + all-features)
- Python integration 63/76/26+2skip ตรง baseline เป๊ะ
- Eval 126/126 cases, recall@10 = 1.0, nDCG@10 = 0.973, hard invariants 100%

แต่ Phase 3–7 ผ่านแบบ **contract-level** — มี types + state machines + tests ครบ
แต่ยังไม่ wire เข้าระบบจริง สิ่งที่ระบบ**ยังทำไม่ได้วันนี้**:

1. ❌ Claude Code ยังเรียก `brain_search` / `brain_capture` ไม่ได้ (MCP surface ยังเป็น wiki tools เดิม)
2. ❌ ไม่มี AI ingestion จริง (Z.ai adapter เป็นแค่ trait)
3. ❌ ไม่มี Console/Galaxy UI จริง (เป็นแค่ contract types ใน Rust)
4. ❌ ยัง deploy production ไม่ได้ (ไม่มี Docker image/compose จริง, ไม่เคย restore drill จริง)
5. ⚠️ **Auth ไม่ enforce บน hot path + HTTP bind `0.0.0.0` ไม่มี authentication**

ข้อ 5 คือเหตุผลที่แผนถัดไปต้องเริ่มจาก **Security Hardening (Phase B) ก่อนเปิดใช้อะไรทั้งสิ้น**

---

## 2. สถานะปัจจุบัน — อะไรจริง / อะไรยัง contract-only

| Layer | สถานะ | หลักฐาน |
|---|---|---|
| Event ledger + object store (encrypted DEK/KEK) + idempotency | ✅ ของจริง | Task 1.1/1.2b suites — re-run เขียว |
| Bitemporal claims + state machine + provenance ×4 + entity model | ✅ ของจริง | Task 1.2/2.2 suites — re-run เขียว |
| Hard-purge saga 7 ขั้น (deny-first, crash/retry matrix) | ✅ ของจริง | Task 1.3 — 83 tests เขียว |
| Projection rebuild (Tantivy/Petgraph/Markdown) + composite checksum | ✅ ของจริง | Task 2.1 — re-run เขียว |
| Backfill/cutover (resolve-only, idempotent, rollback lossless) | ✅ ของจริง | Task 2.3 — re-run เขียว |
| MCP tool annotations + structured content | ✅ ของจริง (บน wiki tools เดิม) | Task 3.1 |
| `brain_*` tool surface (§7.2) | 🟨 ไม่มีสักตัว | grep `brain_` ใน src/mcp = ว่าง |
| Auth: Capability/AuthPolicy/redaction | 🟨 types+tests เท่านั้น — **ไม่ enforce** | `AuthPolicy` ไม่ถูกอ้างนอก auth.rs |
| AiProvider + OutboundPolicy (deny-by-default) + extraction pipeline | 🟨 contract | ไม่มี HTTP adapter จริง |
| Console/Galaxy/Trust views | 🟨 contract | ไม่มี React app; web/ = Hugo publisher เดิม |
| Deployment/Observability/Recovery | 🟨 contract | ไม่มี Dockerfile/compose จริง |
| Retrieval experiments + safe automation (Phase 7) | 🟨 contract | primitives พร้อม ใช้ได้เมื่อ retrieval จริงมา |

---

## 3. ผล Audit (Phase A — ปิดแล้ว, Validator PASS 10/10)

### Gates: reported vs actual

| Gate | Reported | Actual 2026-07-16 | Verdict |
|---|---|---|---|
| cargo fmt / clippy (default + all-features) | clean | exit 0 | ✅ |
| cargo test default / all-features (`-j 2`) | ~700+/0 | **~803 / ~810 pass, 0 fail** | ✅ (โตจาก Phase 5–7) |
| eval v1 (126 cases) | 126/126, byte_lock false (carried) | cases 126/126 ✅ **แต่ gate exit 1 เพราะ byte_lock** | ⚠️ ตรงตามรายงาน แต่ gate โดยรวมแดง |
| Python engine/mcp/acp | 63/76/26+2skip | 63/76/26+2skip | ✅ เป๊ะ |
| governance | 3 fail carried | 3 fail / 7 pass — root cause เดียว = byte-lock | ✅ เป๊ะ |
| cargo audit | 1 vuln + 3 warnings | **crossbeam-epoch 0.9.18 RUSTSEC-2026-0204** (fix ≥0.9.20) + bincode/anyhow/memmap2 warnings | ✅ ระบุตัวจริงได้แล้ว |

### Environment divergences (พบ + แก้แล้ว)

1. **GNU toolchain resolution** — rustup default host เครื่องนี้เป็น GNU, `rust-toolchain.toml` pin แค่ channel → build พัง (`dlltool.exe not found`) → แก้ด้วย `rustup override set 1.95-x86_64-pc-windows-msvc` (repo-local, reversible)
2. **`CARGO_TARGET_DIR=~\.cargo-target`** — build output ไม่อยู่ `./target/`; binary เก่า Jul 13 ใน `./target/` เป็นซาก (ลบแล้ว) — Python suites ต้องชี้ `LLM_WIKI_BIN` ไปที่ CARGO_TARGET_DIR

### Security findings (เรียงตาม severity)

| # | Severity | Finding | ผลกระทบ |
|---|---|---|---|
| S1 | **HIGH** | MCP HTTP bind `0.0.0.0` ไม่มี auth (`src/server.rs:85`) — ป้องกันแค่ Host-header allowlist ซึ่ง spoof ได้ | ถ้ารัน `serve --http` แล้วเครื่อง reachable = อ่าน/เขียนได้เต็มรูปแบบโดยไม่ต้อง auth — **ห้ามรัน `serve --http` บนเครื่อง exposed จนกว่า B1 ปิด** |
| S2 | **HIGH** (carried Phase 1) | `AuthPolicy` ไม่ wire เข้า `call_tool` dispatch; per-handle capability gap | ต้องปิดก่อนเปิด transport ใดๆ (Phase C) |
| S3 | MEDIUM | Eval byte-lock แตก (9 locked files แก้หลัง hash-lock) → eval ใช้เป็น tamper-evident gate ไม่ได้; governance แดง 3 | Process integrity — ต้องปิดก่อนใช้ eval เป็น promotion gate |
| S4 | LOW-MED | crossbeam-epoch RUSTSEC-2026-0204 | Bump ≥0.9.20 จบ |
| S5 | LOW | anyhow/memmap2 unsound warnings, bincode unmaintained | Bump opportunistic |
| S6 | LOW | Test suite เขียน generated schemas ลง tracked fixture; uv.lock ไม่ reproducible | Hygiene debt |

---

## 4. Forward Plan — Phase B → F

> กติกาทุก Task: Builder ทำ (TDD: RED → GREEN → refactor, coverage ≥80% ของ code ที่เปลี่ยน)
> → Independent Validator (agent แยก, read-only) ตรวจ DoD → PASS จึงปิด → รายงานผู้ใช้
> ห้ามหลาย implementation Task พร้อมกัน; error ทุกตัวใช้เป็น feedback วนแก้จนผ่าน

### Phase B — Security Hardening (ถัดไปทันที)

| Task | งาน | DoD |
|---|---|---|
| **B1** Quick wins | bind default → `127.0.0.1` + non-loopback ต้อง explicit opt-in พร้อมคำเตือน; `cargo update -p crossbeam-epoch`; bump anyhow/memmap2 ถ้ามี fix | test พิสูจน์ default = loopback; audit 0 vuln (warnings มี written acceptance); full regression เขียว |
| **B2** Eval re-lock | ตรวจ git history ว่า 9 locked files แก้ผ่าน validator จริง → regenerate manifest hashes | eval runner exit 0; governance 10/10; rationale ใน decision history |
| **B3** Auth wiring | `AuthPolicy::allows` enforce จริงใน dispatch + ปิด per-handle HIGH | negative tests fail ที่ dispatch จริง; per-handle exploit ปิดพร้อม test; regression เขียว |

**Gate B:** security reviewer (agent แยก) ไม่มี CRITICAL/HIGH unresolved

### Phase C — `brain_*` Semantic Wiring (ทำให้ Claude Code ใช้ได้จริง)

| Task | งาน | DoD |
|---|---|---|
| **C1** Read tools | engine ถือ `SemanticStore`; wire `brain_search/get/get_timeline/get_evidence/explain/status` | ครบ 6 ตัวตาม §7.2: structured+text, pagination, bounded, schema tests ผ่าน |
| **C2** Mutation tools | `brain_capture/propose/confirm/supersede/ingest_source/merge_entity/split_entity/purge` + operation_id + capability จาก B3 | idempotency/retry ผ่าน MCP layer จริง; purge มี re-auth + two-step nonce |
| **C3** Interop | stdio + HTTP (loopback) กับ Claude Code + MCP Inspector จริง | read/write/as-of/needs-input ผ่านจาก client จริง ≥2 ตัว; retry ไม่ duplicate |

**Gate C:** GULF temporal scenario (§11) ตอบถูกจาก Claude Code จริง end-to-end

### Phase D — Z.ai Adapter จริง (ยิง network ต้องขออนุมัติทุกครั้ง)

| Task | งาน | DoD |
|---|---|---|
| **D1** HTTP adapter | implement `AiProvider` สำหรับ Z.ai endpoint หลัง config + kill switch | error modes ครบกับ mock server; live smoke (ขออนุมัติ) ผ่าน |
| **D2** Extraction + adversarial | round-trip จริง + §11 adversarial corpus 30 cases | injection ไม่ข้าม policy; secret/local-only egress = 0 ทุก path |

**Gate D:** Phase 4 Gate ปิดแบบ runtime + compliance record ครบ

### Phase E — Console + Galaxy จริง (browser/Playwright)

E1 React Console shell + review workflow → E2 Galaxy 3D + LOD + fallback → E3 Trust/ops + UAT
**Gate E = Phase 5 Gate ปิด:** UAT หุ้น/โปรเจกต์/ความรู้ + browser/security/a11y/perf gates (DoD ละเอียดตาม GOAL §13 Task 5.1–5.3 เดิม)

### Phase F — Production Closure (Docker)

F1 `docker compose up` จริงบน clean host → F2 restore drill จริง + upgrade/rollback rehearsal → F3 external security review
**Gate F = Phase 6 Gate ปิด:** ไม่มี critical/high unresolved; RPO/RTO บันทึก

### ลำดับ + เหตุผล

```
B (hardening) ──► C (brain_* จริง) ──► D (AI ingestion) ──► E (Console) ──► F (production)
   ปิดช่องโหว่ก่อน    ระบบใช้ได้จริงจาก      ความรู้ไหลเข้า        มอง/จัดการได้      ขึ้น Oracle VM
   เปิด transport     Claude Code           อัตโนมัติ                              24/7
```

- B ก่อน C: เปิด transport ทั้งที่ auth ไม่ enforce = ช่องโหว่ทันที (S1+S2)
- C ก่อน D: proposal จาก AI ต้องมีที่ลง (brain_propose) ก่อน adapter จะมีประโยชน์
- E หลัง C/D: Console ต้องมี API จริงให้เรียก ไม่งั้นก็เป็น mock อีกรอบ
- F สุดท้าย: deploy เมื่อของข้างในครบและ external review ผ่าน

---

## 5. ความเสี่ยงคงค้างที่ผู้ใช้ต้องรู้

1. **อย่ารัน `serve --http` บนเครื่องที่ expose ต่อ network** จนกว่า B1 จะปิด (S1)
2. เครื่องนี้ build ได้เพราะ rustup override — เครื่องอื่น/CI ที่ default host เป็น GNU จะเจอปัญหาเดิม (พิจารณาบันทึกใน CONTRIBUTING หรือ CI matrix)
3. Z.ai Coding Plan terms risk (GOAL §2.3) ยังอยู่ — compliance record + kill switch บังคับใน D1
4. Phase E/F ต้องการ dependency ใหม่ (React, Playwright, Docker images) — จะขออนุมัติก่อน install ตามกติกา

---

## 6. Next Action

**เริ่ม Phase B Task B1** (loopback bind guard + dependency bumps) — งานเล็ก ปิด HIGH finding S1 กับ vuln S4 ในรอบเดียว รอคำสั่งจากผู้ใช้
