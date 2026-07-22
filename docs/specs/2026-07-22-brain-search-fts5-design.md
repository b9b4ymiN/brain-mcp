# brain_search FTS5 — Implementation Spec

> **Date:** 2026-07-22
> **Status:** APPROVED — ready for implementation plan
> **Branch:** `vnext/phase-0`
> **Depends on:** Entity Identity Reform (schema v4, shipped)

---

## Problem

`brain_search` cannot find semantic claims. Two root causes:

1. **Linear substring scan, not an index.** `brain_search` (`src/mcp/handlers.rs:856`) calls `store.all_claims_current(head, now)` then does an in-memory `.filter()` with case-insensitive `contains` on `subject` OR `predicate` only. No `value`, no `domain`, no ranking, no tokenization. At ~100 claims this is fine; at 10,000 (BLUEPRINT target) it is O(N) per query.

2. **Substring match is too literal.** Searching `"reinvent wheel"` misses the claim `"Reinvent the Wheel"` because the literal substring doesn't appear (the string has `"the "` in the middle).

## Research — What Production Systems Do

Every production knowledge graph separates structured storage from a full-text search index, with an automated sync pipeline:

| System | Structured Store | Search Index | Sync |
|--------|-----------------|-------------|------|
| Wikidata | Blazegraph (SPARQL) | Elasticsearch/CirrusSearch (BM25) | Streaming Updater |
| MusicBrainz | PostgreSQL | Solr/Lucene | Replication triggers |
| GitHub | MySQL | Elasticsearch | Kafka stream |

**SQLite FTS5** is the lightweight equivalent for embedded systems: it provides BM25 ranking, tokenization (porter stemming + unicode), prefix queries, and column weighting — all inside the SQLite database we already ship (rusqlite bundles SQLite ≥3.34 with FTS5 compiled in).

### FTS5 best practices (from SQLite docs + production patterns)

- **External-content table** (`content='claim_status', content_rowid='claim_id'`) — the FTS table stores only the inverted index, not a copy of the data. Keeps storage lean and avoids dual-write drift.
- **`porter unicode61` tokenizer** — Unicode-aware tokenization + Porter stemming. "running" → "run", "Reinvent" matches "reinvent".
- **`bm25()` with column weights** — `bm25(fts, 10.0, 5.0, 1.0, 2.0)` weights subject > predicate > domain > value for relevance ranking.
- **`MATCH` query syntax** — supports `AND`/`OR`, prefix (`rein*`), phrase (`"exact match"`).
- **Sync in the same transaction** as the write to `claim_status` — atomic, no drift.
- **`INSERT INTO fts(fts) VALUES('rebuild')`** for initial backfill from existing data.
- **`INSERT INTO fts(fts) VALUES('optimize')`** after bulk loads to merge index segments.

---

## Design

### Architecture

```
                                    ┌─────────────────────┐
  brain_search MCP tool             │  SQLite (schema v5)  │
        │                           │                       │
        ▼                           │  claim_status (rows)  │
  SemanticStore::                   │       │               │
  search_claims(query, top_k)       │       │ content='...'  │
        │                           │       ▼               │
        ▼                           │  claim_search_fts     │
  SELECT claim_id, subject,         │  (FTS5 virtual table) │
         predicate, value, domain,  │  - BM25 ranking       │
         bm25(...) AS score         │  - porter unicode61   │
  FROM claim_search_fts             │  - 4 indexed columns  │
  WHERE claim_search_fts MATCH ?    │                       │
    AND active (not superseded/     │  Sync: written in the  │
         retracted)                 │  SAME transaction as   │
  ORDER BY bm25(...) ASC            │  claim_status writes   │
  LIMIT ?                           └─────────────────────┘
```

### Data Flow

**Write path (brain_confirm / brain_supersede):**

```
build_confirmation_material()
  → INSERT INTO claim_status (...)
  → INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
        VALUES (claim_id, subject, predicate, flatten(value), domain)
  ↑ both inside the SAME SQLite transaction (atomic)
```

