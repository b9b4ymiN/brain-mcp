# Entity Identity Reform — Design Document

> Created: 2026-07-21
> Status: **SHIPPED — schema v3→v4, code complete, tests green. Production migration pending (Task 22).**
> Branch: `vnext/phase-0`
> Principle: **"Don't reinvent the wheel"** — adopt the production-proven identity pattern (Wikidata, MusicBrainz, OSM, GitHub, OpenAI)
> Supersedes: `docs/plans/entity-identity-reform-wikidata-pattern.md` (absorbed + expanded)
> Related: ADR-0001 (scope key definition, to be amended by ADR-0002)

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [The Problem — Concrete Examples](#2-the-problem--concrete-examples-from-live-data)
3. [Root Cause Analysis](#3-root-cause-analysis)
4. [The Wheel — Production KG Identity Patterns](#4-the-wheel--production-kg-identity-patterns)
5. [Proposed Design — Wikidata Pattern Applied](#5-proposed-design--wikidata-pattern-applied)
6. [Code Inventory — Every Change Point](#6-code-inventory--every-change-point)
7. [Migration Plan — Atomic + Audited](#7-migration-plan--atomic--audited)
8. [What This Enables — 3-5 Year Horizon](#8-what-this-enables--3-5-year-horizon)
9. [Risk Assessment + Mitigation](#9-risk-assessment--mitigation)
10. [Definition of Done](#10-definition-of-done)
11. [Appendix A — Sources](#appendix-a--sources)

---

## 1. Executive Summary

brain-mcp-vnext มี entity fragmentation: 1 company จริง (CATL) กลายเป็น **6 entity_ids** เพราะ SQLite constraint `UNIQUE(domain, canonical_subject)` ทำให้ทุก domain variant ของ LLM (business/financial/Business Strategy/Finance/technology/...) สร้าง entity ใหม่.

**Fix:** Apply the **Wikidata identity pattern** — subject เป็น identity เดียว, domain เป็น categorization tag (multiple per entity). ทุก production KG system (Wikidata 100M+ items, MusicBrainz 20+ years, OpenStreetMap 17+ years, GitHub 200M+ repos, OpenAI Temporal Agents) converge ที่ pattern เดียวกันนี้ผ่านการเรียนรู้จาก production incidents เป็นสิบปี

**เวลา:** 2-3 วัน | **Migration:** 34 → 24 entities (10 auto-merges) | **Risk:** Medium (atomic transaction + backup mitigate)

---

## 2. The Problem — Concrete Examples from Live Data

### 2.1 CATL — 6 entities for 1 company (verified 2026-07-21)

```
entity_id                                domain               active_claims  total_claims
019f79a5-cd72-7693-a9de-0d09449fc2e8     business             22             23
019f79a5-fc16-7b32-9039-26093bd0262a     financial            19             20
019f79a6-1a87-7e01-bf4d-562efc004e31     Finance               6              6
019f84a8-4b8c-74a2-8a84-af98981ae5d3     Business              5              5
019f84a8-b975-7091-98d2-728b7623a9b7     Business Strategy     2              2
019f84a7-e631-7920-884a-4716174daae1     technology            1              1
─────────────────────────────────────────────────────────────────────────────────────
                                                              55 claims       spread across 6 entities
```

**ผลกระทบ:**
- Home page galaxy view: **6 nodes สำหรับ CATL** แทนที่จะเป็น 1
- User เห็น "CATL เชื่อมโยงกัน" แต่จริงๆ มันคือ 6 disconnected fragments
- 55 claims กระจาย → ไม่สามารถดู "ทุกอย่างที่รู้เกี่ยวกับ CATL" ในที่เดียว

### 2.2 Same fact, different fragments — conflict detection blind spot

Phase 1.6 conflict detection groups by `(domain, subject, predicate)`. ดู claims จริงบน CATL:

```
domain='business'          pred='ESS gross margin'           entity=019f79a5...
domain='business'          pred='ESS revenue share'          entity=019f79a5...
domain='Finance'           pred='ROIC-WACC spread'           entity=019f79a6...
domain='Finance'           pred='WACC'                       entity=019f79a6...
domain='Business Strategy' pred='is a cost leader'           entity=019f84a8...
domain='Business'          pred='customers'                  entity=019f84a8...
```

ถ้า LLM สร้าง "WACC" claim ใหม่ใน domain "financial" แต่มีค่าต่างจาก "Finance/WACC" เดิม → **Phase 1.6 conflict detection ไม่เจอเพราะ group key ต่างกัน** (financial ≠ Finance ≠ business ≠ Business)

### 2.3 Galaxy graph — 10 phantom nodes

```
Subject                    Nodes (should be 1)
'CATL'                              → 6
'Spain JV'                          → 2
'Sodium-ion (Naxtra)'               → 2
'LG Energy'                         → 2
'Hungary overseas plant'             → 2
'BYD'                               → 2
────────────────────────────────────────────
Total extra nodes on home page:     10
```

User เห็น 34 entities แทนที่จะเป็น 24 จริง → สับสน + เสียเวลาหา

### 2.4 Scale projection (3-5 year)

LLM pattern จริง: 5-6 domain variants ต่อ entity (observed). ถ้าไม่แก้:

| Scale | Entities จริง | Entities ในระบบ (fragmented) | Phantom |
|-------|-------------|------------------------------|---------|
| Today | 24 | 34 | 10 (42% inflation) |
| 100 entities | 100 | ~500 | ~400 |
| 1,000 entities | 1,000 | ~5,000 | ~4,000 |
| **10,000 entities (BLUEPRINT target)** | **10,000** | **~50,000** | **~40,000** |

**Fragmentation compounds with scale** — 3-5 year horizon = unsustainable

---

## 3. Root Cause Analysis

### 3.1 Schema constraint (the proximate cause)

```sql
-- src/semantic.rs:6436-6442
CREATE TABLE entities(
    entity_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,
    canonical_subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(domain, canonical_subject)   -- ← FRAGMENTATION SOURCE
);
```

**Every distinct `(domain, canonical_subject)` pair creates a new entity_id.** ถ้า domain เป็น free-text และ LLM สร้าง 6 variants → 6 entities.

### 3.2 Design assumption (the architectural cause)

ADR-0001 §Decision 6 defines scope key:
```
(owner_id, domain, subject_id, predicate, normalized_context)
```

**แต่ ADR-0001 บอกชัดว่า** `latest-user-wins` applies เฉพาะ `preference`, `profile`, `project decision` claims — **ไม่ใช่ entity identity**. และ ADR-0001's own examples:

```json
{"subject":"GULF","predicate":"target_price"}   ← subject เดียว, ไม่มี domain ใน identity
{"scope":"projects:brain:deployment:production"} ← scope เป็น hierarchical path, ไม่ใช่ (domain, subject) tuple
```

**Design intent จริง: subject = identity, domain/scope = categorization**

### 3.3 LLM free-text domain (the trigger)

v2 extraction prompt ปล่อยให้ LLM ใช้ domain free-text → 28 domain variants ใน ingest เดียว (verified จาก `docs/baseline/catl-e2e-20260719.md`). v3 prompt (Phase 1.5) เพิ่ม canon warning แต่ยังไม่ enforce + ไม่ได้แก้ constraint

---

## 4. The Wheel — Production KG Identity Patterns

### 4.1 The convergent pattern (5 production systems, 12-20 years each)

Across five independently-designed systems, the pattern is **universal and exceptionless**:

> **Identity = opaque, immutable, system-assigned identifier (QID/MBID/node_id/UUID). Every human-readable or category field (label, name, type, domain, tags, aliases) is mutable metadata layered on top.**

| System | Identity | Categorization | Years in production |
|--------|----------|----------------|---------------------|
| **Wikidata** | QID (Q42) | `instance of (P31)` statements | 12+ (100M+ items) |
| **MusicBrainz** | MBID (UUID) | artist type, genre, country | 20+ |
| **OpenStreetMap** | element ID (node/way/relation) | tags (key=value pairs) | 17+ (billions of elements) |
| **GitHub** | node ID (base64) | topics, language, license | 15+ (200M+ repos) |
| **OpenAI Temporal Agents** | UUID + `resolved_id` | `type` field, tags | 2025+ (current SOTA) |

**No production system has ever used a categorization field as part of the identity constraint.** The brain-mcp-vnext bug is a textbook violation of this separation.

### 4.2 Wikidata case study (the canonical reference)

**Identity:** QID = opaque integer (Q42 = Douglas Adams). *"Item labels do not need to be unique"* — multiple items can share a label; uniqueness is the QID alone.

**Merge process** ([Help:Merge](https://www.wikidata.org/wiki/Help:Merge)):
1. Source item's statements, sitelinks, labels, aliases → pooled into target
2. Source QID becomes a **permanent redirect** to target (never reused)
3. Merge recorded in page history (audit trail)

> *"Wikidata item IDs are designated as persistent identifiers. Therefore, merged items should be redirected. Never reuse merged items for other things."*

**Why this matters for us:** Wikidata has 100M+ items, 12+ years of operation, handles millions of merges — the pattern is battle-tested at 1000x our scale

### 4.3 MusicBrainz case study (20+ years, music KG)

**Identity:** MBID = 36-char UUID. *"An MBID is permanently assigned to each entity. When merged, its MBIDs redirect to the other entity."*

- "The Beatles" / "Beatles" / "披头士" = same MBID (via aliases)
- Aliases are first-class data model (many-to-many with type metadata)
- MBIDs from 2003 still resolve today (20-year durability)

### 4.4 OpenStreetMap case study (17+ years, geo KG)

**Identity:** element ID (node/way/relation, separate ID spaces). Tags (`amenity=cafe`, `cuisine=thai`) are categorization, never identity.

- Same physical café can carry multiple tags simultaneously
- Changing tags doesn't change identity
- QA tools (Osmose, JOSM Validator) detect duplicate nodes and offer automated merges

### 4.5 OpenAI Temporal Agents cookbook (2025, current SOTA)

```python
class Entity(BaseModel):
    """'id' is the canonical entity id if this is a canonical entity.
    'resolved_id' is set to the canonical id if this is an alias."""
    id: uuid.UUID = Field(default_factory=uuid.uuid4)
    event_id: uuid.UUID | None = None
    resolved_id: uuid.UUID | None = None
```

**Explicit pattern:** identity = fresh UUID, canonicality computed by `Entity Resolution` agent stage, `name`/`type`/`description` are fields on entity (never identity key).

### 4.6 Real-world fragmentation failures

**CDDB/Gracenote (the founding motivation for MusicBrainz):** Same album submitted under different spellings → separate database entries → broke deduplication across music libraries. MusicBrainz solved it with MBID-as-sole-identity + aliases.

**Neo4j agent-memory docs:** *"Without entity resolution, the knowledge graph becomes fragmented — the same entity exists as multiple disconnected nodes, losing multi-hop reasoning capability."*

**Vectorize Hindsight (2026):** *"Do entity resolution at write time. Resolving at query time is too late — fragmentation accumulates and is expensive to repair downstream."*

---

## 5. Proposed Design — Wikidata Pattern Applied

### 5.1 The pattern mapped to our primitives

We already have **4 out of 5** Wikidata primitives. Only one missing piece: use them correctly.

| Wikidata primitive | We have it? | Our equivalent |
|--------------------|-------------|----------------|
| Identity (QID) | ✅ | `entity_id` (UUIDv7) |
| Display name (labels) | ✅ | `canonical_subject` |
| Aliases | ✅ | `entity_aliases` table |
| Merge (MergeItems) | ✅ | `merge_entities` API |
| **"Categorization ≠ identity"** | ❌ → ✅ | **Drop `domain` from UNIQUE constraint** |

### 5.2 Schema migration

```sql
-- BEFORE (fragmentation source):
CREATE TABLE entities(
    entity_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,
    canonical_subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(domain, canonical_subject)   -- ← REMOVE domain from here
);

CREATE TABLE entity_aliases(
    domain TEXT NOT NULL,
    alias TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    aliased_at_event_seq INTEGER NOT NULL,
    PRIMARY KEY(domain, alias, entity_id)   -- ← REMOVE domain from PK
);

-- AFTER (Wikidata pattern):
CREATE TABLE entities(
    entity_id TEXT PRIMARY KEY,
    canonical_subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- domain ย้ายไปเป็น tag (optional, multiple per entity)
    UNIQUE(canonical_subject)
);

CREATE TABLE entity_aliases(
    alias TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    aliased_at_event_seq INTEGER NOT NULL,
    PRIMARY KEY(alias, entity_id)
);
```

**`claim_status.domain`** ยังอยู่ (เป็น tag บน claim แต่ละอัน — multiple domains per entity ผ่านหลาย claims)

### 5.3 What changes, what stays

| Component | Stays the same | Changes |
|-----------|----------------|---------|
| `entity_id` (UUIDv7) | ✅ Identity anchor | — |
| `canonical_subject` | ✅ Display name | Becomes UNIQUE (was part of composite) |
| `entity_aliases` | ✅ Alternative names | PK drops `domain` |
| `merge_entities` API | ✅ Exists | Cross-domain guard **removed** |
| `claim_status.domain` | ✅ Stays as tag | No longer part of entity identity |
| `entities.domain` | — | **Removed** (or becomes nullable tag) |

---

## 6. Code Inventory — Every Change Point

Full inventory from code trace (verified via grep + codegraph):

### 6.1 SQLite schema (3 tables)

| Table | Current constraint | Proposed | Location |
|-------|-------------------|----------|----------|
| `entities` | `UNIQUE(domain, canonical_subject)` | `UNIQUE(canonical_subject)` | `src/semantic.rs:6441` |
| `entity_aliases` | `PRIMARY KEY(domain, alias, entity_id)` | `PRIMARY KEY(alias, entity_id)` | `src/semantic.rs:6453` |
| `claim_status` | No UNIQUE on domain+subject | Unchanged (domain stays as tag) | `src/semantic.rs:6419` |

### 6.2 Core functions (3 signature changes)

| Function | Current signature | Proposed | Location |
|----------|-------------------|----------|----------|
| `resolve_or_create_entity_in_tx` | `(conn, domain, subject, event_seq)` | `(conn, subject, event_seq)` | `src/semantic.rs:7285` |
| `resolve_entity_in_tx` | `(conn, domain, alias)` | `(conn, alias)` | `src/semantic.rs:7315` |
| `insert_alias` | `(conn, domain, alias, entity_id, kind, event_seq)` | `(conn, alias, entity_id, kind, event_seq)` | `src/semantic.rs:7334` |

### 6.3 Guards to remove (2)

| Guard | Location | Action |
|-------|----------|--------|
| `merge_entities` cross-domain check | `src/semantic.rs:3269` | **Delete** (domain no longer identity) |
| `split_entities` cross-domain check | `src/semantic.rs:5751` | **Delete** |
| `rename_entity` domain-scoped collision | `src/semantic.rs:3182` | Simplify to subject-only |

### 6.4 Public read APIs (1 composite-key change)

| API | Current | Proposed |
|-----|---------|----------|
| `claim_timeline(domain, subject, predicate)` | Composite triple | `claim_timeline(subject, predicate)` — domain becomes optional filter |
| `all_claims_current()` | Unchanged | Unchanged |
| `list_pending_proposals()` | Unchanged | Unchanged |
| `entity_canonical_subjects()` | Unchanged | Unchanged |

### 6.5 HTTP endpoints (1 breaking change)

| Endpoint | Current | Proposed |
|----------|---------|----------|
| `GET /entity/timeline?domain=&subject=&predicate=` | domain required | `GET /entity/timeline?subject=&predicate=` (domain optional) |
| All others | domain optional or absent | Unchanged |

### 6.6 MCP tool handlers (2 manifest changes)

| Tool | Current | Proposed |
|------|---------|----------|
| `brain_capture` | `domain` required | `domain` optional |
| `brain_propose` | `domain` required | `domain` optional |
| `brain_search` / `brain_get` | `domain` optional | Unchanged |

### 6.7 Type changes (2 structs)

| Struct | Field | Change |
|--------|-------|--------|
| `ClaimDraft` | `domain: String` | → `domain: Option<String>` (or remove) |
| `EntityRecord` | `domain: String` | → derived/optional |
| `ProposalSummary` | `domain: String` | → derived/optional |

### 6.8 Test impact estimate

- ~40-50 test source files touched
- ~60 `ClaimDraft { domain: "..." }` construction sites to update
- ~40-60 assertions comparing `claim.domain` / `entity.domain`
- Most changes mechanical (strip `domain:` field)

---

## 7. Migration Plan — Atomic + Audited

### 7.1 Phase A — Data migration (atomic SQL transaction)

```sql
BEGIN;

-- Step 1: Pick canonical target for each fragmented subject (most claims wins)
CREATE TEMP TABLE merge_targets AS
WITH ranked AS (
    SELECT e.canonical_subject, e.entity_id,
           (SELECT COUNT(*) FROM claim_status cs WHERE cs.entity_id = e.entity_id
            AND cs.superseded_by_event_seq IS NULL) as active_claims,
           ROW_NUMBER() OVER (
               PARTITION BY e.canonical_subject
               ORDER BY (SELECT COUNT(*) FROM claim_status cs WHERE cs.entity_id = e.entity_id) DESC
           ) as rn
    FROM entities e
)
SELECT canonical_subject, entity_id as target_id FROM ranked WHERE rn = 1;

-- Step 2: Rewrite all claims from duplicate entity_ids to target
UPDATE claim_status
SET entity_id = (SELECT target_id FROM merge_targets mt
                 JOIN entities e ON mt.canonical_subject = e.canonical_subject
                 WHERE e.entity_id = claim_status.entity_id)
WHERE entity_id NOT IN (SELECT target_id FROM merge_targets)
  AND entity_id IN (SELECT entity_id FROM entities
                    WHERE canonical_subject IN (SELECT canonical_subject FROM merge_targets));

-- Step 3: Fold aliases onto targets (mark source domain as 'former')
INSERT OR IGNORE INTO entity_aliases(alias, entity_id, kind, aliased_at_event_seq)
SELECT alias, mt.target_id, 'former_subject', 0
FROM entity_aliases ea
JOIN entities e ON ea.entity_id = e.entity_id
JOIN merge_targets mt ON e.canonical_subject = mt.canonical_subject
WHERE ea.entity_id != mt.target_id;

-- Step 4: Delete duplicate entities
DELETE FROM entities
WHERE entity_id NOT IN (SELECT target_id FROM merge_targets)
  AND canonical_subject IN (SELECT canonical_subject FROM merge_targets);

-- Step 5: Drop old constraint, add new
-- (SQLite requires table recreation for constraint changes)
CREATE TABLE entities_new(
    entity_id TEXT PRIMARY KEY,
    canonical_subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(canonical_subject)
);
INSERT INTO entities_new SELECT entity_id, canonical_subject, created_at FROM entities;
DROP TABLE entities;
ALTER TABLE entities_new RENAME TO entities;

COMMIT;
```

**Result:** 34 entities → 24 entities (10 merges), atomic, auditable.

### 7.2 Phase B — Code changes

1. Schema constants in `src/semantic.rs:6436-6454` — update CREATE TABLE
2. 3 core functions — drop domain parameter
3. 2 guards — delete
4. 1 public API signature — drop domain
5. 1 HTTP endpoint — drop required domain param
6. 2 MCP tools — demote domain to optional
7. 2 struct types — make domain optional
8. Update all call sites (compiler-guided)

### 7.3 Phase C — Test + verify

1. **Migration test:** backup → migrate → assert 34→24 entities, 0 data loss
2. **Identity test:** propose 2 claims with `subject="TEST"` + different domains → 1 entity_id
3. **Merge test:** cross-domain merge succeeds
4. **Galaxy test:** home page shows 1 CATL node (browser verify)
5. **Regression:** all existing tests pass (with updated signatures)

---

## 8. What This Enables — 3-5 Year Horizon

| Capability | Before | After |
|------------|--------|-------|
| Home page CATL | 6 nodes | **1 node** |
| Conflict detection (Phase 1.6) | domain-blocked | **works across domains** |
| Search by entity | 6+ results | **1 result** |
| LLM creates new domain ("ESG") | new fragment | **just a tag** |
| Wikidata-like merge | blocked (cross-domain) | **one-click** |
| 10,000+ entities scale | 50,000 phantom | **stable at 10,000** |
| Cross-domain reasoning | impossible | **native** |
| Entity timeline | domain-scoped | **full entity history** |

**MusicBrainz MBIDs from 2003 still resolve today (20-year proof). The pattern is not theoretical — it's the empirically-validated default.**

---

## 9. Risk Assessment + Mitigation

| Risk | Level | Mitigation |
|------|-------|------------|
| Migration corrupts data | **High** | Backup SQLite first + test on copy + atomic transaction |
| ~50 test files break | Medium | Mechanical updates (strip `domain:` from literals) |
| ADR-0001 semantics change | Medium | ADR-0002 documents the amendment with rationale |
| MCP clients break (domain required → optional) | Low | Backward compatible (optional = old callers still work) |
| Phase 1.6 conflict detection behaves differently | Low | Works **better** (no domain-blocked groups) |
| Historical queries change semantics | Low | `domain` still queryable as filter |

---

## 10. Definition of Done

### 10.1 Functional DoD
- [ ] CATL = 1 entity_id (verified via SQLite)
- [ ] BYD = 1, LG Energy = 1, Sodium-ion = 1, Hungary = 1, Spain JV = 1
- [ ] Home page galaxy shows 1 CATL node (browser-verified)
- [ ] Proposing 2 claims same subject + different domains → 1 entity_id
- [ ] Cross-domain merge succeeds (API + UI)
- [ ] 66 confirmed claims still queryable (0 data loss)

### 10.2 Architectural DoD
- [ ] `entities` UNIQUE = `canonical_subject` only
- [ ] `entity_aliases` PK excludes domain
- [ ] `resolve_or_create_entity_in_tx` drops domain param
- [ ] `merge_entities` cross-domain guard removed
- [ ] `claim_timeline` signature updated
- [ ] `ClaimDraft.domain` optional
- [ ] ADR-0002 created (documents the reform)

### 10.3 Quality DoD
- [ ] Migration script idempotent + tested on copy
- [ ] `cargo test --workspace` passes (excluding pre-existing `semantic_vertical_slice`)
- [ ] New tests: identity resolution, cross-domain merge, migration
- [ ] clippy + fmt clean
- [ ] Docker rebuild + browser test

### 10.4 Documentation DoD
- [ ] ADR-0002: "Entity Identity Reform — Wikidata Pattern"
- [ ] BLUEPRINT.md updated
- [ ] This design doc marked SHIPPED with verified numbers
- [ ] Report file at `docs/reports/YYYY-MM-DD-entity-identity-reform-shipped.md`

---

## Appendix A — Sources

### Production case studies (researched via SearXNG + direct fetch)
- [Wikidata:Identifiers](https://www.wikidata.org/wiki/Wikidata:Identifiers)
- [Help:Items — Wikidata](https://www.wikidata.org/wiki/Help:Items)
- [Help:Merge — Wikidata](https://www.wikidata.org/wiki/Help:Merge)
- [Wikidata:True duplicates](https://www.wikidata.org/wiki/Wikidata:True_duplicates)
- [MusicBrainz Identifier](https://musicbrainz.org/doc/MusicBrainz_Identifier)
- [Style/Aliases — MusicBrainz](https://musicbrainz.org/doc/Style/Aliases)
- [Tags — OpenStreetMap Wiki](https://wiki.openstreetmap.org/wiki/Tags)
- [Elements — OpenStreetMap Wiki](https://wiki.openstreetmap.org/wiki/Elements)
- [Renaming a repository — GitHub Docs](https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository)
- [Temporal Agents with Knowledge Graphs — OpenAI Cookbook](https://developers.openai.com/cookbook/examples/partners/temporal_agents_with_knowledge_graphs/temporal_agents)
- [Entity Resolution and Deduplication — Neo4j Agent Memory](https://neo4j.com/labs/agent-memory/explanation/resolution-deduplication/)
- [Knowledge graph construction with Claude — Claude Cookbook](https://platform.claude.com/cookbook/capabilities-knowledge-graph-guide)

### Standards
- [OWL2 Web Ontology Language Structural Specification — W3C](https://www.w3.org/TR/owl2-syntax/) §9.6.3 Class Assertions

### Project context
- `docs/adr/0001-semantic-authority-time-privacy.md` (scope key definition)
- `docs/baseline/catl-e2e-20260719.md:143` (28 domain variants observed)
- `src/semantic.rs:6436-6454` (current schema)
- `src/semantic.rs:7285-7360` (entity resolution functions)
- `src/semantic.rs:3227-3330` (merge_entities + cross-domain guard)
