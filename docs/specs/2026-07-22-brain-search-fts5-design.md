# brain_search FTS5 — Implementation Spec (v2)

> **Date:** 2026-07-22
> **Status:** DRAFT v2 — pending user review before implementation plan
> **Supersedes:** v1 (this same file, 2026-07-22) — two design points revised after research:
>   1. **Tokenizer:** `porter unicode61` → **`trigram`** (Thai is a first-class `SubjectShape`; porter collapses Thai into one giant token).
>   2. **Sync:** hand-written INSERTs in the write path → **SQL triggers** (research: triggers cannot be forgotten across code paths/migrations and stay atomic).
> **Branch:** `vnext/phase-0`
> **Depends on:** Entity Identity Reform (schema v4, shipped)

---

## Problem

`brain_search` cannot find semantic claims. Two root causes:

1. **Linear substring scan, not an index.** `brain_search` (`src/mcp/handlers.rs:856`) calls `store.all_claims_current(head, now)` then does an in-memory `.filter()` with case-insensitive `contains` on `subject` OR `predicate` only. No `value`, no `domain`, no ranking, no tokenization. At ~100 claims this is fine; at 10,000 (BLUEPRINT target) it is O(N) per query.

2. **Substring match is too literal.** Searching `"reinvent wheel"` misses the claim `"Reinvent the Wheel"` because the literal substring doesn't appear (the string has `"the "` in the middle).

## Research — "Don't Reinvent the Wheel" (2026-07-22)

The brief was to confirm the wheel already exists before building one. Findings from three parallel investigations:

### What this project already ships

| Existing wheel | Designed for | Fit for brain claims? |
|----------------|--------------|----------------------|
| **tantivy 0.26** (`src/index_manager.rs`) | File-based markdown wiki search. Rebuilds the *whole* index when git HEAD changes (`state.commit = git hash`). MMAP segment files, single `IndexWriter` behind `parking_lot::RwLock`. | **No — category mismatch.** brain claims are rows written one-at-a-time inside a SQLite transaction, not files in a git tree. Dual-write (SQLite commit + separate tantivy `commit()`) drifts on crash; single-writer lock contends with concurrent confirm/retract; segment/MMAP/50 MB heap overhead doesn't amortize below ~10k small docs. Real-world projects that combine the two (bichon, ParadeDB) make tantivy *the* store, never a mirror of SQL rows. |
| **SQLite FTS5** (compiled into rusqlite `bundled`) | Full-text index inside the same SQLite file. | **Yes — exact fit.** Lives in the same transaction, no dual-write, BM25 + tokenization + column weighting for free. |

### What production knowledge-graphs do

Wikidata (Blazegraph + Elasticsearch/CirrusSearch) and MusicBrainz (PostgreSQL + Solr) both split structured store from search index. They sync via an outbox/CDC pipeline *only because* the search engine is a separate process. **Here, FTS5 is inside the same SQLite file, so one transaction is enough — an outbox would be overkill** (Kleppmann, *DDIA* Ch. 11). This is the genuine "wheel already invented" for the embedded/transactional case: SQLite's own FTS5 external-content table with SQL triggers.

### FTS5 best practices (from SQLite docs + production patterns)

