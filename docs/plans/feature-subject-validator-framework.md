> ⚠️ **SUPERSEDED (2026-07-21):** This framework spec was refined into
> `docs/plans/subject-validator-v1-spec.md` with implementation plan at
> `docs/plans/subject-validator-v1-implementation-plan.md`. Key changes from
> this doc:
> - Engine/data separation (Wikidata pattern) — Layer 0-2,5 = code, Layer 3-4 = TOML
> - 3 durability additions: SUBJECT_VALIDATOR_VERSION, last_reviewed stamps, Unknown+counter
> - Phase 1.5 verdict for Plain/Unknown shape: accept_info (not defer_to_llm)
> - ExtractionAudit is dead code in production; audit field goes on ProposeInferenceCommand
>
> Keep this doc for the 7-families taxonomy reference.

---

# Spec — Subject Validity Framework (Phase 1.5)

> Created: 2026-07-20
> Status: **DRAFT — awaiting review**
> Branch target: `vnext/phase-0`
> Predecessors:
> - `docs/problems/2026-07-20-inbox-review-clarity.md` (problem)
> - `docs/problems/2026-07-20-inbox-review-clarity-rootcause.md` (root cause + Research A)
> - `docs/problems/2026-07-20-entity-options-analysis.md` (initial options — superseded by this spec)
> Source research: "Entity-Name Validation: A Production Framework" (subagent report, 21 sources)

## TL;DR

แทนที่ heuristic 3 ตัวตื้นๆ ด้วย **multi-layer validator framework** ที่ครอบ edge cases ครบ (ผู้ใช้ติว่า heuristic เดิมคิดแค่เคสที่เจอ ไม่เผื่อเคสอื่น). Architecture เลียนแบบ production systems (Wikidata constraints, ESLint naming-convention, spaCy EntityRuler): **normalize → mechanical-reject → shape-classify → shape-conditional-rule → allowlist/denylist → confidence-scored verdict**.

Phase 1.5 ครอบ ~95% ของ failure modes แบบ deterministic (no LLM call, no NER dep). Thai structurally accept. Phase 2 เพิ่ม Thai NER. Phase 3 เพิ่ม LLM-as-judge.

---

## ทำไมต้อง framework ไม่ใช่ heuristic เดียว

User pushback (verbatim):
> "เห็นจากตัวอย่างมันหยาบไปมาก ทำไมไม่คิดเผื่อเคสอื่นๆๆด้วย ไม่ใช่แค่เอาเท่าที่มีสิ ต้องคิดเผื่อ"

Production systems ทุกตัว (Wikidata, DBpedia, Schema.org, OSM, spaCy, Diffbot, typescript-eslint) ใช้ multi-layer pattern ไม่ใช่ single regex:
- ESLint `naming-convention`: selector dispatch + format + custom regex + filter escape + modifiers
- Wikidata: 11 constraint types แยกกัน (format, one-of, type, value-type, distinct, range, ...)
- spaCy EntityRuler: token attributes (TEXT/LOWER/SHAPE/IS_UPPER/...) + pattern combination

Single heuristic พังเพราะ edge cases มากเกิน (research พบ **7 families × ~40 sub-patterns**).

---

## 7 Families ของ failure modes (research-backed)

ทั้งหมดที่ entity-name field พังได้ — ไม่ใช่แค่ที่เห็นใน 182 proposals:

### Family A — Wrong grammatical category (common LLM failure)
A1 Common noun single ("rate"), A2 Common noun phrase ("Risk-free rate"), A3 Verb ("increased"), A4 Verb-led fragment ("Produced deliverable"), A5 Adjective/quantifier ("4.470B"), A6 Question, A7 Sentence, A8 Pronoun ("It"), A9 Determiner only ("the")

### Family B — Right category, wrong kind of name
B1 Section heading ("DCF Assumptions"), B2 Entity+metric fused ("CATL market share"), B3 Slug ("thai-shipping-bf-report"), B4 Filename ("peers_primary_20F_data_2026"), B5 URL, B6 Code, B7 Date/time, B8 Number-led

### Family C — Structural defects (mechanical)
C1 Empty, C2 Whitespace only, C3 Punctuation only, C4 Too long, C5 Too short (1 char), C6 Untrimmed, C7 Internal whitespace runs

