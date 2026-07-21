# Spec — Entity Identity Reform (Wikidata Pattern)

> Created: 2026-07-21
> Status: **DRAFT — awaiting user review**
> Branch target: `vnext/phase-0`
> Research basis: W3C OWL2 spec + Wikidata (12+ years production) + OpenAI Temporal Agents Cookbook + Obsidian/Logseq personal memory patterns
> Authoritative principle: **"Don't reinvent the wheel"** — apply the production-proven identity pattern to the existing schema

---

## TL;DR

ปัญหา: 1 entity จริง (CATL) กลายเป็น 6 entity IDs เพราะ `entities` table ใช้ `UNIQUE(domain, canonical_subject)` เป็น identity constraint — domain แยกกัน (business/Business Strategy/Finance/financial/production/technology) สร้าง entity_id ใหม่แต่ละอัน

**Fix:** Apply Wikidata pattern — subject เป็น identity เดียว, domain เป็น categorization tag (multiple per entity). เปลี่ยน `UNIQUE(domain, canonical_subject)` → `UNIQUE(canonical_subject)` + auto-merge 7 CATL → 1 + ป้องกัน future fragmentation

**Phase:** 1.7 (push C4 predicate ontology ไป 1.8 เพราะ fix นี้สำคัญกว่า + เป็น prerequisite)
**เวลา:** 2-3 วัน

---

## 1. Problem (verified จาก schema จริง)

```sql
CREATE TABLE entities(
    entity_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,
    canonical_subject TEXT NOT NULL,
    UNIQUE(domain, canonical_subject)   -- ← FRAGMENTATION SOURCE
);
```

**Data จริง 2026-07-21:** 34 entities แต่มีแค่ 24 unique subjects → fragmentation ratio 1.42x
- CATL = 6 entities (Business/Business Strategy/Finance/business/financial/technology)
- BYD, Sodium-ion (Naxtra), Hungary overseas plant, Spain JV, LG Energy = 2 entities each

**Chain effect:**
- Home page galaxy view แสดง 7 CATL nodes แทน 1 → user สับสน
- Conflict detection (Phase 1.6) group by `(domain, subject, predicate)` → peer conflicts ไม่เจอเพราะ domain ต่างกัน
- Search by entity พบ 7 results แทน 1
- Future ingest → fragmentation ทวีคูณ

---

## 2. Root cause analysis

### 2.1 Schema design assumption (ผิด)

Schema ออกแบบตาม ADR-0001 §Decision 6: `scope key = (owner_id, domain, subject_id, predicate, normalized_context)`. แต่ ADR-0001 บอกชัดว่า `latest-user-wins` applies เฉพาะ preference/profile/project-decision claims — **ไม่ใช่ entity identity**.

ADR-0001 example `{"subject":"GULF"}` + `{"scope":"projects:brain:deployment:production"}` แสดงว่า design intent จริงๆคือ **subject = identity** (GULF, brain), domain/scope = categorization

### 2.2 LLM domain free-text (trigger)

LLM สร้าง domain variants 28 แบบ (business/Business/Business Strategy/Financial/Finance/financial/production/technology/...) ใน ingest เดียว. Phase 1.5 แก้ subject แต่ไม่ได้แก้ domain

### 2.3 Wikidata + W3C OWL2 + OpenAI converge ที่ pattern เดียวกัน

W3C OWL2 Syntax §9.6.3 Class Assertions:
> *"A class assertion `ClassAssertion(CE a)` states that individual `a` is an instance of class expression `CE`"*

An individual can be instance of multiple classes — class membership ≠ identity. CATL เป็น instance ของ business + financial + technology พร้อมกันได้ — เป็น individual ตัวเดียว

---

## 3. Design — Wikidata pattern applied

### 3.1 The wheel (Wikidata, 12+ years, 100M+ items)

| Component | Wikidata | เรา | Notes |
|-----------|----------|-----|-------|
| Identity | QID (Q42) | `entity_id` (UUIDv7) | มีอยู่แล้ว ✅ |
| Display name | labels per language | `canonical_subject` | มีอยู่แล้ว ✅ |
| Category/type | `instance of (P31)` | `domain` (tag) | เปลี่ยนจาก identity → tag |
| Alternative names | aliases | `entity_aliases` | มีอยู่แล้ว ✅ |
| Merge | MergeItems special page | `merge_entities` API | มีอยู่แล้ว ✅ |