**Delete path (supersede / retract):**

```
UPDATE claim_status SET superseded_by_event_seq = ...
  → DELETE FROM claim_search_fts WHERE rowid = superseded_claim_id
  ↑ same transaction
```

**Read path (brain_search):**

```
brain_search(query="reinvent wheel", top_k=10)
  → SELECT cs.claim_id, cs.subject, cs.predicate, cs.value, cs.domain,
           bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) AS score
    FROM claim_search_fts fts
    JOIN claim_status cs ON cs.claim_id = fts.rowid
    WHERE claim_search_fts MATCH ?
      AND cs.superseded_by_event_seq IS NULL
      AND cs.retracted_at_event_seq IS NULL
    ORDER BY score ASC
    LIMIT ?
```

---

## Scope — What Changes

### 1. Schema migration (v4 → v5)

**File:** `src/semantic.rs`

- `CURRENT_DISK_SCHEMA_VERSION: 4 → 5`
- Add `(4, 5)` to `schema_upgrade_path_exists` and `schema_upgrade_reachable`
- `run_upgrade_step_forward` — add `(4, 5, 0)` arm:
  ```sql
  CREATE VIRTUAL TABLE claim_search_fts USING fts5(
      subject,
      predicate,
      value_flat,
      domain,
      content='claim_status',
      content_rowid='claim_id',
      tokenize = 'porter unicode61'
  );
  -- Backfill from existing claim_status rows.
  -- value_flat must be the JSON value flattened to a searchable string.
  INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
  SELECT claim_id, subject, flatten_json_value(value), domain
  FROM claim_status
  WHERE superseded_by_event_seq IS NULL AND retracted_at_event_seq IS NULL;
  INSERT INTO claim_search_fts(claim_search_fts) VALUES('optimize');
  ```
  **Note:** `flatten_json_value` is not a built-in SQL function — it must be done in Rust during the migration step (read rows, flatten, insert). Or store the flattened value on `claim_status` as a new column `value_flat TEXT` during the migration, then the FTS5 external-content table references it. The latter is simpler for sync (write `value_flat` alongside every `claim_status` INSERT).

  **Recommended approach:** add a `value_flat TEXT` column to `claim_status` (populated by Rust at confirm time via `flatten_json(&value)`), and the FTS5 table indexes `subject, predicate, value_flat, domain` from `claim_status` directly. This keeps the external-content sync purely SQL-triggered (no Rust in the read path).

- `run_upgrade_step_reverse` — add `(4, 5, 0)` arm:
  ```sql
  DROP TABLE claim_search_fts;
  -- value_flat column stays (harmless); or recreate claim_status without it
  -- (table recreation, same pattern as v3→v4 constraint change)
  ```
- Fresh-store DDL in `initialize_schema` — add the FTS5 table + `value_flat` column for new stores created at v5.

### 2. `value_flat` helper

**File:** `src/semantic.rs` (new function)

```rust
/// Flatten a serde_json::Value into a searchable string for FTS5 indexing.
/// - String → the string itself
/// - Number → its string representation
/// - Bool → "true"/"false"
/// - Array → space-joined elements (recursively flattened)
/// - Object → space-joined values (recursively flattened)
/// - Null → empty string
fn flatten_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(flatten_json)
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Object(obj) => obj
            .values()
            .map(flatten_json)
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Null => String::new(),
    }
}
```

### 3. Write path — populate FTS5 on confirm

**File:** `src/semantic.rs` — `build_confirmation_material` (around line 7175)

Add the FTS5 INSERT in the same transaction block that writes `claim_status`:

```rust
// After the existing INSERT INTO claim_status(...):
let value_flat = flatten_json(&value);
transaction.execute(
    "INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
     VALUES (?1, ?2, ?3, ?4, ?5)",
    params![claim_id.to_string(), &subject, &predicate, &value_flat, &domain_for_claim],
)?;
```

