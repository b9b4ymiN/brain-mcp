# Feature — AI Pre-Review (Inbox) + Quality Rules (Extraction) ⬜ pending

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> สถานะ: **SPEC ONLY — ยังไม่ implement** (สร้าง 2026-07-20 เพื่อส่งต่อ new session)
> เป้าหมาย: แก้ปัญหา "มนุษย์ approve ยาก + AI extract ออกมาเป็นขยะ" โดยไม่บังคับเปลี่ยน design (ADR-0001 §Decision 1 ยังอยู่: proposed → confirmed ผ่าน human review)

## ที่มา (Context — ทำต้องมี feature นี้)

หลัง ingest `CATL_BF-Report.html` (201 proposed claims) + ข้อมูล thai-shipping/v2/v4 (~25 claims) พบปัญหาจริง 2 ข้อในการ review ใน Inbox:

1. **มนุษย์ลำบาก approve** — proposals ใน Inbox มีทั้ง fact จริง + metadata/log + opinion ปนกัน ต้องเปิดอ่าน evidence ยาวๆ (3000+ ตัวอักษร) ทีละอันจึงจะตัดสินใจได้ → คิวรอ review ยาว (187 pending ในการทดสอบ 2026-07-20)
2. **AI extract ออกมาเป็นขยะที่ต้นน้ำ** — `build_extraction_prompt` (`src/extraction.rs:147`) มีแค่ schema rules (field ครบไหม, value เป็น null ไหม) ไม่มี semantic quality rules → AI สร้าง claims ที่:
   - ยัดหลาย fact ใน value เดียว (`"26% vs 15%"` — anti-pattern #18)
   - predicate กำกวม (`"margin"` ไม่ระบุ segment/ปี — ทำให้มี claims margin 6 ค่าขัดแย้งกัน)
   - confidence 1.0 ทั้งที่มาจาก extracted source (anti-pattern #20)
   - domain/kind free-form → galaxy แตก (anti-pattern #17)

ทั้งสองปัญหามี root cause ร่วม: **skill `brain` มี anti-patterns.md ถึง 21 ข้อ แต่ rules เหล่านั้นอยู่ใน "คู่มืออ่านด้วยตา" ไม่ได้ถูกบังคับในตัว AI ที่ทำงานจริง** (ทั้ง `brain_extract` และ Inbox review flow)

## เป้าหมาย (Goals)

| # | เป้าหมาย | วัดยังไงว่าสำเร็จ |
|---|---------|-----------------|
| G1 | ลดเวลา review ต่อ proposal จาก ~2-3 นาที (อ่าน evidence เอง) → <30 วินาที (อ่าน tag สรุป) | ทดสอบกับ 187 pending proposals |
| G2 | ดักขยะที่ต้นน้ำ — extraction prompt v3 ลด packing/duplicate/vague ลง ≥80% vs v2 | Re-ingest CATL chunk 0 เทียบขนาด output |
| G3 | ไม่เปลี่ยน design — human ยังเป็นคน approve/reject สุดท้าย (ไม่ใช่ AI auto-confirm) | ADR-0001 §Decision 1 ยังเป็นจริง |
| G4 | ทำงานได้แม้ provider disabled (deterministic rules อย่างเดียว) | Phase 1 deploy โดยไม่ต้องใช้ provider |

## Non-goals (สิ่งที่จะไม่ทำ)

- ❌ Auto-approve / auto-reject (human-in-the-loop ตลอด — AI แค่ tag/warn)
- ❌ แก้ claims ประวัติศาสตร์ (historical CATL v2 claims เก็บไว้เพื่อ audit)
- ❌ เพิ่มปุ่ม "Capture claim" ใน Inbox UI (separate feature — capture ยังผ่าน MCP เท่านั้นตาม design)
- ❌ ลบ/แทนที่ rules 21 ข้อใน anti-patterns.md (ยังเป็นคู่มืออ่าน แค่เพิ่ม enforcement ใน code)

---

## Decision (2026-07-20)

- **Trigger model: on-demand ราย proposal** — ปุ่ม "AI Review" ใน detail panel ของแต่ละ proposal (lazy, ไม่เรียกจนกว่าผู้ใช้กด) ประหยัด token ที่สุด + แม่นที่สุด (เทียบกับ auto-on-open ที่จะใช้ token 187 ครั้งต่อหน้า)
- **Rule implementation: Deterministic + AI hybrid** — rules ที่เช็คด้วย code ได้ (regex/schema/search) ทำก่อน เพราะแม่น 100% + ประหยัด; rules ที่ต้องใช้ LLM (semantic) เพิ่มทีหลัง
- **Order: ทาง A (Pre-Review ใน Inbox) ก่อน → ทาง C (Extraction prompt) ตาม** — A แก้ปัญหาปัจจุบัน (187 pending) ได้ทันที; C ป้องกันขยะใหม่

---

## Architecture (มุมมอง 3 ชั้น)

```
┌─────────────────────────────────────────────────────────────────┐
│ Frontend (Inbox.svelte)                                          │
│   ปุ่ม "AI Review" ใน detail panel                                │
│   → GET /api/v1/inbox/{proposal_id}/ai-review                   │
│   → แสดง tags ใน detail panel (chip + color + tooltip)          │
│   → cache 5 นาทีใน frontend state                               │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ Backend (src/api.rs)                                             │
│   GET /inbox/{proposal_id}/ai-review                            │
│   extractor: AuthSession (session-only, NO CSRF — read-only)    │
│   → อ่าน ProposalSummary + EvidenceSummary (public store methods)│
│   → เรียก QualityChecker::check(proposal, evidence, all_claims) │
│   → return JSON { tags: [...], checked_at, version }            │
│   ไม่ mutate ledger/event ใดๆทั้งสิ้น                              │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ QualityChecker (src/quality.rs — ไฟล์ใหม่)                      │
│   Phase 1 (deterministic): เช็ค rules แบบ rule-based            │
│     - DUPLICATE_PREDICATE: search confirmed by (subject, pred)   │
│     - PACKED_FACTS: regex comparator keywords in value          │
│     - VAGUE_PREDICATE: regex `^current |^margin$|^price$`       │
│     - TAXONOMY_DRIFT: whitelist domain/kind                     │
│     - CONFIDENCE_TOO_HIGH: extracted + confidence==1.0           │
│     - ... (รายการเต็มด้านล่าง)                                   │
│   Phase 2 (AI — optional): เรียก provider เฉพาะ semantic rules  │
│     - SOURCE_CLAIM_MISMATCH: value ไม่ตรง evidence excerpt       │
│     - SEMANTIC_DUPLICATE: คนละ predicate แต่ความหมายเดียวกัน    │
│     - ทำงานเฉพาะเมื่อ ai_provider = Some(...)                    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Rules 21 ข้อ — แยก deterministic vs AI vs out-of-scope

จาก `skills/brain/references/anti-patterns.md`:

### 🟢 กลุ่ม 1: Deterministic (ทำด้วย rule-based code — แม่น 100%)

| # | Anti-pattern | วิธีเช็ค | Phase |
|---|-------------|---------|-------|
| **8** | Source without redact | prompt instruction (extraction) | C |
| **9** | Idempotency retry with changed payload | operation_id reuse check | — (runtime มีอยู่แล้ว) |
| **11** | `brain_propose` for user utterance (or reverse) | claim_kind vs evidence match | 1 |
| **13** | Tool call when schema_version degraded | brain_status check | — (runtime มีอยู่แล้ว) |
| **16** | `[[double-bracket]]` in body | regex `\[\[.*\]\]` | 1 |
| **17** | Free-form domain/kind (taxonomy drift) | whitelist domain/kind | 1 |
| **18** | Packing two facts in one value | value contains `vs`, `→`, `compared to` | 1 |
| **19** | Re-stating same fact under 2nd predicate | search confirmed by `(subject, lemmatized-predicate)` | 1 |
| **20** | `current X` predicate + confidence 1.0 | regex `^current ` + `confidence == 1.0` | 1 |

### 🟡 กลุ่ม 2: AI (ต้องใช้ LLM — semantic)

| # | Anti-pattern | ทำไมต้อง AI | Phase |
|---|-------------|-----------|-------|
| **5** | Collapsing source into concept (provenance loss) | เข้าใจ semantics ของ source vs claim | 3 |
| **7** | Subject ไม่ใช่ proper noun | เข้าใจ context | 3 |
| **18 บางส่วน** | Packed facts ซับซ้อน | regex ไม่พอ | 3 |
| **19 บางส่วน** | Semantic duplicate (คนละ predicate แต่เหมือนกัน) | semantic similarity | 3 |
| — | Predicate vague (margin ไม่ระบุ segment) | เข้าใจ domain | 3 |
| — | Source-claim mismatch (value ไม่ตรง evidence) | อ่านทั้งสองฝั่ง | 3 |

### 🔴 กลุ่ม 3: Out of scope (agent discipline หรือ architectural — ไม่ใช่ claim quality)

| # | Anti-pattern | เหตุผลที่ไม่เอา |
|---|-------------|---------------|
| 1, 2, 14, 15 | Silent write / no-read-before-write / skip bootstrap | agent discipline ไม่ใช่ claim |
| 3, 4 | Type system confusion (procedural vs concept) | เกี่ยวกับ wiki page |
| 6 | Edit `site/content/` directly | wiki plumbing |
| 7 | Destructive without consent | UX flow |
| 10 | Public port exposure | ops |
| 12 | Edit Markdown for ledger claim | already-prevented by architecture |
| 21 | Client-scope trap (Console retract MCP claim) | ปัญหาเฉพาะ retract |

**สรุป**: Phase 1 ทำ deterministic 9 ข้อ (แม่น 100% + ประหยัด) → Phase 3 เพิ่ม AI 6 ประเด็น

---

## Tag format (TypeScript/Rust shared)

```typescript
// QualityTag — ผลลัพธ์จาก QualityChecker
interface QualityTag {
  kind: QualityTagKind       // enum, ตรงกับ Rust serde
  severity: 'info' | 'warning' | 'critical'
  message: string            // ภาษาคน บอกทำไม flag
  evidence?: string          // optional: quote ที่ trigger rule
}
type QualityTagKind =
  | 'duplicate_predicate'    // #19 — same (subject, predicate) มีอยู่แล้ว
  | 'packed_facts'           // #18 — value มี comparator
  | 'vague_predicate'        // margin/price ไม่จำเพาะ
  | 'taxonomy_drift'         // #17 — domain/kind ไม่อยู่ใน whitelist
  | 'confidence_too_high'    // #20 — extracted + 1.0
  | 'double_bracket'         // #16 — [[...]] in body
  | 'kind_mismatch'          // #11 — claim_kind vs evidence
  | 'source_claim_mismatch'  // AI — value ไม่ตรง evidence
  | 'semantic_duplicate'     // AI — ความหมายซ้ำ claim อื่น
  | 'provenance_loss'        // AI #5 — source collapse

interface AiReviewResponse {
  proposal_id: string
  tags: QualityTag[]
  checked_at: string         // ISO 8601
  checker_version: string    // e.g. "quality-v1"
  ai_used: boolean           // true if Phase 3 ran
}
```

---

## Implementation phases

### Phase 1 — QualityChecker skeleton + deterministic rules (1-2 วัน)

**ไฟล์:**
| ไฟล์ | การเปลี่ยนแปลง |
|------|--------------|
| `src/quality.rs` (ใหม่) | `QualityChecker` struct + `check()` → `Vec<QualityTag>` + tag enum |
| `src/lib.rs` | `pub mod quality;` |
| `tests/quality_rules_v1.rs` (ใหม่) | unit tests 9 rules + false-positive test กับ confirmed claims 30 อัน |

**DoD:**
1. ✅ Unit test ครบ 9 deterministic rules
2. ✅ ทดสอบกับ proposals จริง 187 pending — แต่ละ rule ต้องจับ ≥1 proposal จริง
3. ✅ False positive rate < 5% บน confirmed claims 30 อัน (claims ที่ผ่าน review ต้องไม่โดน tag ผิด)
4. ✅ `cargo fmt + clippy + test` green

### Phase 2 — HTTP endpoint + Frontend ปุ่ม (1 วัน)

**ไฟล์:**
| ไฟล์ | การเปลี่ยนแปลง |
|------|--------------|
| `src/api.rs` | +route `/inbox/{proposal_id}/ai-review` + handler `ai_review` |
| `src/api.rs` | +`ai_provider: Option<Arc<dyn AiProvider>>` field ใน `ConsoleApiState` (Phase 3 ใช้) |
| `src/server.rs:176-190` | สร้าง adapter ครั้งเดียว แล้ว clone `Arc` ให้ทั้ง MCP + Console |
| `web/console/src/lib/api.ts` | +`aiReview(proposalId)` + `QualityTag`/`AiReviewResponse` types |
| `web/console/src/pages/Inbox.svelte` | ปุ่ม "AI Review" + state + render tags ใน detail panel |
| `tests/api_ai_review_v1.rs` (ใหม่) | endpoint tests: 401/404/200 + tag shape |

**DoD:**
1. ✅ `GET /inbox/{id}/ai-review` คืน `{tags, checked_at, checker_version, ai_used:false}` (Phase 2: ai_used=false เสมอ)
2. ✅ 401 missing session, 404 bad id, 200 valid
3. ✅ Frontend: กดปุ่ม → loading → tags ปรากฏใน <1s (deterministic เร็วมาก)
4. ✅ Tags cached 5min ใน frontend state (refetch on proposal change)
5. ✅ Console 0 errors + svelte-check 0 errors + build ผ่าน
6. ✅ ทดสอบด้วย Playwright: เปิด `CATL margin is 26%` → กด AI Review → เห็น tag DUPLICATE_PREDICATE + VAGUE_PREDICATE

### Phase 3 — AI rules (semantic, optional) (3-5 วัน)

**ไฟล์:**
| ไฟล์ | การเปลี่ยนแปลง |
|------|--------------|
| `src/quality.rs` | +`AiQualityChecker` ที่เรียก provider + build review prompt |
| `src/provider.rs` | ใช้ `OutboundPolicy::check_text` ที่มีอยู่แล้วสำหรับ egress gate |
| `tests/quality_ai_v1.rs` (ใหม่) | mock provider tests + egress denial tests |

**Review prompt (ส่วนหนึ่งของ build):**
```
You are reviewing a proposed claim for quality. Return JSON only.

CLAIM:
  subject: <subject>
  predicate: <predicate>
  value: <value>
  domain: <domain>
  claim_kind: <kind>

EVIDENCE (source text the claim was extracted from):
  <evidence excerpt>

EXISTING CLAIMS in same scope (subject + similar predicate):
  <list of confirmed claims, max 10>

Check for:
1. SOURCE_CLAIM_MISMATCH — value ไม่ตรงหรือเกินจริงจาก evidence
2. SEMANTIC_DUPLICATE — เหมือน claim ใน existing list (ความหมายซ้ำ)
3. PROVENANCE_LOSS — source collapse (concept ที่ไม่มี source citation)
4. VAGUE_PREDICATE_AI — predicate กำกวม (ถ้า deterministic ยังไม่ตัด)

Response shape (mandatory):
  {"tags":[{"kind":"source_claim_mismatch"|...,"severity":"warning","message":"..."}]}
  No tags = {}
```

**DoD:**
1. ✅ ทดสอบ `CATL margin is 26%` → tag SOURCE_CLAIM_MISMATCH (value 26% มาจากไหนใน evidence ไม่ชัด)
2. ✅ ทดสอบ `international-peers-deep` → tag PACKED_FACTS (AI ยืนยัน)
3. ✅ Provider disabled → ตกกลับเป็น deterministic-only (`ai_used: false`) ไม่ error
4. ✅ Egress denial (local_only proposal หรือ detected secret) → ไม่ error แค่ `ai_used:false`
5. ✅ Cache: same proposal + 5min = ไม่เรียก provider ซ้ำ
6. ✅ max_tokens จำกัด 2048 (review prompt สั้นกว่า extraction)

### Phase 4 — Extraction prompt v3 (1 วัน)

**ไฟล์:**
| ไฟล์ | การเปลี่ยนแปลง |
|------|--------------|
| `src/extraction.rs:147-173` | +section "QUALITY RULES" ใน `build_extraction_prompt` |
| `src/extraction.rs:120` | bump `EXTRACTION_PROMPT_VERSION` = `"d3-extraction-v3"` |
| `skills/brain/references/anti-patterns.md` | +cross-ref note "rules นี้บังคับใน extraction prompt ตั้งแต่ v3" |
| `tests/extraction_prompt_v3.rs` (ใหม่) | snapshot test ของ prompt ใหม่ |

**Quality rules ที่จะเพิ่ม (10 ข้อ ใน prompt):**
```
QUALITY RULES (mandatory, verified downstream by QualityChecker):
1. ONE atomic fact per claim — never pack comparators
   BAD:  predicate="margin", value="26% vs peer 15%"
   GOOD: predicate="EV Battery segment gross margin FY2025", value="24%"

2. Predicate MUST be specific — include segment + time period
   BAD:  predicate="margin"           (which? when?)
   GOOD: predicate="Q1 2026 gross margin"

3. DEDUPE — if the same (subject, predicate) exists in your prior
   claims this chunk, OMIT the duplicate (do not emit a second one)

4. CONFIDENCE ≤ source confidence — never 1.0 on extracted values
   (1.0 reserved for human-asserted facts; extraction max = 0.9)

5. AVOID `current X` predicates without time anchor
   BAD:  predicate="current stock price"
   GOOD: predicate="stock price (as of 2026-07-08)"

6. VALUE must be a single scalar/string — no embedded tables,
   no newline-packed DCF reports (split into separate claims)

7. SUBJECT must be a proper noun (entity name), not a section heading
   BAD:  subject="DCF assumptions"
   GOOD: subject="CATL"

8. Domain must be in taxonomy: stocks | fx | shipping | crypto |
   projects | personal | financial | business | workflow
   (no free-form domains — they fragment the galaxy graph)

9. CLAIM_KIND must be: user_assertion | inference | metric |
   fact | projection (no changelog/event/log/meta kinds)

10. SKIP non-facts: deliverable logs, workflow status, commit
    messages, file paths, "report delivered" — these are NOT claims
```

**DoD:**
1. ✅ `EXTRACTION_PROMPT_VERSION = "d3-extraction-v3"` + doc comment อธิบายการเปลี่ยนแปลง
2. ✅ Snapshot test ของ prompt ผ่าน (anti-regression)
3. ✅ Re-ingest CATL chunk 0 → claims ใหม่ผ่าน QualityChecker ทั้งหมด (target: ≥80% ลด packing/duplicate)
4. ✅ Historical claims ยังเป็น v2 (audit trail intact — `prompt_version` recorded per claim)
5. ✅ anti-patterns.md cross-ref note added

---

## ความเสี่ยง + ทางออก

| ความเสี่ยง | ระดับ | ทางออก |
|----------|------|-------|
| Provider ไม่ได้ติดตั้ง → Phase 3 AI rules ไม่ทำงาน | ต่ำ | Phase 1+2 ทำงานได้เลย (deterministic-only) — Phase 3 เป็น optional enhancement |
| LLM เสียเวลา/เงิน | กลาง | on-demand trigger + cache 5min + max_tokens 2048 + reuse `OutboundPolicy` egress gate |
| False positive ทำให้ผู้ใช้เสียความมั่นใจ | กลาง | tag เป็น **warning** ไม่ใช่ block — ผู้ใช้ยัง approve/reject เองได้; DoD false positive < 5% |
| Extraction v3 ทำให้ historical audit ขาด | ต่ำ | `prompt_version` บันทึก per-claim — historical ยัง v2 ไม่สูญ |
| ครบทั้ง 4 phase อาจจะยาว | กลาง | แต่ละ phase deploy อิสระ — Phase 1+2 ใช้งานได้จริงทันที (จบปัญหา "approve ยาก") |
| Provider call from Console API (ใหม่ใน codebase) | กลาง | clone `Arc<dyn AiProvider>` จาก MCP path; read-only handler ไม่มี mutation risk; egress policy บังคับเหมือนเดิม |
| `local_only` proposal รั่วผ่าน AI | สูง | `OutboundPolicy::check_text` deny-by-default (เหมือน `brain_extract`); denial = `ai_used:false` ไม่ใช่ error |

---

## การทดสอบจริงกับข้อมูลจริง (live-fire)

ใช้ proposals จริง 3 ตัวอย่างที่เจอในการทดสอบ 2026-07-20:

| Proposal | คาดว่าจะ tag | Phase ที่จะจับได้ |
|---------|------------|----------------|
| `USDTHB-2026-07-20 has_rate 33.59` | (no tags — clean fact) | 1+ (negative test) |
| `CATL margin is 26%` | DUPLICATE_PREDICATE (มี margin 15%/24%/30% อยู่แล้ว) + VAGUE_PREDICATE + PACKED_FACTS + SOURCE_CLAIM_MISMATCH | 1 (deterministic) + 3 (AI) |
| `thai-shipping-v4 produced_deliverable v4_202KB_...` | TAXONOMY_DRIFT (kind=deliverable ไม่ใช่ claim_kind) + VAGUE_PREDICATE | 1 |
| `international-peers-deep has_peer_data peers_primary_...` | PACKED_FACTS (ตารางใน value) + VAGUE_PREDICATE | 1 + 3 |

---

## คำถามที่ยังไม่ได้ตัดสินใจ (open — สำหรับ new session ตัดสินใจ)

1. **Cache strategy**: frontend-only (5min TTL ใน Inbox state) vs backend (SQLite table)? — default frontend-only (ง่าย + เพียงพอ)
2. **Phase 4 re-ingest**: ปล่อย historical CATL 201 claims เป็น v2 ไว้ หรือ batch re-ingest? — default ปล่อยไว้ (audit)
3. **Tag rendering**: chip สีเดียวต่อ severity หรือไอคอนต่างกันต่อ kind? — default chip + tooltip (เรียบง่าย)
4. **Rate limit**: cap AI Review calls ต่อ session (ป้องกัน spam)? — default ใช้ `IngestRateLimiter` ที่มีอยู่ (Phase 3)

---

## Cross-references

- ที่มาปัญหา: `skills/brain/references/anti-patterns.md` (21 ข้อ)
- Design constraint: `docs/adr/0001-semantic-authority-time-privacy.md` §Decision 1 (proposal → confirmed)
- Extraction prompt ปัจจุบัน: `src/extraction.rs:147` (`EXTRACTION_PROMPT_VERSION = "d3-extraction-v2"`)
- Provider abstraction: `src/provider.rs:55` (`AiProvider` trait)
- Inbox API pattern: `src/api.rs:799` (`evidence` GET handler — template สำหรับ `ai_review`)
- Existing contradiction detector (confirmed-only): `src/semantic.rs:5097` (`contradictions()`) — ใช้ deep Value equality เป็น primitive
- Frontend inbox: `web/console/src/pages/Inbox.svelte` + `web/console/src/lib/api.ts`

---

## การ hand-off ไป new session

New session ควร:
1. อ่าน spec นี้ทั้งหมดก่อนเริ่ม
2. เริ่มจาก **Phase 1** (QualityChecker skeleton + deterministic rules) — ไม่ต้องใช้ provider เลย
3. Deploy Phase 1+2 ก่อนแล้วค่อยทำ Phase 3 (เห็นผลจริงเร็วที่สุด)
4. อ้างอิง file:line ใน spec นี้เพื่อหาจุดแก้ไข
5. ทดสอบกับ proposals จริง 3 ตัวอย่าง (live-fire section) หลัง Phase 2 + Phase 3