**We already have 4/5 primitives. The only missing piece: use them correctly per the pattern.**

### 3.2 Schema migration

```sql
-- Migration: domain ออกจาก identity constraint
-- (run inside a SQLite transaction, atomic)

-- Step 1: Add domain_tags column to track which domains existed before merge
-- (optional — for backward-compat audit, not strictly needed)

-- Step 2: For each (canonical_subject) with multiple entity_ids,
-- pick the one with most claims as canonical target, merge the rest
-- This is done via merge_entities API (audited event per ADR-0001)

-- Step 3: Drop UNIQUE(domain, canonical_subject), add UNIQUE(canonical_subject)
CREATE UNIQUE INDEX idx_entities_canonical_subject_unique
    ON entities(canonical_subject);
DROP INDEX IF EXISTS idx_entities_domain_canonical_subject;
```

### 3.3 Code changes

**`src/semantic.rs`:**

| Function | Before | After |
|----------|--------|-------|
| `resolve_or_create_entity_in_tx(connection, domain, subject, event_seq)` | Uses `(domain, subject)` lookup | Uses `subject` only lookup |
| `resolve_entity_in_tx(connection, domain, alias)` | `(domain, alias)` lookup | `alias` only lookup |
| `insert_alias(connection, domain, alias, entity_id, ...)` | PK `(domain, alias, entity_id)` | PK `(alias, entity_id)` (drop domain) |
| `merge_entities` cross-domain guard (line 3269) | `if source_domain != target_domain { reject }` | **DELETE** — domain no longer identity |
| `claim_timeline(domain, subject, predicate)` | Filter by `(domain, subject, predicate)` | Filter by `(subject, predicate)` + domain as OR clause if provided |
| `all_claims_current(...)` | Iterates `(domain, subject, predicate)` buckets | Iterates `(subject, predicate)` buckets |

**`src/quality.rs`:** `ALLOWED_DOMAINS` canon stays as a **validator hint** (warn when domain not in canon) but no longer controls entity identity

**`src/extraction.rs`:** extraction prompt v4 — clarify that `domain` is a tag, not identity. No enforcement needed

### 3.4 Public API impact

| API | Change |
|-----|--------|
| `GET /api/v1/entity/merge` | Works across domains now (cross-domain guard removed) |
| `GET /api/v1/timeline?domain=&subject=&predicate=` | `domain` becomes optional filter (not required for identity) |
| `GET /api/v1/inbox` | No change (uses entity_id) |
| `GET /api/v1/galaxy` | Auto-benefits (renders 1 node per entity_id) |
| `GET /api/v1/search` | Auto-benefits |

---

## 4. Migration plan (3-phase)

### Phase A — Schema migration (atomic transaction)

```sql
BEGIN;
-- For each subject with multiple entity_ids, merge via SQL
-- (this is the legacy migration; new merges go through API)
WITH duplicates AS (
    SELECT canonical_subject, entity_id,
           ROW_NUMBER() OVER (PARTITION BY canonical_subject ORDER BY claim_count DESC) as rn
    FROM (
        SELECT e.canonical_subject, e.entity_id,
               (SELECT COUNT(*) FROM claim_status c WHERE c.entity_id = e.entity_id) as claim_count
        FROM entities e
    )
)
-- Pick rn=1 as target, rewrite all rn>1 claims to target's entity_id
UPDATE claim_status
SET entity_id = (SELECT entity_id FROM duplicates d2 WHERE d2.canonical_subject = duplicates.canonical_subject AND d2.rn = 1)
FROM duplicates
WHERE claim_status.entity_id = duplicates.entity_id AND duplicates.rn > 1;

-- Move aliases onto target
INSERT OR IGNORE INTO entity_aliases(alias, entity_id, kind, aliased_at_event_seq)
SELECT alias,
       (SELECT entity_id FROM duplicates d2 WHERE d2.canonical_subject = e.domain || ':' || e.canonical_subject AND d2.rn = 1),
       'former_subject',  -- mark as historical
       0
FROM entity_aliases ea JOIN entities e ON ea.entity_id = e.entity_id
WHERE ea.entity_id IN (SELECT entity_id FROM duplicates WHERE rn > 1);

-- Delete source entities
DELETE FROM entities WHERE entity_id IN (SELECT entity_id FROM duplicates WHERE rn > 1);

-- Drop old constraint, add new
DROP INDEX IF EXISTS sqlite_autoindex_entities_2;  -- the UNIQUE(domain, canonical_subject)
CREATE UNIQUE INDEX idx_entities_canonical_subject_unique ON entities(canonical_subject);
COMMIT;
```