### Family D — Character-class defects
D1 Mixed-script noise ("การเงิน finance 2026"), D2 Control chars, D3 Emoji, D4 Tab/newline, D5 Wiki markup leak

### Family E — Semantic defects (need context)
E1 Description masquerade ("China's largest battery maker"), E2 Possessive ("Tesla's CFO"), E3 Ambiguous acronym ("BAT"), E4 Metric-value fused, E5 Multi-entity list ("CATL, BYD, LG"), E6 Temporal drift (Twitter→X), E7 Currency-led

### Family F — Adversarial / security
F1 Prompt injection, F2 HTML injection, F3 Path traversal, F4 Template injection, F5 Null bytes, F6 Homoglyph (Cyrillic С), F7 RTL override

### Family G — LLM-specific pathologies (research's "unknown unknowns")
G1 Subject = prompt example (LLM copies few-shot), G2 Subject = document H1, G3 Subject = placeholder ("<entity>", "[SUBJECT]", "TBD", "N/A"), G4 Subject = prior claim's predicate (CoT leak), G5 Subject = "Unknown"/"N/A"/"TBD"

---

## Architecture — 6-layer validator

```
subject ───► [L0: Normalize] ───► [L1: Mechanical hard-fail]
                                            │
                                            ▼
                                    [L2: Shape Classifier]
                                            │
                                            ▼
                                    [L3: Shape-conditional rules]
                                            │
                                            ▼
                                    [L4: Allowlist/Denylist overrides]
                                            │
                                            ▼
                                    [L5: Confidence + Verdict]
                                            │
                                            ▼ (Phase 3 only)
                                    [L6: LLM-as-judge for ambiguous]
```

### Layer 0 — Normalize (always runs, never rejects)
- NFC normalize (é = e+́  → é)
- Strip zero-width chars (U+200B/C/D, U+FEFF) — ไม่งั้น dedup พัง
- Replace NBSP (U+00A0) + thin/em space → regular space — `str::trim()` ไม่จับ NBSP
- Collapse internal whitespace runs to single space
- Trim
- Severity: `info` (log ว่า normalization fired)

### Layer 1 — Mechanical hard-fail (Critical, deterministic)
Catch Family C/D/F ก่อน shape classification:
- Empty / whitespace-only / punctuation-only → `BadSubjectEmpty` (Critical)
- Control chars (Cc) → `BadSubjectStructural` (Critical)
- Tab/newline/CR in field → `BadSubjectStructural`
- Wiki markup leak (`[[`, `]]`, `==`, `'''`) → `BadSubjectStructural`
- HTML injection (`<script>`, `<`, `>`) → `BadSubjectAdversarial`
- Template injection (`${`, `{{`, `%{`, `<%`) → `BadSubjectAdversarial`
- RTL override (U+202E) → `BadSubjectAdversarial`
- Emoji (Unicode Emoji property ranges) → `BadSubjectStructural`

**Verdict:** any hit → Critical, no further layers run

### Layer 2 — Shape Classifier (the heart of the framework)
Produce `SubjectShape` enum 22 variants (Layer 3 dispatch on this):

```rust
enum SubjectShape {
    Empty, Slug, Filename, Url, Date, TimeExpr,
    NumberLed, CurrencyLed, Sentence, Question,
    ThaiPure, ThaiLatinMixed,
    Ticker, Acronym, TitleCase, LowercaseNoun,
    VerbLed, Demonstrative, MultiEntity, Possessive,
    WikiMarkup, Placeholder, Plain,
}
```

Detection order (first match wins; cheap tests first):
1. Acronym (all-caps 2-8 chars, no lowercase)
2. Ticker (`^[A-Z0-9]{1,6}(\.[A-Z]{1,4})?$`)
3. Slug (`^[a-z0-9]+(-[a-z0-9]+){1,}$`)
4. Filename (`^[a-z0-9]+(_[a-z0-9]+){1,}$`)
5. URL (`^(https?://|www\.|ftp://)`)
6. Date (`^\d{4}-\d{2}(-\d{2})?|^Q[1-4]\s+\d{4}$`)
7. CurrencyLed (`^[¥$€£฿]\s*[\d,.]+`)
8. NumberLed (`^[\d,.]+\s*[BMK]?\b`)
9. Question (ends `?` or starts with interrogative)
10. MultiEntity (contains `,` or ` and ` joining capitalized tokens)
11. Possessive (contains `'` or ends `'s`)
12. WikiMarkup (`==`, `'''`, `[[`)
13. Placeholder (`N/A`, `TBD`, `unknown`, `[SUBJECT]`, `<entity>`)
14. Thai detection: any char in `\u{0E00}-\u{0E7F}`
    - has_thai && !has_latin → `ThaiPure`
    - has_thai && has_latin → `ThaiLatinMixed`