- **External-content table** (`content='claim_status', content_rowid='claim_id'`) — the FTS table stores only the inverted index, not a copy of the data. Keeps storage lean and avoids dual-write drift.
- **SQL triggers** (`AFTER INSERT` / `AFTER DELETE` / `AFTER UPDATE`) — the documented production pattern. Triggers cannot be forgotten across code paths or migrations, and they fire atomically inside the source write transaction. v1 of this spec proposed hand-written INSERTs in `build_confirmation_material`; research shows triggers are strictly safer, so v2 adopts them.
- **`trigram` tokenizer** — indexes every 3-character window, giving substring match across **all scripts including Thai/CJK**. `porter unicode61` (v1's choice) is English-only and collapses unspaced Thai into a single oversized token, which would make `SubjectShape::ThaiPure` subjects (e.g. "บมจ. ปตท.") effectively unsearchable. The cost is a larger index and the loss of English stemming (`running` no longer matches `run`), accepted at this scale (10k rows × <200 bytes).
- **`bm25()` with column weights** — `bm25(fts, 10.0, 5.0, 1.0, 2.0)` weights subject > predicate > value > domain for relevance ranking.
- **`MATCH` query syntax** — supports `AND`/`OR`, prefix (`rein*`), phrase (`"exact match"`). Note: `trigram` supports phrase and substring queries; prefix queries still work.
- **`INSERT INTO fts(fts) VALUES('rebuild')`** for initial backfill; `VALUES('integrity-check')` for drift detection.

### Source URLs (selected)

- SQLite FTS5 — external content: https://www.sqlite.org/fts5.html#external_content_tables
- SQLite FTS5 — external content pitfalls (no-trigger drift): https://www.sqlite.org/fts5.html#external_content_table_pitfalls
- SQLite FTS5 — special insert commands (`rebuild`/`integrity-check`/`optimize`): https://www.sqlite.org/fts5.html#special_insert_commands
- SQLite FTS5 — trigram tokenizer: https://www.sqlite.org/fts5.html#the_trigram_tokenizer
- SQLite FTS5 — unicode61 tokenizer (Thai limitation): https://www.sqlite.org/fts5.html#unicode61_tokenizer
- Wikidata Streaming Updater: https://wikitech.wikimedia.org/wiki/Wikidata_Query_Service/Streaming_Updater
- MusicBrainz Search Architecture (trigger + RabbitMQ + SIR worker): https://musicbrainz.org/doc/Development/Search_Architecture
- Transactional outbox pattern (Debezium): https://debezium.io/blog/2019/02/19/reliable-microservices-data-exchange-with-the-outbox-pattern/
- "Don't reinvent the wheel" (Atwood): https://blog.codinghorror.com/dont-reinvent-the-wheel-unless-you-plan-on-learning-more-about-wheels/

---

## Design

### Architecture

```
                                    ┌─────────────────────────────┐
  brain_search MCP tool             │  SQLite (schema v5)          │
        │                           │                               │
        ▼                           │  claim_status (rows)          │
  SemanticStore::                   │    + value_flat TEXT column   │
  search_claims(query, top_k)       │       │                       │
        │                           │       │ content='claim_status'│
        ▼                           │       ▼                       │
  SELECT claim_id, subject,         │  claim_search_fts             │
         predicate, value, domain,  │  (FTS5 virtual table)         │
         bm25(...) AS score         │  - BM25 ranking               │
  FROM claim_search_fts             │  - trigram tokenizer          │
  WHERE claim_search_fts MATCH ?    │  - 4 indexed columns          │
    AND active (not superseded/     │                               │
         retracted)                 │  Sync: SQL triggers on        │
  ORDER BY bm25(...) ASC            │  claim_status (INSERT/DELETE/ │
  LIMIT ?                           │  UPDATE), in the same tx      │
                                    └─────────────────────────────┘
```

### Why triggers instead of hand-written INSERTs (v2 change)

v1 proposed adding `INSERT INTO claim_search_fts ...` calls to `build_confirmation_material`, the supersede path, and the retract path. Research found two problems:

1. **Three call sites to remember.** The retract path (`src/semantic.rs:2964`) does an `UPDATE claim_status SET retracted_at_event_seq=...`, and the supersede path does a similar `UPDATE ... SET superseded_by_event_seq=...`. Forgetting any site silently drifts the index. A future migration or refactor could add a fourth.
2. **SQLite's documented pitfall.** The FTS5 docs explicitly warn that hand-maintained external-content tables desync when the source row is UPDATEd or DELETEd without a matching FTS write, and that triggers are the recommended cure because they fire for *every* DML path including bulk tooling.

With triggers, the write path in `build_confirmation_material` and the supersede/retract paths stay **unchanged** — they keep issuing their existing `INSERT`/`UPDATE` on `claim_status`, and the triggers maintain the FTS index atomically inside the same transaction.

### Active-claim filtering

FTS5 cannot filter "active" (not superseded/retracted) inside the index itself — that status is derived from sequence columns. The read query therefore JOINs back to `claim_status` and filters in SQL:

```sql
SELECT cs.claim_id, cs.subject, cs.predicate, cs.value, cs.domain,
       bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) AS score
FROM claim_search_fts fts
JOIN claim_status cs ON cs.claim_id = fts.rowid
WHERE claim_search_fts MATCH ?
  AND cs.superseded_by_event_seq IS NULL
  AND cs.retracted_at_event_seq IS NULL
ORDER BY score ASC
LIMIT ?
```

This keeps as-of queries correct and means the FTS row for a superseded claim is *retained* (still indexable for historical/timeline views), just excluded from the default search. The trigram tokenizer means the `MATCH ?` argument is treated as a phrase/substring query by default.

### Data Flow

**Write path (brain_confirm):** `build_confirmation_material` issues its existing `INSERT INTO claim_status(...)`. The `AFTER INSERT` trigger populates the FTS row from the just-inserted columns. Both run in the same transaction.

**Supersede path:** `UPDATE claim_status SET superseded_by_event_seq=...`. The `AFTER UPDATE` trigger fires (because content columns were potentially touched — even if not, it's a no-op on FTS) and the row stays in the index, filtered out at read time by the `superseded_by_event_seq IS NULL` clause.

**Retract path:** `UPDATE claim_status SET retracted_at_event_seq=...`. Same as supersede — row stays indexed, filtered at read time.

**Read path (brain_search):** the `search_claims` method runs the FTS5 MATCH + JOIN + active-filter query above.

---

## Scope — What Changes

### 1. Schema migration (v4 → v5)

**File:** `src/semantic.rs`

- `CURRENT_DISK_SCHEMA_VERSION: 4 → 5`
- Add `(4, 5)` to `schema_upgrade_path_exists` and `schema_upgrade_reachable`.
- `plan_schema_upgrade` — add a `(4, 5)` arm describing the FTS5 + `value_flat` + triggers migration (reversible).
- `run_upgrade_step_forward` — add a `(4, 5, 0)` arm that calls a new `run_fts5_forward(transaction)` helper:
  1. Recreate `claim_status` with a new `value_flat TEXT` column (same table-recreation pattern proven in v3→v4 Entity Identity Reform).
  2. Backfill `value_flat` for existing rows in Rust: read each row's `value` JSON, run `flatten_json`, `UPDATE claim_status SET value_flat=? WHERE claim_id=?`.
  3. Create the FTS5 virtual table:
     ```sql
     CREATE VIRTUAL TABLE claim_search_fts USING fts5(
         subject,
         predicate,
         value_flat,
         domain,
         content='claim_status',
         content_rowid='claim_id',
         tokenize = 'trigram'
     );
     ```
  4. Create the three sync triggers (`claim_status_ai`, `claim_status_ad`, `claim_status_au`) — SQL bodies identical to the SQLite docs canonical external-content example, adapted to our columns.
  5. Backfill the index from existing rows:
     ```sql
     INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
       SELECT claim_id, subject, predicate, value_flat, domain FROM claim_status;
     INSERT INTO claim_search_fts(claim_search_fts) VALUES('optimize');
     ```
- `run_upgrade_step_reverse` — add a `(4, 5, 0)` arm that calls `run_fts5_reverse(transaction)`: `DROP TABLE claim_search_fts`, drop the three triggers, and recreate `claim_status` without `value_flat` (table-recreation, same pattern as v3→v4 reverse).
- Fresh-store DDL in `initialize_schema` — emit the `value_flat` column, the FTS5 table, and the three triggers so brand-new v5 stores are consistent.

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

### 3. Write path — populate `value_flat` (triggers handle the FTS row)

**File:** `src/semantic.rs` — `build_confirmation_material` (around line 7546)

The only change to the write path is that the `INSERT INTO claim_status(...)` now also writes the new `value_flat` column, computed via `flatten_json(&value)`. The FTS row is populated by the `AFTER INSERT` trigger, *not* by a hand-written `INSERT INTO claim_search_fts`. This keeps the write path single-site and impossible to forget.

Supersede and retract paths: **no change**. They already `UPDATE claim_status`; the `AFTER UPDATE` trigger keeps the index consistent. Active filtering happens at read time.

### 4. New `search_claims` method on `SemanticStore`

**File:** `src/semantic.rs` — new public method

```rust
/// Full-text search over confirmed, active claims using the FTS5 index.
/// Returns claims ranked by BM25 relevance to the query.
/// The query is run as a trigram phrase/substring query; prefix and boolean
/// operators are also supported by FTS5 MATCH syntax.
pub fn search_claims(
    &self,
    query: &str,
    domain: Option<&str>,
    top_k: usize,
) -> Result<Vec<ClaimSearchHit>> {
    // Build SQL: FTS5 MATCH + JOIN claim_status + filter active + optional domain
    // Column weights: subject=10, predicate=5, value_flat=1, domain=2
    // ORDER BY bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) ASC
    // LIMIT top_k
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

Replace the linear-scan + substring-filter with a call to `store.search_claims(...)`.

**`value` field resolution (verified against the codebase).** `claim_status` does **not** store the claim `value` — it lives encrypted in the confirmation event payload (`src/semantic.rs:3938-3948` shows `claims_current` resolving it via `confirmed_event_seq → events.object_id → decrypt_object → ConfirmationObject.claim.value`). `search_claims` must follow the **same** rehydration path: the FTS5 query returns the rowids + ranking, then for each hit the method joins `claim_status.confirmed_event_seq → events.object_id`, decrypts the payload, and extracts `value`. This keeps the response shape identical to today's (`claim_id`, `subject`, `predicate`, `value`, `domain`) and adds `score`.

### 6. `claim_timeline` interaction

No change needed — `claim_timeline` queries `claim_status` directly and is unaffected by the FTS5 index. Because superseded/retracted rows remain in the index (filtered at read time), historical timeline views still work.

### 7. Tests

**File:** `tests/semantic_search_fts5_v1.rs` (new)

Test cases:

1. **Basic match** — confirm a claim, `search_claims("reinvent")` finds it.
2. **Tokenized match** — search `"reinvent wheel"` finds `"Reinvent the Wheel"` (trigram indexes overlapping windows of both strings).
3. **Value search** — confirm a claim with `value: json!(["Tesla", "BMW"])`, search `"tesla"` finds it via `value_flat`.
4. **Thai search** — confirm a claim with subject `"บมจ. ปตท."`, search `"ปตท"` finds it (validates the trigram-over-porter decision).
5. **Domain filter** — search with `domain: Some("engineering")` filters to that domain only.
6. **BM25 ranking** — confirm 3 claims with different overlap to the query; assert the most-relevant one has the lowest (best) score.
7. **Supersede excludes from search** — confirm → supersede → search no longer returns the old claim (filtered by `superseded_by_event_seq IS NULL`), but a direct `claim_timeline` still sees it.
8. **Migration test** — stage a v4 store with existing claims, upgrade to v5, assert FTS5 backfill finds all pre-existing claims and `integrity-check` passes.
9. **Empty/edge queries** — empty string, special characters, very long query, query shorter than 3 characters (trigram minimum — document the behavior).

**File:** `tests/semantic_migration_v1.rs` (extend)

Add a v4→v5 round-trip: forward, verify FTS works, reverse, verify rollback drops the FTS table and `value_flat` column cleanly.

### 8. MCP tool manifest

**File:** `src/mcp/tools.rs`

No change to the `brain_search` tool definition (same args: `query`, `domain`, `top_k`). Update the description to mention BM25 ranking:

```
"Search confirmed claims in the semantic brain (BM25-ranked full-text search across subject, predicate, value, and domain)"
```

---

## Migration Plan

### Phase A — Schema + migration (v4→v5)

1. Add `value_flat TEXT` column to `claim_status` (table recreation, same pattern as v3→v4).
2. Create `claim_search_fts` FTS5 virtual table with `trigram` tokenizer.
3. Create the three sync triggers (`claim_status_ai`/`_ad`/`_au`).
4. Backfill `value_flat` in Rust, then `INSERT INTO claim_search_fts(...) SELECT ... FROM claim_status`.
5. `INSERT INTO claim_search_fts(claim_search_fts) VALUES('optimize')`.
6. Bump `CURRENT_DISK_SCHEMA_VERSION` to 5.
7. Test: migration on a copy of the live store (34 entities, 97 claims) and an `integrity-check` pass.

### Phase B — Write path

1. Add `flatten_json` helper.
2. In `build_confirmation_material`: compute `value_flat`, include it in the existing `INSERT INTO claim_status(...)`. The trigger populates the FTS row.
3. Supersede/retract paths: **no change** (already `UPDATE claim_status`; trigger + read-time filter handle the rest).
4. Test: confirm a claim → search finds it immediately.

### Phase C — Read path

1. Add `search_claims` method.
2. Rewrite `handle_brain_search` handler.
3. Test: all 9 test cases pass.
4. Run full workspace test suite.

### Phase D — Production migration

1. Stop Docker container.
2. Backup v4 store.
3. `docker compose build && docker compose up -d`.
4. `recovery upgrade` (v4→v5).
5. Verify: `brain_search "reinvent wheel"` returns the claim; `brain_search "ปตท"` returns the Thai claim.

---

## Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| FTS5 not compiled into bundled SQLite | Low | rusqlite `bundled` feature compiles SQLite with FTS5 enabled by default. Verify with `SELECT fts5(?)` during migration. |
| External-content table drift (FTS5 out of sync with claim_status) | **Low (improved from v1)** | v1's risk was Medium because it relied on three hand-written INSERT sites. v2 uses SQL triggers, which the SQLite docs identify as the canonical drift cure. Add `INSERT INTO fts(fts) VALUES('integrity-check')` to a health-check endpoint. |
| `value_flat` column addition requires table recreation | Medium | Same table-recreation pattern as v3→v4 (proven in Entity Identity Reform). Atomic, reversible. |
| `trigram` index larger than `porter unicode61` | Low | At 10k rows × <200 bytes the absolute size is still small. `VALUES('optimize')` compacts segments after backfill. |
| Loss of English stemming vs v1 (`running` no longer matches `run`) | Low | Trigram still matches the full word `running`. Stemming is a nice-to-have, not a requirement; the trigram choice buys Thai/CJK support which is a first-class `SubjectShape`. |
| Trigram requires ≥3 characters per query token | Low | Claim search keywords are typically ≥3 chars. Sub-3-char queries (e.g. "AI") are documented as a known limitation; fall back to `claim_timeline` for those. |
| BM25 column weights need tuning | Low | Weights are a single constant array in `search_claims`; easy to adjust post-ship. Start with subject=10, predicate=5, value=1, domain=2. |
| Existing `brain_search` callers expect substring behavior | Low | Trigram MATCH is a superset of substring for ≥3-char queries. Old behavior preserved as a subset. |

---

## Definition of Done

### Functional
- [ ] `brain_search "reinvent wheel"` returns the "Reinvent the Wheel" claim (ranked by BM25)
- [ ] `brain_search "tesla"` finds claims whose `value` is a JSON array containing "Tesla"
- [ ] `brain_search "ปตท"` finds the Thai subject claim (validates trigram choice)
- [ ] `brain_search` with `domain: "engineering"` filters correctly
- [ ] Superseded/retracted claims are excluded from search results but visible in `claim_timeline`
- [ ] BM25 ranking: most-relevant result appears first

### Architectural
- [ ] `claim_search_fts` FTS5 table exists with `trigram` tokenizer
- [ ] Three SQL triggers (`claim_status_ai`/`_ad`/`_au`) exist and keep the index in sync
- [ ] `value_flat` column on `claim_status`
- [ ] `search_claims` method uses FTS5 MATCH, not linear scan
- [ ] No hand-written `INSERT INTO claim_search_fts` in any write path (triggers only)
- [ ] Schema version bumped to 5 with reversible migration

### Quality
- [ ] `cargo test --workspace` passes
- [ ] `tests/semantic_search_fts5_v1.rs` — 9 new tests pass
- [ ] `tests/semantic_migration_v1.rs` — v4→v5 round-trip passes
- [ ] `INSERT INTO claim_search_fts(claim_search_fts) VALUES('integrity-check')` returns no rows after migration
- [ ] Docker rebuild + browser verify

---

## Key Files

| File | Change |
|------|--------|
| `src/semantic.rs` | Schema bump, FTS5 DDL, triggers, migration (`run_fts5_forward`/`reverse`), `flatten_json`, `search_claims`; `build_confirmation_material` adds `value_flat` to its INSERT |
| `src/mcp/handlers.rs` | Rewrite `handle_brain_search` to call `search_claims` |
| `src/mcp/tools.rs` | Update `brain_search` description (optional) |
| `tests/semantic_search_fts5_v1.rs` | New test file (9 tests) |
| `tests/semantic_migration_v1.rs` | Extend with v4→v5 round-trip |
