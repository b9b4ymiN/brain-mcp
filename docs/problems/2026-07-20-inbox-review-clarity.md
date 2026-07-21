# Problem — Inbox Review Clarity (root cause analysis)

> Created: 2026-07-20
> Status: **PROBLEM DOCUMENTED — root cause research next**
> Author: observed by user during real review session on `http://127.0.0.1:8080/#/inbox`
> Evidence: `inbox-current-state-evidence.png` (full-page screenshot of CATL `Current case price is ¥361` proposal detail)
> Related: this is the **actual problem**; the previous taxonomy/embedding plans (`docs/plans/feature-taxonomy-*.md`) were a wrong-direction detour.

## 1. User's words (verbatim)

> "อยากให้แนะนำมากว่าแบบไหนใช้งานได้จริง — ปัญหามันเกิดมาจากที่ฉันไปรีวิวแล้วเห็นว่า **ข้อมูลมันบอกอะไรได้ไม่ชัดเจน** จากหน้า index แล้วก็ **ไม่รู้ควร Approve ไหม**"

> Example cited by user: "Beta is 0.95 finance 19/07/2026, 16:08:05 = ไม่บอก Beta ตัวไหน"

## 2. The actual problem (not taxonomy_drift noise)

The previous 2-hour detour into taxonomy/embedding was wrong-direction. The real problem is **review clarity**: when the user opens a proposal in the Inbox, they cannot decide whether to Approve/Reject because the displayed information is ambiguous, sparse, or misleading.

## 3. Concrete observations from live review session

Inspected `http://127.0.0.1:8080/#/inbox` while logged in. Sampled proposal `Current case price is ¥361` (domain=finance, submitted 7/19/2026).

### 3.1 List view problems

The list of 182 pending proposals shows for each:
- `subject` (free text from LLM)
- `predicate` (free text from LLM)
- `value`
- `domain` (free text)
- timestamp

But many proposals have **non-entity subjects**:
- `Current case price` — should be `CATL` (the actual entity)
- `DCF price per share` — should be `CATL`
- `Shares outstanding` — should be `CATL`
- `Equity Value` — should be `CATL`
- `international-peers-deep-2026-07 has_peer_data` — subject looks like a filename, value is a slug

The `predicate` ends up as `is` (generic verb) or as a long slug. The LLM has effectively **swapped subject and predicate** — the real entity got buried inside the predicate instead of being the subject.

### 3.2 Detail view problems

> ✅ **Closed 2026-07-21 by Phase 1.6:** Evidence excerpt replaced with
> ±200-char value-anchored snippet (`<mark>` highlight on the matched value,
> honest "Value not found in source" fallback showing first 300 chars when
> the LLM-inferred value isn't present in the span). New pure module
> `src/snippet.rs` (19 unit tests). Live Docker verification on 50-proposal
> sample: avg excerpt 356 chars (down from 3000+), 58% `value_located=true`.

For `Current case price is ¥361`:

| Field | Value | Problem |
|-------|-------|---------|
| Subject | `Current case price` | ❌ Not an entity — should be `CATL` |
| Predicate | `is` | ❌ Empty/generic — real predicate was lost |
| Value | `¥361` | OK |
| Diff preview | `Before: —, After: ¥361` | ❌ Doesn't compare against other claims on CATL in same scope |
| Provenance | `inference` | OK |
| Source ID | `—` (none) | ❌ No source linked |
| Quote hash | `—` (none) | ❌ No verifiable quote |
| Evidence excerpt | 3000+ char DCF table dump | ❌ Wall of text — user can't find where ¥361 came from |
| "No prior confirmed claim in scope" | text | ❌ Misleading — there ARE other pending proposals on CATL in same scope (DCF price ¥447.6, Equity Value ¥2,000.8B) that should be compared |

### 3.3 The Approve decision is unsupported

> ✅ **Closed 2026-07-21 by Phase 1.6:** Misleading "N current confirmed
> claims in scope" text replaced with real same-predicate conflict detection
> across pending + confirmed claims. New pure module
> `src/inbox_conflicts.rs` (19 unit tests) implements C1 HardValue (numeric
> rel_diff > 0.1% → Warning) and C2 Duplicate (identical value → Info).
> `InboxProposal` API now carries a `conflicts` field per proposal; Console
> renders a badge on the list row + expandable peers panel. Live 181-proposal
> dataset currently yields 0 conflicts (correct — only one duplicate
> `(subject, predicate)` group exists and its values are non-numeric /
> non-equal). Cross-predicate tension (`¥361` vs `¥447.6`) is intentionally
> Phase 1.7 (C4 ontology) territory.

User has to mentally answer:
- Is `¥361` correct? (no comparison to other claims)
- Is this CATL? (not stated — derived from context)
- Does it conflict with `DCF price per share = ¥447.6`? (not flagged)
- Where did `¥361` come from? (buried in 3000-char evidence dump)
- Should I trust this `inference`? (no source, no quote hash)

None of these are answerable from the current UI without manual work the user shouldn't have to do.

## 4. Problems NOT covered here (for separate scoping)

- `taxonomy_drift` rule noise (77% FP) — separate, smaller issue than this
- High proposal volume (182 pending) — separate queue-management issue
- Mixed Thai/English text — separate i18n issue

These are real but secondary. The primary problem is **per-proposal review clarity**.

## 5. What would actually help (per user)

User explicitly said these would help them decide:
- ✅ **จับขัดแย้ง** (catch conflicts) — `¥361` vs `¥447.6` should be flagged as same-scope tension
- ✅ **เปรียบเทียบกับมีอยู่** (compare against existing) — show other claims on same entity in same scope
- (implied) **evidence excerpt that points at the relevant span** — not a 3000-char dump
- (implied) **structured subject/predicate** — entity name as subject, metric as predicate

## 6. Research needed

The next step is NOT to design a solution yet. The user asked for:
1. ✅ **Document this problem in Markdown** (this file)
2. ⬜ **Research root cause** — why does the current pipeline produce this output?
3. ⬜ **Research solution directions** — how do production systems solve review clarity?

## 7. Detour acknowledgement (lessons)

This problem file is being written AFTER a 2-hour detour where the agent:
- Built a `feature-taxonomy-drift-fix.md` plan (Phase A+B)
- Built a `feature-taxonomy-v2-embedding-canonical.md` plan (BGE-M3)
- Built a `feature-taxonomy-v2-spec.md` (5-phase embedding architecture)
- Discovered via research that BGE-M3 was already in BLUEPRINT.md (but BLUEPRINT is 2 months stale)
- Discovered via reading GOAL-vNext §2 that the embedding-as-canonical plan **contradicts the project's stated principle** that embeddings are rebuildable projections, not canonical truth

Root cause of the detour: **the agent took a single number (77% FP rate) as the problem statement** instead of opening the actual UI and watching the user's experience. The taxonomy_drift noise is real but minor compared to the review-clarity problem.

Lesson for next iteration: **open the actual UI and read the user's experience before designing a fix**.
