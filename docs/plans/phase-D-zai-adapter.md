# Phase D — Z.ai Adapter จริง (AI Ingestion end-to-end) ⬜ NEXT

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0` (ต่อจาก D1 RED `babda7d`)
> เป้าหมาย: ปิด **Phase 4 Gate** แบบ runtime — AI สร้าง proposal ที่ตรวจย้อนได้จริงผ่าน adapter จริง โดย worker ไม่มีสิทธิ์ confirm

## Decisions (2026-07-16)
- **NO MOCK** — adapter ทดสอบกับ endpoint จริง `https://api.z.ai/api/coding/paas/v4/chat/completions`
- **Network policy:** live test gate ด้วย `ZAI_API_KEY` env present; ยิง batch แรกต้อง**ขออนุมัติ**; response จริงถูกเก็บเป็น **golden fixtures (real bytes)** เพื่อ replay ใน CI โดยไม่ยิงซ้ำ (ไม่ใช่ mock). ไม่มี key → live tests `skip` (ไม่ fabricate)
- **Secret:** API key ผ่าน env/Docker secret เท่านั้น; grep-gate กัน key เข้า repo/log
- ต้องเพิ่ม dependency HTTP client (เช่น `reqwest`) — **ขออนุมัติก่อน install**

## Task D1 — ZaiHttpAdapter จริง + golden replay *(ต่อจาก RED `babda7d`)*
- `ZaiHttpAdapter` implement `AiProvider` — ยิง HTTP จริง; **ลบ mock-transport `Box<dyn Fn>`** ที่ค้างใน `provider.rs:312` (ปิด clippy `type_complexity` ในตัว)
- Map `ProviderError` ครบ 8 modes (timeout/quota/429/5xx/invalid-JSON/partial-stream/outage/disabled) + bounded retry/backoff + kill switch
- ทุก request ผ่าน `OutboundPolicy` **check-then-send ครบทั้ง normal + retry + dead-letter path**
- Live smoke: chat completion + `json_object` → validate JSON schema ฝั่งเราเอง → บันทึก golden response
- `ComplianceRecord` เขียนจริง: user decision, endpoint, workload, known-terms risk, retention/region, `acknowledged_at`

**DoD:**
1. Live smoke (ขออนุมัติ): real chat completion สำเร็จ, JSON schema-valid หลัง bounded repair, golden fixture บันทึก
2. Error-mode tests ครบ 8 + retry bounded + kill switch block
3. Intercepted-outbound: `local_only`/detected secret **ไม่ปรากฏ**ใน request/retry/dead-letter/telemetry/log — 100%
4. `ComplianceRecord` ครบทุก field + persisted
5. ไม่มี key ใน repo/log (grep gate); fmt/clippy clean (รวม provider.rs); full regression เขียว

## Task D2 — Worker identity + privilege separation *(ปิด F2)*
- brain_* handlers เลิก hardcode `trusted_context()` → derive context จาก MCP principal: owner (full) หรือ registered worker (`register_client_scoped(label, [Propose])`)
- เพิ่ม `brain_propose` (worker → `propose_inference`, status `proposed` เสมอ)
- Enforcement 2 ชั้น: MCP dispatch (`check_capability` — มีแล้วจาก F1) + store layer (propose-only context เรียก confirm → fail-closed)

**DoD:**
1. Worker context เรียก `brain_confirm`/`brain_supersede`/`brain_purge` → denied **ทั้งชั้น dispatch และชั้น store** (negative tests แยก 2 ชั้น)
2. Worker เรียก `brain_propose` ได้; proposal เป็น `proposed` เสมอ ไม่มีทาง auto-confirm
3. Owner path ไม่พัง: brain E2E stdio เขียวเหมือนเดิม

## Task D3 — Extraction pipeline live: source → spans → typed proposals
- `brain_ingest_source` (URL/file/text → quarantine ใน object store) + **SSRF guard**: URL allow/deny, private-IP block, redirect cap, size/MIME limits
- Worker: quarantined source → `ExtractionProposal` + **exact evidence span** (slice bytes จาก rendition → hash ต้องตรง `quote_sha256` — ปิด F1/F2 informational ของ Task 4.2 เดิม)
- JSON schema validation + bounded repair; partial stream = ทิ้ง
- Adversarial corpus §11 (30 cases) รันกับ adapter จริง (golden replay สำหรับ regression)

**DoD:**
1. Evidence-span exactness **100%** บน annotated fixtures
2. Prompt-injection corpus: worker ไม่ execute instruction/side effect — 100%
3. `local_only`/secret negative corpus 100% ครบ normal/repair/retry/failure
4. Schema-valid after bounded repair ≥99%; audit ครบ prompt/model/schema version
5. SSRF tests (private IP, redirect-to-internal, oversized, bad MIME) — denied ทั้งหมด

**Phase D Gate:** eval domain (stocks/projects/knowledge) ผ่าน adapter จริง + adversarial 100% + ไม่มี direct truth mutation + compliance record ครบ → **Independent Validator PASS**
