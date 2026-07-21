# Report — Subject Validator v1 (Phase 1.5) — SHIPPED

> Date: 2026-07-21
> Branch: `vnext/phase-0`
> Spec: `docs/plans/subject-validator-v1-spec.md`
> Plan: `docs/plans/subject-validator-v1-implementation-plan.md`
> Status: ✅ **Production-verified on live 182-proposal inbox**

---

## TL;DR

ส่งมอบระบบ **6-layer deterministic subject validator** พร้อม engine/data separation (Wikidata pattern) — ตรวจ `subject` field ของทุก proposal ก่อนเข้า Inbox. ทดสอบจริงบน Docker container + Web UI ด้วย 17 edge cases ผ่านทั้งหมด, bulk scan 122 unique subjects → 39 rejects (32%) ตรงตามเป้า DoD.

Phase 1.5 แก้ปัญหาต้นน้ำ: LLM ดึง subject เป็น metric term ("Risk-free rate", "Beta", "Cash") หรือ slug ("thai-shipping-stocks-2026-07-20") แทน entity name ("CATL") → scope key พัง → conflict detection ใช้ไม่ได้ → user ไม่รู้จะ Approve ไหม.

---

## 🎯 Outcome เชิงประจักษ์ (verified ใน Docker)

### ทดสอบ API จริง 17 เคสผ่าน 100%

| # | Subject | Expected | Actual | ✅ |
|---|---------|----------|--------|---|
| 1 | `Risk-free rate` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 2 | `US tariff rate` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 3 | `Median target price` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 4 | `Price` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 5 | `Cash` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 6 | `Cash (post-placement)` | CRITICAL metric (paren) | 🔴 CRITICAL | ✅ |
| 7 | `Beta` | CRITICAL metric_single | 🔴 CRITICAL | ✅ |
| 8 | `Cost of equity` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 9 | `Current case price` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 10 | `China CAGR` | CRITICAL metric | 🔴 CRITICAL | ✅ |
| 11 | `50% Fib yearly level` | CRITICAL NumberLed | 🔴 CRITICAL | ✅ |
| 12 | `thai-shipping-stocks-2026-07-20` | CRITICAL Slug | 🔴 CRITICAL | ✅ |
| 13 | `simandou-first-shipment` | CRITICAL Slug | 🔴 CRITICAL | ✅ |
| 14 | `freight-china-soybean-macro-2026-07` | CRITICAL Slug | 🔴 CRITICAL | ✅ |
| 15 | `international-peers-deep-2026-07` | CRITICAL Slug | 🔴 CRITICAL | ✅ |
| 16 | `China challengers (CALB, EVE)` | WARNING MultiEntity | 🟠 WARNING | ✅ |
| 17 | `CATL` / `BYD` / `TSLA` / `NVDA` | silent accept | 🟢 silent | ✅ |

### Bulk scan ทั้ง 182 pending proposals

| Verdict | Count | % | หมายเหตุ |
|---------|-------|---|---------|
| 🟢 accept | 82 unique subjects | 67% | ~143 proposals silent |
| 🔴 reject | 39 unique subjects | 32% | 26 metric + 11 Slug + 1 NumberLed + 1 metric_single |
| 🟠 soft_flag | 1 unique subject | 1% | MultiEntity |

**หมวด reject ทั้งหมด:**
- 26 × metric_head (`rate`/`price`/`growth`/`value`/`cash`/`equity`/...)
- 11 × Slug (`thai-shipping-*`, `simandou-*`, `psl-*`, `freight-china-*`, `international-peers-*`)
- 1 × NumberLed (`50% Fib yearly level`)
- 1 × metric_single (`Beta`)

### DoD §8.1 verification

| DoD | Expected | Actual | Status |
|-----|----------|--------|--------|
| Critical count | 30-50 | **39** | ✅ ตรงกลาง |
| Silent count | ≥85 proposals | ~143 proposals | ✅ |
| FP rate | <5% | 0% (จาก 17 cases) | ✅ |
| Thai accept | "บมจ. ปตท." ไม่ reject | n/a (ไม่มี Thai ใน inbox ตอนนี้) | ⚠️ untested live |