15. VerbLed (first token in verb_cue list: produced, filed, grew, ...)
16. Demonstrative (starts with: it, this, that, these, those, the)
17. LowercaseNoun (all alphabetic chars lowercase)
18. TitleCase (every word starts uppercase)
19. Sentence (ends `.` and not all-caps)
20. Plain (fallback)

### Layer 3 — Shape-conditional rules
| Shape | Verdict | Severity | Rationale |
|-------|---------|----------|-----------|
| Empty, Slug, Filename, Url, Date, TimeExpr | Reject | Critical | B3/B4/B5/B7 |
| NumberLed, CurrencyLed | Reject | Critical | B8 — belongs in `value` |
| Question, Sentence, VerbLed, Demonstrative | Reject | Critical | A3-A8 |
| LowercaseNoun | Reject | Critical | A1/A2 — most common LLM fail |
| WikiMarkup, Placeholder | Reject | Critical | D5/G3-G5 |
| Possessive | SoftFlag | Warning | E2 — split into subject+predicate |
| MultiEntity | SoftFlag | Warning | E5 — split into N claims |
| ThaiPure | Accept if len 2-60 | Info | Phase 1 can't eval Thai semantics |
| ThaiLatinMixed | Accept if Thai+Latin only | Info | e.g. "บมจ. ปตท. (PTT)" |
| Ticker | Accept | Info | strong positive |
| Acronym | Accept + ambiguous flag if len≤3 or in ambiguous list | Info | E3 |
| TitleCase | Accept if ≤6 tokens AND not in heading_denylist | Info/Warning | B1 defense |
| Plain | DeferToLLM | Warning | couldn't classify confidently |

### Layer 4 — Allowlist / Denylist overrides (escape hatches)

**Allowlists:**
- `entity_canonical_subjects_owned()` (existing API at `api.rs:989`) — subjects ที่ confirm แล้ว = valid เสมอ, ฟรี
- Ticker allowlist (NYSE/NASDAQ/SET symbols) — boost Acronym → definite
- User-configurable allowlist file `subject_allowlist.txt`

**Denylists:**
- Heading denylist: `{DCF Assumptions, Risk Factors, Executive Summary, Financial Highlights, Sensitivity Analysis, ...}`
- Stopword denylist: `{the, a, an, and, or, of, it, this, that}`
- LLM-bleed denylist: `{N/A, TBD, unknown, none, null, [SUBJECT], <entity>, ___, unspecified, not specified, unknown entity}` (Family G)

### Layer 5 — Confidence + Verdict

```rust
enum SubjectVerdict {
    Accept,                    // strong positive shape, no defect
    AcceptWithInfo(Vec<Tag>),  // ambiguous acronym, or Thai structurally accepted
    SoftFlag(Vec<Tag>),        // Possessive, MultiEntity — split suggestion
    DeferToLLM(Vec<Tag>),      // Plain shape, or conflicting signals
    Reject(Vec<Tag>),          // Critical defect
}
```

Decision rule:
1. Any Critical → Reject
2. Any Warning from non-recoverable shape → Reject
3. Any Warning from Possessive/MultiEntity → SoftFlag
4. Plain shape → DeferToLLM (Phase 3 routes to glm-4.6)
5. Acronym ambiguous → AcceptWithInfo
6. Else → Accept

---

## การ integrate เข้า codebase ปัจจุบัน

### 1. `src/quality.rs` — add `check_subject_shape` function

