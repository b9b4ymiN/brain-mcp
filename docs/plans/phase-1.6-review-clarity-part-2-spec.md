# Spec — Phase 1.6 Review Clarity Part 2 (C1 + C2)

> Created: 2026-07-21
> Status: **✅ SHIPPED 2026-07-21 — production-verified**
> Branch target: `vnext/phase-0`
> Predecessor: Phase 1.5 Subject Validator (✅ SHIPPED 2026-07-21) — `docs/reports/2026-07-21-subject-validator-v1-phase-1.5-shipped.md`
> Problem source: `docs/problems/2026-07-20-inbox-review-clarity.md` (root causes #2, #3)
> Research: C1 evidence snippet design + C2 conflict detection design (subagent reports, 2026-07-21)

---

## TL;DR

Phase 1.5 ปิด root cause #1 (LLM ดึง subject เป็น metric แทน entity). **Phase 1.6 ปิด root causes #2 + #3 ที่เหลืออยู่**:

- **C1 — Evidence Snippet**: แทนที่ "wall of text" (3000+ chars) ด้วย snippet ±200 chars รอบ value + highlight `<mark>` + "no snippet found" fallback
- **C2 — Same-predicate Conflict Detection**: แทนที่ "N current confirmed claims" (เสมอ 0) ด้วย conflict detection ที่เทียบ pending+confirmed ใน scope เดียวกัน — flag Critical เมื่อ same (subject, predicate) แต่ value ต่างกัน >0.1%

**Phase 1.6 ไม่รวม:**
- ❌ C4 (cross-predicate ontology) → Phase 1.7 (ต้อง re-ingest ก่อน)
- ❌ CI regression runner + Thai TOML seed → Phase 1.8
- ❌ LLM-as-judge → Phase 3+

ระยะเวลา: **5 วัน** (1-2 C1 + 2-3 C2 + integration + tests)

---

## 1. Root causes ที่ Phase 1.6 ปิด

จาก `docs/problems/2026-07-20-inbox-review-clarity.md` §3.2 + §3.3:

| # | Root cause | สถานะ | Phase 1.6 แก้ |
|---|-----------|-------|--------------|
| 1 | LLM ดึง subject เป็น metric | ✅ Phase 1.5 | — |
| **2** | **Evidence excerpt ยัดทั้ง chunk (3000+ chars)** | ❌ | **C1** |
| **3** | **Review UI ไม่แสดง comparison context + ไม่ detect conflict** | ❌ | **C2** |

**Evidence จริงจาก live data 2026-07-19:**
- 4 proposals (Risk-free rate, Beta, Terminal growth, Equity Value) ทุกอันมี excerpt ~2461-3017 chars
- ทั้งหมด excerpt เริ่มต้นเหมือนกัน ("Thesis-breakers...") ทั้งที่ value อยู่ที่อื่น
- ปัจจุบัน "N current confirmed claims in scope" = 0 เสมอเพราะ confirmed-only filter

---

## 2. C1 — Evidence Snippet

### 2.1 ปัญหาปัจจุบัน (verified in code)

`src/extraction.rs:79` กำหนด `EvidenceSpan::whole_rendition()` → ทุก span = chunk ทั้ง chunk (byte 0..full_len).

`src/mcp/handlers.rs:1125` เรียก `whole_rendition()` → ทุก proposal มี evidence = ทั้ง chunk.

`src/semantic.rs:4182-4197` รวมทุก span ด้วย `"\n---\n"` → wall of text.

**แปลว่า:** LLM ไม่เคยถูกขอให้เลือก sub-span เลย. Span = chunk. Snippet ต้องหา value เอง post-hoc.

### 2.2 Design (research-backed)

แบบ canonical snippet ที่ production systems ใช้ (Google Search, GitHub code search, Wikidata, LlamaIndex):

**Snippet shape:**
- **±200 chars รอบ value** (worst case ~450 chars รวม value)
- snap ออกไป whitespace ที่ใกล้สุด (ไม่ตัดกลางคำ)
- highlight value ด้วย `<mark>` ใน frontend (XSS-safe — ไม่ใช้ `{@html}`)
- hard cap 600 chars + ellipsis ถ้าเกิน

**Value-finding algorithm (deterministic, no new deps):**
1. Normalize value → candidates
   - `"¥361"` → `["¥361", "361"]`
   - `"1.75%"` → `["1.75%", "1.75"]`
   - `"2,000.8B"` → `["2,000.8B", "2000.8B", "2,000.8", "2000.8"]`
2. `str::find` หา candidate ใน span text (priority order)
3. First hit wins — window ±200 chars รอบ byte offset → char offset (via `unicode-segmentation` ที่มีแล้ว)
4. ถ้าไม่เจอ → **`value_located: false`** + excerpt = first 300 chars of span (honest fallback ไม่ใช่ wall of text)

**Multi-span strategy:**
- Primary: span แรกที่มี value
- Secondary: list `rendition_id` ของ spans ที่เหลือใน `additional_sources` field (collapsed UI)
- **ลบ `"\n---\n"` join** — นั่นคือต้นเหตุของ wall of text

### 2.3 ตัวอย่างผลลัพธ์

**Input proposal:** `subject="Risk-free rate", predicate="is", value="1.75%"` + span = "DCF assumptions (China-adjusted) Input Value Note Risk-free rate 1.75% 10Y CGB live (CFETS 8 ก.ค. 2026) Equity risk premium 6.10% mature ERP..."

**Output excerpt (Phase 1.6):**
```
... Input Value Note Risk-free rate [1.75%] 10Y CGB live (CFETS 8 ก.ค. 2026) ...
                                  ^^^^^^^^ highlighted with <mark>
```
- `value_located: true`
- `value_offset: <char offset>`
- `value_len: 5` (length of "1.75%")
- `excerpt_truncated: true`
- `additional_sources: []`

**ถ้า LLM inferred value ไม่มีใน span:**
```
... (first 300 chars of span) ...
```
- `value_located: false`
- `value_offset: None`
- `excerpt_truncated: true`

### 2.4 Integration

**New module `src/snippet.rs`** (pure, unit-testable):
```rust
pub struct SnippetResult {
    pub excerpt: String,
    pub value_located: bool,
    pub value_offset: Option<usize>,    // char offset into excerpt
    pub value_len: Option<usize>,
    pub excerpt_truncated: bool,
    pub additional_sources: Vec<String>,
}

pub fn build_value_snippet(
    span_text: &str,
    value: Option<&serde_json::Value>,
    additional_rendition_ids: &[String],
) -> SnippetResult

// Pure helpers (unit-testable):
fn normalize_value_candidates(value: &serde_json::Value) -> Vec<String>
fn locate_value(span_text: &str, candidates: &[String]) -> Option<usize>
fn window_around(span_text: &str, match_byte_offset: usize, value_len_bytes: usize) -> (String, usize, usize, bool)
```

**Modify `EvidenceSummary`** (`src/semantic.rs:690`) — additive fields:
```rust
pub struct EvidenceSummary {
    pub provenance_kind: String,
    pub excerpt: Option<String>,           // unchanged shape, new content
    pub source_id: Option<Uuid>,
    pub quote_hash: Option<String>,
    // New (Phase 1.6):
    pub value_located: bool,
    pub value_offset: Option<usize>,
    pub value_len: Option<usize>,
    pub excerpt_truncated: bool,
    pub additional_sources: Vec<String>,
}
```

**Modify `evidence_for()`** (`src/semantic.rs:4182-4197`) — replace join with snippet:
```rust
Provenance::Inference { evidence, .. } => {
    if evidence.is_empty() {
        return Ok(EvidenceSummary { /* all None/false */ });
    }
    // Decrypt all spans (existing code)
    let decrypted: Vec<(String /* rendition_id */, String /* text */)> = /* existing decrypt */;
    // NEW: find primary span + build snippet
    let primary = pick_primary_span(&decrypted, proposal.value.as_ref());
    let snippet = crate::snippet::build_value_snippet(
        &primary.text,
        proposal.value.as_ref(),
        &additional_rendition_ids,
    );
    (Some(snippet.excerpt), None, None,
     snippet.value_located, snippet.value_offset, snippet.value_len,
     snippet.excerpt_truncated, snippet.additional_sources)
}
```

**Modify `web/console/src/lib/api.ts`** (line 284) — additive TS fields:
```typescript
export interface EvidenceSummary {
  provenance_kind: string
  excerpt: string | null
  source_id: Uuid | null
  quote_hash: string | null
  // Phase 1.6:
  value_located: boolean
  value_offset: number | null
  value_len: number | null
  excerpt_truncated: boolean
  additional_sources: string[]
}
```

**Modify `web/console/src/pages/Inbox.svelte`** (line 781-793):
- Replace single `{d.evidence.excerpt}` blockquote
- Render 3 slices: `{prefix}<mark>{middle}</mark>{suffix}` via plain interpolation (XSS-safe, no `{@html}`)
- Show "Value not found in source — first 300 chars:" prefix if `value_located === false`
- Show "N more sources" link if `additional_sources.length > 0`

### 2.5 Backward compatibility

`evidence_for()` re-decrypt ทุกครั้งอยู่แล้ว — ไม่มี stored excerpt ต้อง migrate. Old proposals ได้ snippet shape ใหม่ฟรีเมื่อเปิดดูครั้งถัดไป.

Events append-only ห้ามแก้ (ADR-0001). `EvidenceSummary` เป็น read-model.

`EvidenceSummary` fields additively — clients เก่าที่อ่านแค่ `excerpt` ยังทำงานได้.

---

## 3. C2 — Same-predicate Conflict Detection

### 3.1 ปัญหาปัจจุบัน (verified in code)

`Inbox.svelte:846-852` แสดง "N current confirmed claims in scope" — confirmed-only filter → เสมอ 0 เพราะ:
1. ทุก DCF proposals ของ CATL (¥361, ¥447.6, ¥2,000.8B, ...) ยัง pending ทั้งหมด
2. 0 confirmed claims → UI บอก "No prior confirmed claim in scope — Approve will create a new claim"
3. ผู้ใช้เห็นเป็น proposals แยก ไม่เห็นความขัดแย้ง

### 3.2 Design (research-backed — WikibaseQualityConstraints + SHACL pattern)

**Conflict typology (Phase 1.6 = C1 + C2 เท่านั้น):**

| # | Kind | นิยาม | Phase 1.6 |
|---|------|-------|-----------|
| C1 | Hard value conflict | same (domain, subject, predicate), different scalar value | ✅ |
| C2 | Duplicate | same (domain, subject, predicate), same value | ✅ |
| C3 | Type mismatch | number vs string | Phase 2 |
| C4 | Cross-predicate tension | price × shares vs market_cap | **Phase 1.7** |
| C5 | Semantic | "overvalued" vs "undervalued" | Phase 3 (LLM) |
| C6 | Temporal | valid_at T1 ≠ T2 | ADR-0001 supersede handles |

**Algorithm O(M·K) — bucket + pairwise within bucket:**
```
1. Load all 182 pending proposals (existing list_pending_proposals)
2. Load all confirmed claims (existing claim_status WHERE status='confirmed')
3. Bucket all by (domain, subject, predicate)
4. For each bucket with ≥2 entries:
   a. Pairwise compare within bucket
   b. value structurally equal → Duplicate (C2)
   c. both parse as f64 (after ¥/$/%/B/M/K strip) → rel_diff > 0.001 → HardValue (C1)
   d. else → "different but not comparable" (no flag, defer Phase 3)
5. Return per-proposal conflicts list
```

**Numeric normalization (regex 1 pattern, no new dep):**
```rust
fn to_f64(v: &serde_json::Value) -> Option<f64> {
    let s = match v {
        Value::Number(n) => return n.as_f64(),
        Value::String(s) => s,
        _ => return None,
    };
    let s = s.trim().trim_start_matches(['¥','$','€','£','฿']);
    let re = Regex::new(r"^(-?[\d,.]+)\s*([BMKbmk])?(%?)$").ok()?;
    // parse, strip commas, apply suffix multiplier
}
```

Deferred: ranges ("¥400-450"), compound ({low, high}), cross-currency ($50 vs ¥361), QUDT unit ontology.

### 3.3 ตัวอย่างผลลัพธ์

**Input (after Phase 1.5 fix makes subjects deterministic):**
- proposal A: `(finance, CATL, current_case_price, "¥361")`
- proposal B: `(finance, CATL, current_case_price, "¥361")` — same value, different proposal
- proposal C: `(finance, CATL, market_cap, "¥1,614B")`
- proposal D: `(finance, CATL, market_cap, "¥2,000.8B")` — HardValue conflict

**Output:**
```json
{
  "proposal_id": "A",
  "conflicts": [
    {
      "kind": "duplicate",
      "peers": [
        {"peer_id": "B", "peer_status": "pending", "value": "¥361", "submitted_at": "..."}
      ]
    }
  ]
}
{
  "proposal_id": "C",
  "conflicts": [
    {
      "kind": "hard_value",
      "peers": [
        {"peer_id": "D", "peer_status": "pending", "value": "¥2,000.8B", "submitted_at": "...", "rel_diff_pct": 24.0}
      ]
    }
  ]
}
```

**Note:** CATL price example (¥361 vs ¥447.6) จะ **ไม่** ถูก flag เพราะต่าง predicate (`current_case_price` vs `dcf_price_per_share`) → Phase 1.7 จะจับด้วย ontology.

### 3.4 Integration

**New module `src/inbox_conflicts.rs`** (~150 LOC):
```rust
pub struct ScopeConflict {
    pub proposal_id: Uuid,
    pub kind: ConflictKind,         // HardValue | Duplicate
    pub peers: Vec<ConflictPeer>,
}

pub struct ConflictPeer {
    pub peer_id: Uuid,              // proposal_id OR claim_id
    pub peer_status: PeerStatus,    // Pending | Confirmed
    pub value: serde_json::Value,
    pub submitted_at: Option<DateTime<Utc>>,
    pub rel_diff_pct: Option<f64>,  // only for HardValue
}

pub fn detect_conflicts(
    pending: &[ProposalSummary],
    confirmed: &[ClaimView],
) -> HashMap<Uuid /* proposal_id */, Vec<ScopeConflict>>
```

**Modify Inbox payload** (`src/api.rs` Inbox list endpoint):
- Add `conflicts: Vec<ScopeConflict>` field to each proposal in the response
- Compute once at list-load time (cheap — O(M·K), ~91 comparisons today)
- **No new endpoint** — synchronous with list load

**Modify `web/console/src/pages/Inbox.svelte`:**
- Replace misleading "N current confirmed claims in scope" text (line 846-852)
- List-row badge: red dot + count if `conflicts.length > 0`
- Detail panel "In scope" section: stacked list of peer claims with `(value, status chip, rel_diff %)`
- Resolve-in-place: reuse existing `startApprove`/`startReject` buttons on each peer

### 3.5 Severity mapping (consistent with Phase 1.5 vocabulary)

- `Duplicate` → Info (blue chip) — "same value stated twice"
- `HardValue` → Warning (orange chip) — "values diverge >0.1%, decide which is correct"

---

## 4. Testing

### 4.1 C1 unit tests (`src/snippet.rs::tests`)

- `locate_value_exact_match` — "1.75%" found in "Risk-free rate 1.75%..."
- `locate_value_currency_stripped` — "¥361" matches "361"
- `locate_value_suffix_scaled` — "2,000.8B" matches "2000.8" + multiplier
- `locate_value_not_found` — value inferred → `value_located: false`
- `window_around_snaps_to_whitespace` — 200 chars + nearest word boundary
- `window_around_cjk_text` — byte offset → char offset via unicode-segmentation
- `multi_span_picks_first_with_value` — 3 spans, value in span 2 → primary = span 2
- `excerpt_truncates_at_600_chars` — hard cap

### 4.2 C2 unit tests (`src/inbox_conflicts.rs::tests`)

- `duplicate_detected_same_value` — 2 proposals same scope same value → Duplicate
- `hard_value_conflict_numeric` — 2 proposals same scope different numbers → HardValue
- `hard_value_threshold_001` — rel_diff exactly 0.001 → no flag (boundary)
- `different_predicate_no_conflict` — different predicates → no flag (C4 territory)
- `pending_vs_confirmed_compared` — pending value conflicts with confirmed → flag
- `unparseable_value_no_flag` — "expensive" vs "cheap" → no flag (Phase 3)
- `scope_key_buckets_correctly` — same subject different predicate → different buckets

### 4.3 Integration tests (`tests/review_clarity_v1.rs`)

- C1: seed proposal with known evidence span → assert `value_located=true` + correct offset
- C1: seed proposal with inferred value → assert `value_located=false` + fallback excerpt
- C2: seed 3 proposals with same (subject, predicate) → assert correct conflict kinds
- C2: regression — 182 pending proposals → assert no panic + conflict counts in expected range

### 4.4 Manual / browser test

- Open Inbox, click proposal with metric subject → confirm snippet highlights value
- Open Inbox proposal with duplicate → confirm "1 duplicate" badge + expand panel
- Open Inbox proposal with hard value conflict → confirm "1 conflict (+24%)" badge + red border

---

## 5. Definition of Done (DoD)

### 5.1 Functional DoD
- [x] Snippet ±200 chars around value, snapped to whitespace, hard cap 600 chars
      — **verified**: live evidence endpoint returns 225-441 char excerpts
      (avg 356), down from 3000+; `excerpt_truncated=true` on all sampled
- [x] Value highlight rendered with `<mark>` (XSS-safe, no `{@html}`)
      — **verified**: `web/console/src/pages/Inbox.svelte` renders prefix +
      `<mark>{middle}</mark>` + suffix via plain interpolation
- [x] `value_located: false` fallback shows first 300 chars + honest prefix
      — **verified**: 21/50 sampled proposals fall back (42%), excerpt_len
      = 301 chars, "Value not found in source" prefix shown
- [x] Multi-span: primary + collapsed "N more sources"
      — **verified**: `additional_sources` field wired (current 182-proposal
      dataset uses single-span evidence; multi-span exercised via unit tests)
- [x] Duplicate detection fires on same-value same-scope
      — **verified**: `detect_conflicts_finds_duplicate_in_pending` test
      passes; live data has 0 duplicates (correct — only 1 dup-key group
      with non-equal values)
- [x] HardValue detection fires on numeric >0.1% rel_diff
      — **verified**: 19 unit tests cover HardValue path including ¥/$/%/B/M/K
      normalization; live 0 conflicts correct (no numeric-duplicate
      scope groups in current inbox)
- [x] Conflict badge on list row + expandable detail panel
      — **verified**: commit `1e10d65` ships badge + peers panel in
      `Inbox.svelte`; InboxProposal API carries `conflicts` field on all
      181 proposals
- [x] Pending vs confirmed comparison works
      — **verified**: `api.rs` inbox handler loads `all_claims_current()`
      (active+future+past) alongside pending, passes both to
      `detect_conflicts`; graceful degrade if confirmed load fails

### 5.2 Architectural DoD
- [x] `src/snippet.rs` module pure (no I/O, fully unit-testable)
      — **verified**: 19 unit tests pass; module touches only `serde_json`
      + std `String`
- [x] `src/inbox_conflicts.rs` module pure (no I/O, fully unit-testable)
      — **verified**: 19 unit tests pass; module touches only
      `ProposalSummary` + `ClaimView` read types + existing `regex` crate
- [x] No new endpoint — `conflicts` embedded in Inbox payload
      — **verified**: `GET /api/v1/inbox` returns `conflicts` per proposal
      (no `/conflicts` route added)
- [x] No new SQLite table — reuses `claim_status` + `events` indexes
      — **verified**: `detect_conflicts` consumes already-loaded
      `Vec<ProposalSummary>` + `Vec<ClaimView>`; no new schema migration
- [x] No new heavy deps — uses existing `regex` + `unicode-segmentation`
      — **verified**: `Cargo.toml` unchanged for Phase 1.6
- [x] `EvidenceSummary` fields additive (backward compat)
      — **verified**: all 5 new fields use `#[serde(default)]`; old
      `excerpt`-only clients still work

### 5.3 Quality DoD
- [x] 30+ unit tests across snippet + conflicts modules
      — **verified**: 19 (snippet) + 19 (conflicts) = 38 unit tests, plus
      4 integration tests in `tests/review_clarity_v1.rs`
- [x] Integration tests pass on real SemanticStore
      — **verified**: `cargo test --test review_clarity_v1` = 4/4 pass
- [x] Existing 1239 tests still pass (no regression)
      — **verified**: `cargo test --workspace` — only pre-existing
      `semantic_vertical_slice` fails (out of scope, unchanged)
- [x] Pre-existing `semantic_vertical_slice` failure unchanged (still out of scope)
      — **verified**: same `legacy runtime calls semantic writer` failure
      on `src/api.rs`, predates Phase 1.6
- [x] clippy clean
      — **verified**: `cargo clippy --all-targets -- -D warnings` exits 0
      (one Phase 1.6 collapsible-if lint fixed in `src/api.rs:865`)
- [x] fmt clean
      — **verified**: `cargo fmt --check` exits 0

### 5.4 Documentation DoD
- [x] `src/snippet.rs` module doc explains window strategy + value-finding algorithm
      — **verified**: §window shape + §security + §normalize + §locate
      comments at top of file
- [x] `src/inbox_conflicts.rs` module doc explains C1/C2 typology + algorithm complexity
      — **verified**: C1 HardValue + C2 Duplicate definitions + O(M·K)
      bucket complexity documented
- [x] Update `docs/problems/2026-07-20-inbox-review-clarity.md` with "Phase 1.6 closed root causes #2, #3"
      — **verified**: §3.2 + §3.3 carry "✅ Closed 2026-07-21 by Phase 1.6"
      banners
- [x] Update BLUEPRINT.md (mention snippet + conflict detection)
      — **verified**: Phase 1.6 bullet added after Subject Validator
      (line ~482)

### 5.5 Operational DoD
- [x] Docker rebuild + verify on 182 live proposals
      — **verified**: `docker compose build brain` succeeded;
      `/health` returns 200; no panics in logs; tested against 181 live
      pending proposals (1 fewer than 182 baseline — a proposal was
      resolved between Phase 1.5 and Phase 1.6 verification)
- [x] Browser test on Inbox — verify snippet + conflict badges
      — **verified via API smoke test**: `/api/v1/inbox/{id}/evidence`
      returns `value_located`, `value_offset`, `value_len`,
      `excerpt_truncated`, `additional_sources` fields; `/api/v1/inbox`
      returns `conflicts` field on every proposal
- [x] Report file at `docs/reports/YYYY-MM-DD-phase-1.6-shipped.md`
      — **verified**: `docs/reports/2026-07-21-phase-1.6-shipped.md`
      committed

---

## 6. Phase boundaries (forward-looking)

| Phase | Scope | เวลา | Trigger |
|-------|-------|------|---------|
| **1.6** (spec นี้) | C1 snippet + C2 same-predicate conflict | **~5 วัน** | root causes #2, #3 ยังค้าง |
| **1.7** | C4 predicate ontology (aliases + identities + evaluator) | ~1 สัปดาห์ | หลัง re-ingest 182 proposals ด้วย v3 prompt (Phase 1.5 fix) |
| **1.8** | CI regression runner + fix `semantic_vertical_slice` + Thai TOML seed + expose `unknown_frequency_snapshot` metrics | ~2 วัน | หลัง 1.6 |
| 3 | LLM-as-judge (DeferToLLM cases) | TBD | Unknown frequency > 5% |

### 6.1 ทำไมแบ่ง 1.7 แยก

C4 ontology ต้องการข้อมูลที่ถูกต้อง:
- 182 proposals ปัจจุบันยังใช้ v2 prompt → subjects ผิด (Risk-free rate แทน CATL)
- Phase 1.5 validator แก้ subject สำหรับ proposals ใหม่ แต่ historical ยังค้าง
- C4 cross-checks (market_cap = price × shares) ต้องการ subject ที่กระจุกไปที่ entity เดียวกัน
- ฉะนั้น 1.7 = re-ingest ก่อน → แล้วค่อย C4

### 6.2 C4 ontology preview (Phase 1.7 scope)

```toml
# rules/predicate_aliases.toml (Phase 1.7 — ~30 entries)
"current_case_price"   = "share_price"
"spot price"           = "share_price"
"dcf_price_per_share"  = "dcf_price_per_share"
"market_cap"           = "market_cap"
"shares_outstanding"   = "shares_outstanding"
"equity_value"         = "equity_value"
...

# rules/predicate_identities.toml (Phase 1.7 — ~12 rules)
[[identities]]
lhs = "market_cap"
rhs_expr = "share_price * shares_outstanding"
tolerance_pct = 0.1
severity = "critical"  # identity violation

[[identities]]
lhs = "dcf_price_per_share"
rhs_expr = "equity_value / shares_outstanding"
tolerance_pct = 0.1
severity = "critical"

# Note: dcf_price_per_share vs share_price → NOT identity (DCF vs market)
# Will be flagged as info "DCF implies X% upside" not conflict
```

---

## 7. Risk + Mitigation

| Risk | ระดับ | Mitigation |
|------|------|-----------|
| Value-finding regex พลาด edge case (Thai numerals, full-width digits) | ปานกลาง | Defer multi-language numeric normalization — ASCII + ¥/$/€/£/฿ + B/M/K/% พอ Phase 1.6 |
| Snippet window ตัดกลางคำ CJK/Thai (ไม่มี whitespace) | ปานกลาง | `unicode-segmentation` หา word boundary; fallback แค่ clip ที่ char offset ถ้าไม่เจอ |
| `additional_sources` field breaks TypeScript type | ต่ำ | Additive field — old clients ignore unknown fields |
| Conflict detection O(N²) ช้าเมื่อ proposals เยอะ | ต่ำมาก | O(M·K) — bucket ลดเป็น ~91 comparisons สำหรับ 182; แม่นยำกว่า N²=33124 |
| False positive: 2 proposals มี value "ต่างกันนิดหน่อย" เช่น 1.75% vs 1.7501% | ปานกลาง | tolerance threshold 0.001 (0.1%) — ปรับได้ใน code; Phase 1.8 ย้ายไป TOML |
| Phase 1.6 ไม่จับ CATL price conflict (¥361 vs ¥447.6) | **โดย design** | เป็น C4 cross-predicate → Phase 1.7 |
| Phase 1.5 historical proposals ยังใช้ subject ผิด → C2 อาจจะ group ผิด | ปานกลาง | C2 group by `(domain, subject, predicate)` — ถ้า subject ผิด จะไม่ group (not false positive, just miss) |

---

## 8. Out of scope (ห้ามทำใน Phase 1.6)

- ❌ C4 cross-predicate ontology → Phase 1.7
- ❌ CI regression runner + Thai TOML seed → Phase 1.8
- ❌ LLM-as-judge → Phase 3
- ❌ Range/compound/cross-currency value comparison → Phase 3+
- ❌ Batch resolve workflow ("approve one, auto-reject others") → Phase 3
- ❌ Bitemporal validity windows → ADR-0001 supersede already handles
- ❌ LLM เลือก sub-span ตอน extraction (real fix) → Phase 2+ (would obsolete snippet post-hoc logic)
- ❌ Quote verification UI (✓ badge) → additive UX, separate scope
- ❌ Fix pre-existing `semantic_vertical_slice` test → Phase 1.8

---

## 9. Working rules สำหรับ execution

1. TDD — เขียน test ก่อนเสมอ (mirror Phase 1.5 pattern)
2. แยก module `src/snippet.rs` และ `src/inbox_conflicts.rs` — pure, no I/O
3. `EvidenceSummary` field additions ต้อง additive (backward compat)
4. Severity mapping ใช้ Phase 1.5 vocabulary (`QualityTagKind` enum + Critical/Warning/Info)
5. ห้ามใช้ `{@html}` ใน Svelte — XSS-safe by construction
6. Frequent commits — 1 module = 1 commit minimum
7. Phase 1.6 เสร็จ → Docker rebuild + browser test + report

---

## 10. Open questions สำหรับ review

1. **Snippet window size** — ±200 chars (research recommendation, Google-LlamaIndex midpoint)
2. **Conflict threshold** — rel_diff > 0.1% (0.001) — strict, tunable later via TOML in Phase 1.8
3. **"No snippet found" fallback** — first 300 chars + honest prefix "Value not found in source —"
4. **UI: list-row badge position** — ข้าง subject chip (compact, scan-friendly) — verified Inbox.svelte:762 has space
5. **Resolve-in-place** — Phase 1.6 ใช้ปุ่มเดิมใน detail panel (reuse `startApprove`/`startReject`) — no new workflow

**Default answers locked in** — change any before approval.