---

## 📦 Commits ทั้งหมด (9)

| Commit | Phase | Description |
|--------|-------|-------------|
| `dd6667a` | 0 | Spec — `subject-validator-v1-spec.md` |
| `7f1622a` | 0 | Implementation plan — 18 TDD tasks |
| `bbdfb3f` | A | Engine Layer 0-2 + 39 tests |
| `c888464` | B | Rules Data Layer 3-4 + 12 tests |
| `091f93d` | C | Facade + wire + boot + audit field |
| `7479328` | D | Console UI 8 new QualityTagKind variants |
| `b47c2ea` | E | Integration tests + clippy |
| `15754d0` | E | Regression runner + docs supersede |
| `29cdbe0` | 1.5.1 | **Metric detection** (edge case จริง) |
| `f998efc` | docker | Dockerfile + bind mount rules/ |
| `3829400` | chore | fmt cleanup |

---

## 🏗️ สถาปัตยกรรมส่งมอบจริง

```
subject ──► [L0 Normalize] ──► [L1 Mechanical] ──► [L2 Shape] ──►
         [L4 Denylist] ──► [L4 Allowlist] ──► [L4b Metric Check] ──►
         [L3 Shape Verdict] ──► [L5 Final Verdict + QualityTag]
```

**Engine (code, 5+ year lifetime):**
- `src/subject_validator.rs` — 1230 LOC engine
- Layer 0: NFC + zero-width + NBSP normalize
- Layer 1: 11 mechanical defect families (Empty/Ctrl/HTML/Template/RTL/Emoji/Length/...)
- Layer 2: 23 shapes (Acronym/Ticker/Slug/Date/NumberLed/Thai/Verb/TitleCase/...)
- Layer 5: Verdict combiner + ambiguous-acronym boost + unknown counter

**Data (TOML, review annually):**
- `rules/subject_rules.toml` — 24 shape → verdict mappings
- `rules/subject_allowlist.toml` — NYSE/NASDAQ/SET tickers + user_overrides + corporate_suffixes
- `rules/subject_denylist.toml` — headings + stopwords + llm_bleed + ambiguous_acronyms + **metric_heads + metric_single_words** (Phase 1.5.1)

**Integration:**
- `QualityChecker` inject `Arc<SubjectValidator>` ที่ boot
- `ProposeInferenceCommand.subject_validator_version` audit field (`#[serde(default)]` for backward compat)
- `ConsoleApiState.subject_validator` ผ่าน builder `.with_subject_validator()`
- Web UI: 8 new QualityTagKind variants + Critical meta highlight ("N critical")

---

## 🔬 Research ที่ใช้ตัดสินใจ (Don't reinvent the wheel)

1. **Wikidata pattern** — constraint engine (code) separated from constraint definitions (data edited by community). เราใช้ Layer 0-2,5 = code / Layer 3-4 = TOML
2. **Microsoft GraphRAG** (arxiv 2404.16130) — two-pass extraction แต่เราเลือก validator ก่อน (Phase 4 GraphRAG ทีหลัง)
3. **spaCy POS tagging** (PROPN vs NOUN) — เราทำ metric detection ผ่าน head-noun membership แทน (no NER dep ใน Phase 1.5)
4. **ESLint naming-convention filter/modifiers pattern** — เราใช้ shape gating (เฉพาะ TitleCase/LowercaseNoun/Plain เท่านั้นที่ผ่าน metric check)
5. **Andy Matuschak evergreen notes** — durability มาจาก tiering + compaction, ไม่ใช่ infinite accumulation

---

## ⚠️ Known Limitations (ติดตาม Phase ถัดไป)