```rust
// Existing: check_vague_predicate อยู่ที่ประมาณ line 203-228
// เพิ่ม check_subject_shape ข้างๆ (ใหม่)

fn check_subject_shape(input: &QualityCheckerInput<'_>, tags: &mut Vec<QualityTag>) {
    let raw = &input.proposal.subject;
    let normalized = normalize_subject(raw);  // Layer 0
    // Layer 1: mechanical
    if let Some(defects) = check_subject_mechanical(&normalized) {
        tags.extend(defects);
        return;  // Critical หยุดที่นี่
    }
    // Layer 2: shape
    let shape = classify_shape(&normalized);
    // Layer 3: shape-conditional
    let shape_tags = shape_conditional_rules(&shape, &normalized);
    // Layer 4: allowlist/denylist overrides
    let final_tags = apply_overrides(shape_tags, &normalized, input);
    // Layer 5: verdict
    tags.extend(final_tags);
}
```

Register ใน `QualityChecker::check_deterministic` (เรียกต่อจาก `check_vague_predicate`).

### 2. `src/quality.rs` — add 7 ใหม่ `QualityTagKind` variants

```rust
pub enum QualityTagKind {
    // existing...
    BadSubjectEmpty,         // C1/C2
    BadSubjectStructural,    // C3-C7, D, F
    BadSubjectShape,         // A1-A9, B1-B8
    BadSubjectLength,        // too long/short
    BadSubjectMixedScript,   // D1
    BadSubjectAdversarial,   // F1-F7
    SubjectAmbiguousAcronym, // E3 — info
    SubjectNeedsContext,     // DeferToLLM (Phase 3)
}
```

### 3. Cargo.toml — add 2 deps

```toml
unicode-segmentation = "1.12"   # UAX#29 word boundaries
unicode-normalization = "1.24"  # NFC for homoglyph detection
```

(`regex` มีอยู่แล้วผ่าน existing dep tree; `once_cell` หรือ `LazyLock` สำหรับ compiled regex)

### 4. Tests — comprehensive coverage

Unit tests (ใน `src/quality.rs::tests`):
- Layer 0: NFC normalize, NBSP handling, zero-width stripping
- Layer 1: 1 test per mechanical check (8 tests)
- Layer 2: 1 test per shape variant (22 tests)
- Layer 3: 1 test per shape→verdict mapping (15+ tests)
- Layer 4: allowlist hit/miss, denylist hit/miss

Integration tests (`tests/subject_validator_v1.rs`):
- 182 จริงจาก inbox → expected: ~24 Tier-4 common nouns flagged Critical, ~12 Tier-5 slugs flagged Critical, ~96 clean silent
- Edge cases: Thai ("บมจ. ปตท."), ticker ("AAPL"), ambiguous acronym ("BAT"), LLM-bleed ("<entity>"), possessive ("Tesla's CFO"), multi-entity ("CATL, BYD, LG")
- Adversarial: homoglyph (Cyrillic С), RTL override, prompt-injection

---

## Expected outcome บนข้อมูลจริง 182 proposals

| Tier | จำนวน | Phase 1.5 verdict |
|------|------|-------------------|
| Tier 1 (Pure entity: CATL, BYD) | ~96 | Accept silent ✅ |
| Tier 2 (Entity + qualifier) | ~20 | Accept + Info |
| Tier 3 (Entity + metric) | ~4 | Warning (SoftFlag) |
| **Tier 4 (Common noun)** | **~24** | **Critical (Reject)** |
| **Tier 5 (Slug)** | **~12** | **Critical (Reject)** |
| Edge cases (placeholder, etc.) | ~2 | Critical |

**ผลที่คาดหวาน:**
- ~38 proposals ที่ชัดเจนว่าผิด → flag Critical (user เห็นชัดว่าต้อง reject ไม่ใช่ approve)
- ~24 proposals ambiguous → SoftFlag (split suggestion)
- ~96 proposals ที่สะอาด → silent (ลด noise)
- ลดจาก 77% taxonomy_drift noise → ประมาณ 25-30% meaningful subject flags

---

## Phase boundaries

| Phase | Scope | เวลา |
|-------|-------|------|
| **1.5** (spec นี้) | Layer 0-5, Eng+Thai structural | 3-4 วัน |
| 2 | + Thai NER (CRF model) สำหรับ ThaiPure/ThaiLatinMixed | ~1 สัปดาห์ |
| 3 | + LLM-as-judge สำหรับ DeferToLLM cases (route ~5% ผ่าน glm-4.6) | 2-3 วัน |

---

## Risk + Mitigation