**File:** `src/semantic.rs` — supersede path

When `claim_status` is marked superseded (`UPDATE ... SET superseded_by_event_seq = ...`), delete the FTS5 row:

```rust
transaction.execute(
    "DELETE FROM claim_search_fts WHERE rowid = ?1",
    [superseded_id.to_string()],
)?;
```

**File:** `src/semantic.rs` — retract path (if separate from supersede)

Same pattern — delete the FTS5 row when the claim is retracted.

### 4. New `search_claims` method on `SemanticStore`

**File:** `src/semantic.rs` — new public method

```rust
/// Full-text search over confirmed, active claims using the FTS5 index.
/// Returns claims ranked by BM25 relevance to the query.
/// The query supports FTS5 MATCH syntax: tokens, prefixes (`rein*`),
/// phrases (`"exact match"`), AND/OR.
pub fn search_claims(
    &self,
    query: &str,
    domain: Option<&str>,
    top_k: usize,
) -> Result<Vec<ClaimSearchHit>> {
    // Build SQL: FTS5 MATCH + join claim_status + filter active + optional domain
    // Column weights: subject=10, predicate=5, value_flat=1, domain=2
    // ORDER BY bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) ASC
    // LIMIT top_k
    // Each hit: { claim_id, subject, predicate, value (from event payload),
    //            domain, score }
}
```

**New struct:**

```rust
/// A single search result from FTS5-ranked claim search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimSearchHit {
    pub claim_id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub domain: String,
    /// BM25 relevance score (lower = more relevant; FTS5 returns negatives).
    pub score: f64,
}
```

### 5. Rewrite `brain_search` handler

**File:** `src/mcp/handlers.rs` — `handle_brain_search` (line 856)

Replace the linear-scan + substring-filter with:

```rust
pub fn handle_brain_search(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult {
    let Some(store) = &server.semantic_store else {
        return Err("brain not initialized".to_owned());
    };
    let query = arg_str_req(args, "query")?;
    let domain = arg_str(args, "domain");
    let top_k = arg_str(args, "top_k")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);

    let hits = store
        .search_claims(&query, domain.as_deref(), top_k)
        .map_err(|e| format!("{e}"))?;

    let payload = serde_json::json!({
        "count": hits.len(),
        "query": query,
        "results": hits.iter().map(|h| serde_json::json!({
            "claim_id": h.claim_id,
            "subject": h.subject,
            "predicate": h.predicate,
            "value": h.value,
            "domain": h.domain,
            "score": h.score,
        })).collect::<Vec<_>>(),
    });
    ok_text(serde_json::to_string_pretty(&payload)?)
}
```

### 6. `claim_timeline` interaction

No change needed — `claim_timeline` already queries `claim_status` directly and is unaffected by the FTS5 index.

### 7. Tests

**File:** `tests/semantic_search_fts5_v1.rs` (new)

Test cases:

1. **Basic match** — confirm a claim, `search_claims("reinvent")` finds it.
2. **Tokenized match** — search `"reinvent wheel"` finds `"Reinvent the Wheel"` (the tokenizer splits both sides and matches all tokens).
3. **Value search** — confirm a claim with `value: json!(["Tesla", "BMW"])`, search `"tesla"` finds it via `value_flat`.
4. **Domain filter** — search with `domain: Some("engineering")` filters to that domain only.
5. **BM25 ranking** — confirm 3 claims with different overlap to the query; assert the most-relevant one has the lowest (best) score.
6. **Supersede removes from search** — confirm → supersede → search no longer finds the old claim.
7. **Migration test** — stage a v4 store with existing claims, upgrade to v5, assert FTS5 backfill finds all pre-existing claims.
8. **Empty/edge queries** — empty string, special characters, very long query.

### 8. MCP tool manifest

**File:** `src/mcp/tools.rs`