| Limitation | Phase แก้ | Trigger |
|-----------|---------|---------|
| Thai semantic check (Phase 1.5 accept structural เท่านั้น) | Phase 2 — Thai NER | เมื่อ Thai subjects เยอะจริง |
| Cyrillic homoglyph hard reject | Phase 2 — MixedScript reject | เมื่อ prompt injection เริ่มเป็นปัญหา |
| LLM-as-judge สำหรับ Plain/Unknown | Phase 3 | เมื่อ Unknown frequency > 5% |
| GraphRAG two-pass extraction | Phase 4 | เมื่อ validator ยังไม่พอ |
| `"Shares outstanding"` — last token "outstanding" ไม่อยู่ใน metric_heads | (manual TOML edit) | เมื่อเจอบ่อย |
| `"WACC"` (all-caps) — classify เป็น Acronym หลุด metric check | Phase 2 — context-aware | เมื่อเจอบ่อย |

---

## 📊 ตัวเลขสุดท้าย

| เมตริก | จำนวน |
|--------|-------|
| Workspace tests ผ่าน | 1239 ✅ |
| Pre-existing failure (unrelated) | 1 (`semantic_vertical_slice`) |
| Subject validator unit tests | 71 ✅ |
| Subject validator integration tests | 17 ✅ |
| Live API tested edge cases | 17 ✅ (100% match) |
| Commits ทั้งหมด | 11 |
| Dep ใหม่ | 2 (`unicode-normalization`, `unicode-segmentation`) |
| LOC ใหม่ | ~2400 (engine + tests) + ~100 (TOML) + ~30 (UI) |

---

## 🚀 ขั้นตอนถัดไป (recommendations)

ตามลำดับ ROI:

1. **Phase 2 — Thai NER** (เมื่อ Thai subjects เริ่มเยอะ): ใช้ WangchanBERTa หรือ CRF สำหรับ ThaiPure/ThaiLatinMixed semantic validation
2. **Phase 3 — LLM-as-judge** (เมื่อ Unknown frequency เกิน 5%): route DeferToLLM cases ผ่าน glm-4.6 batch
3. **Wire `pending_proposals_subjects()` API** เพื่อรัน `tests/subject_validator_182_regression.rs` อัตโนมัติใน CI
4. **Phase 4 — GraphRAG two-pass** (เมื่อ validator ยังไม่พอ): extraction-side fix
5. **Fix pre-existing `semantic_vertical_slice` test** (out of scope Phase 1.5) — `src/api.rs` มี `crate::semantic::` imports ที่ break architecture boundary test

---

## 🔐 Security

- ✅ Adversarial inputs (HTML/Template/RTL/Emoji/Control chars) ถูก reject Critical
- ✅ Prompt injection patterns (`<script>`, `${}`, `{{`, `<%`) ถูก block
- ✅ Zero-width chars strip กัน dedup bypass
- ✅ ไม่มีการ log secrets/env ในระหว่าง development
- ⚠️ Pre-existing failure ใน `semantic_vertical_slice` บอกว่ามี `crate::semantic::` import ใน `src/api.rs` — เป็น architecture boundary concern ไม่ใช่ security risk แต่ควร track

---

## 🎓 Lessons learned

1. **Live testing caught a spec bug** — `"Risk-free rate"` (TitleCase) หลุด LowercaseNoun check. Research พบ metric_heads pattern (head-noun membership จาก spaCy POS concept). แก้ใน Phase 1.5.1 — ทำให้ coverage เพิ่มจาก 11 → 39 rejects
2. **TOML key case inconsistency** — Rust parse_shape รับทั้ง snake_case และ PascalCase (production TOML ใช้ PascalCase; spec เขียน snake_case). ปรับให้รับทั้งสอง — permissive for ops
3. **ExtractionAudit dead code** — spec เดิมคิดว่า audit field ไปใน ExtractionAudit แต่ verify จริงใน codebase พบว่า production path ใช้ `ProposeInferenceCommand.prompt_version` ตรงๆ
4. **`#[serde(deny_unknown_fields)]` backward compat** — add field ต้องใส่ `#[serde(default)]` ไม่งั้น historical JSON break
5. **Pre-existing test failure** — subagent เจอ `semantic_vertical_slice` fail ตั้งแต่ก่อน Phase 1.5 เริ่ม. verify ด้วย `git stash` บน parent commit. สำคัญที่จะแยกจากงานเรา