| Risk | ระดับ | Mitigation |
|------|------|-----------|
| False positive บน legitimate entity ที่ shape ไม่ match | ปานกลาง | Layer 4 allowlist (ledger + ticker + user file) เป็น escape hatch |
| Thai ที่ Phase 1.5 accept structurally แต่จริงๆเป็น common noun | ปานกลาง | Phase 2 เพิ่ม Thai NER; Phase 1.5 บอก `info` severity ไม่ใช่ accept เงียบ |
| Shape classifier ผิดพลาด edge case | ต่ำ | แต่ละ shape มี unit test; integration test รันบน 182 จริง |
| Regex compile ช้าตอน startup | ต่ำมาก | `LazyLock<Regex>` compile ครั้งเดียว |
| Dependency ใหม่ (unicode-segmentation, unicode-normalization) | ต่ำ | Both mature, widely-used, Apache/MIT |

---

## Open questions สำหรับ review

1. **Scope Phase 1.5** — all 6 layers หรือ layer 0-3 พอ? (Layer 4-5 add complexity แต่ก็คุ้ม)
2. **Critical vs Warning** สำหรับ slug/common-noun — Critical เหมาะไหม? (UI แดงเข้ม บังคับให้สนใจ)
3. **Heading denylist** — เริ่มต้นใส่อะไรบ้าง? (DCF Assumptions, Risk Factors, Executive Summary, Financial Highlights, Sensitivity Analysis พอ?)
4. **LLM-bleed denylist** — ครบไหม? (`N/A, TBD, unknown, none, null, [SUBJECT], <entity>, ___, unspecified, not specified, unknown entity`)
5. **Ticker allowlist** — sync จาก NYSE/NASDAQ/SET feed หรือใส่ manual ทีละ ticker?
6. **Phase 2 (Thai NER) เร็วเกินไปไหม** — หรือ Phase 1.5 accept structurally พอสำหรับตอนนี้?

---

## Sources (21, from research report)

1. Wikidata property constraints — https://www.wikidata.org/wiki/Wikidata:WikiProject_property_constraints
2. WikibaseQualityConstraints extension — https://www.mediawiki.org/wiki/Extension:WikibaseQualityConstraints
3. Wikipedia:Article titles (WP:TITLE) — https://en.wikipedia.org/wiki/Wikipedia:Article_titles
4. Schema.org Thing + name — https://schema.org/Thing
5. CoNLL-2003 NER — https://www.clips.uantwerpen.be/conll2003/ner/
6. OntoNotes 5 entity tagset — https://catalog.ldc.upenn.edu/LDC2013T19
7. spaCy NER annotation (BILUO) — https://spacy.io/api/annotation#named-entities
8. spaCy EntityRuler — https://spacy.io/api/entityruler
9. OpenStreetMap Key:name — https://wiki.openstreetmap.org/wiki/Key:name
10. DBpedia publications (Lehmann 2015) — https://www.dbpedia.org/resources/publications/
11. typescript-eslint naming-convention — https://typescript-eslint.io/rules/naming-convention/
12. ESLint camelcase — https://eslint.org/docs/latest/rules/camelcase
13. Rust char primitives — https://doc.rust-lang.org/std/primitive.char.html
14. unicode-segmentation (UAX#29) — https://docs.rs/unicode-segmentation
15. unicode-normalization (NFC) — https://docs.rs/unicode-normalization
16. PyThaiNLP — https://github.com/PyThaiNLP/pythainlp
17. WangchanBERTa — https://huggingface.co/airesearch/wangchanberta-base-att-spm-uncased
18. rust-bert — https://github.com/guillaume-be/rust-bert
19. OWASP Input Validation Cheat Sheet — https://cheatsheetseries.owasp.org/cheatsheets/Input_Validation_Cheat_Sheet.html
20. Unicode TR #29 — https://unicode.org/reports/tr29/
21. Microsoft GraphRAG (arxiv 2404.16130) — https://arxiv.org/abs/2404.16130

---

## Working rules สำหรับ execution

1. หลัง user approve spec → execute Phase 1.5 (3-4 วัน)
2. แต่ละ layer implement + test แยก — layer 0 → layer 1 → layer 2 → ...
3. รัน integration test บน 182 จริงก่อน close phase
4. ห้ามใช้ embedding (GOAL §2)
5. ห้ามใช้ NER library ใน Phase 1.5 (Phase 2 เท่านั้น)
6. Severity Critical = red chip ใน UI, Warning = orange, Info = blue