No change to the `brain_search` tool definition (same args: `query`, `domain`, `top_k`). The description could be updated to mention BM25 ranking:

```
"Search confirmed claims in the semantic brain (BM25-ranked full-text search across subject, predicate, value, and domain)"
```

---

## Migration Plan

### Phase A — Schema + migration (v4→v5)

1. Add `value_flat TEXT` column to `claim_status` (table recreation, same pattern as v3→v4).
2. Create `claim_search_fts` FTS5 virtual table.
3. Backfill: iterate existing `claim_status` rows, compute `value_flat` in Rust, populate FTS5.
4. Bump `CURRENT_DISK_SCHEMA_VERSION` to 5.
5. Test: migration on a copy of the live store (34 entities, 97 claims).

### Phase B — Write path

1. Add `flatten_json` helper.
2. In `build_confirmation_material`: compute `value_flat`, write to `claim_status.value_flat` + `claim_search_fts` in the same transaction.
3. In supersede/retract paths: delete FTS5 row in the same transaction.
4. Test: confirm a claim → search finds it immediately.

### Phase C — Read path

1. Add `search_claims` method.
2. Rewrite `handle_brain_search` handler.
3. Test: all 8 test cases pass.
4. Run full workspace test suite.

### Phase D — Production migration

1. Stop Docker container.
2. Backup v4 store.
3. `docker compose build && docker compose up -d`.
4. `recovery upgrade` (v4→v5).
5. Verify: `brain_search "reinvent wheel"` returns the claim.

---

## Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| FTS5 not compiled into bundled SQLite | Low | rusqlite `bundled` feature compiles SQLite with FTS5 enabled by default. Verify with `SELECT fts5(?)` during migration. |
| External-content table drift (FTS5 out of sync with claim_status) | Medium | All writes go through the same transaction. Add `INSERT INTO fts(fts) VALUES('integrity-check')` to a health-check endpoint. |
| `value_flat` column addition requires table recreation | Medium | Same table-recreation pattern as v3→v4 (proven in Entity Identity Reform). Atomic, reversible. |
| BM25 column weights need tuning | Low | Weights are a single constant array in `search_claims`; easy to adjust post-ship. Start with subject=10, predicate=5, value=1, domain=2. |
| Existing `brain_search` callers expect substring behavior | Low | FTS5 MATCH is a superset of substring (prefix queries + tokenization). The old behavior is preserved as a subset. |

---

## Definition of Done

### Functional
- [ ] `brain_search "reinvent wheel"` returns the "Reinvent the Wheel" claim (ranked by BM25)
- [ ] `brain_search "tesla"` finds claims whose `value` is a JSON array containing "Tesla"
- [ ] `brain_search` with `domain: "engineering"` filters correctly
- [ ] Superseded/retracted claims are excluded from search results
- [ ] BM25 ranking: most-relevant result appears first

### Architectural
- [ ] `claim_search_fts` FTS5 table exists with `porter unicode61` tokenizer
- [ ] `value_flat` column on `claim_status`
- [ ] `search_claims` method uses FTS5 MATCH, not linear scan
- [ ] FTS5 writes are in the same transaction as `claim_status` writes
- [ ] Schema version bumped to 5 with reversible migration

### Quality
- [ ] `cargo test --workspace` passes
- [ ] `tests/semantic_search_fts5_v1.rs` — 8 new tests pass
- [ ] Migration tested on a copy of the live store
- [ ] Docker rebuild + browser verify

---

## Key Files

| File | Change |
|------|--------|
| `src/semantic.rs` | Schema bump, FTS5 DDL, migration, `flatten_json`, `search_claims`, write-path sync |
| `src/mcp/handlers.rs` | Rewrite `handle_brain_search` to call `search_claims` |
| `src/mcp/tools.rs` | Update `brain_search` description (optional) |
| `tests/semantic_search_fts5_v1.rs` | New test file (8 tests) |
| `tests/semantic_migration_v1.rs` | v4→v5 migration test |