**Result:** 34 entities → 24 entities (10 merges)

### Phase B — Code changes

1. `resolve_or_create_entity_in_tx` — drop domain parameter
2. `resolve_entity_in_tx` — drop domain parameter
3. `insert_alias` — drop domain from PK
4. `merge_entities` — remove cross-domain guard
5. `claim_timeline` — domain becomes optional filter
6. Update all callers (~10 sites in semantic.rs, api.rs)

### Phase C — Test + verify

1. **Migration test:** before → 34 entities, after → 24
2. **Identity test:** proposing 2 claims with subject="TEST" + different domains → 1 entity_id
3. **Merge test:** cross-domain merge succeeds
4. **Galaxy test:** home page shows 1 CATL node (not 7)
5. **Backward compat:** existing tests pass (some may need update for new API)

---

## 5. What this enables (3-5 year horizon)

| Feature | Before | After |
|---------|--------|-------|
| CATL on home page | 7 nodes | **1 node** |
| Conflict detection (Phase 1.6) | domain-blocked | **works across domains** |
| Search by entity | 7 results | **1 result** |
| LLM creates new domain ("ESG") | new entity fragment | **just a new tag on existing entity** |
| Wikidata-like merge | manual cross-domain blocked | **one-click** |
| 10,000+ pages scale | fragmentation explodes | **stable** |

---

## 6. Definition of Done (DoD)

### 6.1 Functional DoD
- [ ] CATL = 1 entity_id (verified via SQLite)
- [ ] BYD = 1 entity_id
- [ ] Home page galaxy shows 1 CATL node
- [ ] Proposing 2 claims with same subject different domains → 1 entity_id
- [ ] Cross-domain merge succeeds without error
- [ ] Existing 66 confirmed claims still queryable (no data loss)

### 6.2 Architectural DoD
- [ ] `entities` UNIQUE constraint = `canonical_subject` only
- [ ] `entity_aliases` PK excludes domain
- [ ] `resolve_or_create_entity_in_tx` signature drops domain
- [ ] `merge_entities` cross-domain guard removed
- [ ] `domain` field remains on `claim_status` (as tag)

### 6.3 Quality DoD
- [ ] Migration script is idempotent (re-run safe)
- [ ] Migration script tested on copy of production data
- [ ] All existing tests pass (some updated for new API signatures)
- [ ] New tests cover: identity resolution, cross-domain merge, migration
- [ ] clippy + fmt clean

### 6.4 Documentation DoD
- [ ] ADR-0002 created: "Entity identity reform — Wikidata pattern"
- [ ] BLUEPRINT updated
- [ ] Spec marked SHIPPED with verified numbers

---

## 7. Risk + Mitigation

| Risk | ระดับ | Mitigation |
|------|------|-----------|
| Migration script พังข้อมูล | สูง | Run on copy first + backup + atomic transaction |
| Existing tests break | ปานกลาง | Update API signatures, expect ~10-20 test updates |
| ADR-0001 scope key semantics เปลี่ยน | ปานกลาง | ADR-0002 documents the amendment explicitly |
| Search/timeline behavior change | ต่ำ | domain ยังใช้เป็น filter ได้ (optional) |
| Conflict detection (Phase 1.6) ทำงานผิด | ต่ำ | domain เป็น tag แล้ว → conflict detection ทำงานถูกขึ้น |

---

## 8. Out of scope

- ❌ Predicate ontology (Phase 1.8 — push back)
- ❌ Subject canonicalization (CATL vs Contemporary Amperex) — separate phase
- ❌ Thai NER — Phase 1.9
- ❌ Domain canon enforcement (warning only, not block)
- ❌ ADR-0001 rewrite — just ADR-0002 amendment

---

## 9. Working rules for execution

1. Backup SQLite before migration
2. Run migration on copy first, verify counts
3. TDD for code changes (mirror Phase 1.5 pattern)
4. Atomic transaction for migration (commit or rollback entirely)
5. ADR-0002 documents the change with rationale
6. BLUEPRINT update
7. After migration: Docker rebuild + browser test (home page CATL = 1)
