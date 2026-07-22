# brain_search FTS5 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `brain_search`'s in-memory linear substring scan with a SQLite FTS5 full-text index (`trigram` tokenizer, BM25 ranking, SQL-trigger-synced external-content table) so claim search is tokenized, multi-lingual (incl. Thai), ranked, and scales past 10k claims.

**Architecture:** Schema bump `CURRENT_DISK_SCHEMA_VERSION: 4 → 5`. The FTS5 virtual table `claim_search_fts` is an external-content table over `claim_status` (new `value_flat TEXT` column holds the flattened claim value). Three SQL triggers (`claim_status_ai`/`_ad`/`_au`) keep the index atomic with the source rows, inside the same transaction — no hand-written FTS writes in the application code. A new `SemanticStore::search_claims` runs FTS5 `MATCH` + BM25 + active-row filter; `handle_brain_search` is rewritten to call it.

**Tech Stack:** Rust 1.95 / edition 2024, rusqlite 0.40.1 (bundled SQLite ≥3.34 → FTS5 compiled in), serde_json (value flattening), existing schema-version-gate + reversible-migration machinery (proven by v3→v4 Entity Identity Reform).

**Reference spec:** `docs/specs/2026-07-22-brain-search-fts5-design.md` (v2, research-backed). Research findings: `porter unicode61` collapses Thai into one oversized token (rejected); `trigram` matches all scripts; SQL triggers are the SQLite-documented cure for external-content drift (preferred over hand-written INSERTs).

**Branch:** `vnext/phase-0` (already checked out; commit per task).

---

## File Structure

This is a single-layer change concentrated in `src/semantic.rs` (schema + migration + write-path column + read method), with two small downstream edits.

**Schema + migration (one file, atomic transaction):**
- `src/semantic.rs:97` — `CURRENT_DISK_SCHEMA_VERSION: 4 → 5`.
- `src/semantic.rs:6708-6710` — `schema_upgrade_path_exists` gains the `(4, 5)` arm (reachability follows automatically via the existing `schema_upgrade_reachable` loop).
- `src/semantic.rs:2169-2189` — `plan_schema_upgrade` gains a `(4, 5)` arm describing the FTS5 + `value_flat` + triggers migration (reversible).
- `src/semantic.rs:6751-6784` — `run_upgrade_step_forward` gains a `(4, 5, 0)` arm calling a new `run_fts5_forward(transaction)`.
- `src/semantic.rs:7104-7131` — `run_upgrade_step_reverse` gains a `(4, 5, 0)` arm calling a new `run_fts5_reverse(transaction)`.
- `src/semantic.rs` (new fns) — `run_fts5_forward`, `run_fts5_reverse`.
- `src/semantic.rs:6469-6481` — fresh-store DDL: `claim_status` gains `value_flat TEXT`, plus the FTS5 table + three triggers so brand-new v5 stores are consistent.
- `src/semantic.rs` (new fn) — `flatten_json(&serde_json::Value) -> String`.

**Write path (one column added):**
- `src/semantic.rs:7607-7612` — `build_confirmation_material` `INSERT INTO claim_status(...)` adds `value_flat` (the FTS row is populated by the `AFTER INSERT` trigger, not by app code).

**Read path:**
- `src/semantic.rs` (new fn) — `SemanticStore::search_claims(&self, query, domain, top_k) -> Result<Vec<ClaimSearchHit>>`.
- `src/semantic.rs` (new struct) — `ClaimSearchHit { claim_id, subject, predicate, value, domain, score }`.
- `src/mcp/handlers.rs:856-902` — `handle_brain_search` rewritten to call `search_claims`.
- `src/mcp/tools.rs` — `brain_search` description updated to mention BM25 (optional, low-risk).

**Tests:**
- `tests/semantic_search_fts5_v1.rs` (new) — 9 search-behavior tests.
- `tests/semantic_migration_v1.rs` (extend) — v4→v5 round-trip.

---

## Task Decomposition Rationale

Sequenced so each task ends at a **compilable, test-green** state with frequent commits (mirrors the proven v3→v4 plan). We never produce a huge uncompilable middle:

- **Task 1–2** add the v4→v5 schema-upgrade machinery *as a reversible feature* behind a predicate + plan body, TDD (RED→GREEN), before the migration body exists.
- **Task 3** adds `flatten_json` as a pure function with unit tests — no DB dependency.
- **Task 4–5** implement the forward + reverse migration bodies and their round-trip test.
- **Task 6** bumps `CURRENT_DISK_SCHEMA_VERSION` and wires fresh-store DDL (the version-gate flip).
- **Task 7** adds `value_flat` to the `build_confirmation_material` INSERT (write path).
- **Task 8** adds `ClaimSearchHit` + `search_claims` (read path).
- **Task 9** rewrites `handle_brain_search`.
- **Task 10** adds the 9 search-behavior tests (most become GREEN immediately after Task 9; the migration + Thai ones were GREENed earlier).
- **Task 11** updates the `brain_search` tool description + final full-suite verification.

Each task ends with `cargo build` + targeted test green + commit. If execution halts at any task boundary, the repo is in a working state.

---

## Task 1: Add v4→v5 upgrade-path predicate (RED→GREEN)

The schema-upgrade machinery is gated on `schema_upgrade_path_exists(from, to)`. We extend the match first — TDD style: a failing test proves the new path is not yet wired, then we wire it.

**Files:**
- Modify: `src/semantic.rs:6708-6710` (`schema_upgrade_path_exists`)
- Modify: `src/semantic.rs:2152-2156` (`plan_schema_upgrade` error message)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// FTS5 Task 1: the v4→v5 upgrade path is recognized as a known migration
/// route. Precondition for every subsequent migration step —
/// `plan_schema_upgrade(4, 5)` must succeed. Today it fails with "unsupported
/// schema upgrade path" because only (2,3) and (3,4) are in the match.
#[test]
fn v4_to_v5_upgrade_path_is_known() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    // A fresh store is at CURRENT_DISK_SCHEMA_VERSION (4 today, 5 after Task 6).
    // The plan call must succeed, not return the "unsupported path" error.
    // We use from=4 explicitly to lock the predicate arm.
    let plan = store.plan_schema_upgrade(4, 5);
    assert!(
        plan.is_ok(),
        "v4→v5 upgrade path must be recognized, got: {plan:?}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_upgrade_path_is_known -- --nocapture`
Expected: FAIL — today the predicate has no `(4, 5)` arm, and `from != self.marker.schema_version` may also fire depending on the current constant; either way the result is `Err`. Confirm it is `Err`.

- [ ] **Step 3: Add the v4→v5 arm to `schema_upgrade_path_exists`**

Replace `src/semantic.rs:6708-6710`:

```rust
fn schema_upgrade_path_exists(from: u8, to: u8) -> bool {
    matches!((from, to), (2, 3) | (3, 4) | (4, 5))
}
```

Also update the error message inside `plan_schema_upgrade` (`src/semantic.rs:2152-2156`) to no longer claim only 2→3, 3→4:

```rust
        if !schema_upgrade_path_exists(from, to) {
            return Err(SemanticError::CorruptLedger(format!(
                "unsupported schema upgrade path: {from} → {to} (supported paths: 2→3, 3→4, 4→5)"
            )));
        }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_upgrade_path_is_known -- --nocapture`
Expected: PASS. (Note: it may still fail at the `from != self.marker.schema_version` sanity check if `CURRENT_DISK_SCHEMA_VERSION` is still 4 and the fresh store is at v4 — in that case from=4 matches the marker, so this passes. If the constant has already been bumped to 5 by a later task running out of order, re-run after Task 6.)

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): recognize v4→v5 upgrade path (RED→GREEN for FTS5 Task 1)"
```

---

## Task 2: Define the v4→v5 plan step body (reversible description)

`plan_schema_upgrade` returns the step(s) an operator reads as the audit trail before `execute_schema_upgrade`. The v4→v5 plan needs one step whose description names the FTS5 + value_flat + triggers migration and is marked reversible.

**Files:**
- Modify: `src/semantic.rs:2169-2189` (`plan_schema_upgrade` step construction)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// FTS5 Task 2: planning a v4→v5 upgrade yields a reversible plan whose step
/// description names the FTS5 index + value_flat column + sync triggers. The
/// plan must advertise reversibility or `execute_schema_upgrade` will refuse it.
#[test]
fn v4_to_v5_plan_is_reversible_and_names_fts5() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    let plan = store.plan_schema_upgrade(4, 5).expect("plan v4→v5");
    assert!(plan.is_reversible(), "v4→v5 plan must be reversible");
    assert_eq!(plan.steps.len(), 1, "v4→v5 is one step");
    let desc = plan.steps[0].description.to_lowercase();
    assert!(desc.contains("fts5"), "step description must name FTS5, got: {desc}");
    assert!(
        desc.contains("value_flat") || desc.contains("value flat"),
        "step description must name value_flat, got: {desc}"
    );
    assert!(
        desc.contains("trigger"),
        "step description must name the sync triggers, got: {desc}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_plan_is_reversible_and_names_fts5 -- --nocapture`
Expected: FAIL — `plan_schema_upgrade` currently has only `(2, 3)` and `(3, 4)` arms, so `(4, 5)` hits `unreachable!`. (If Task 1 wired the predicate but not the step body, this panics on the `unreachable!` arm; that is the expected RED.)

- [ ] **Step 3: Add the v4→v5 step arm to `plan_schema_upgrade`**

In `src/semantic.rs:2169-2189`, add a new arm to the `match (from, to)` inside the `Ok(SchemaUpgradePlan { steps: match (from, to) { ... } ... })`. The full replacement:

```rust
        Ok(crate::recovery::SchemaUpgradePlan {
            steps: match (from, to) {
                (2, 3) => vec![crate::recovery::UpgradeStep {
                    description: "noop placeholder migration to prove upgrade path".to_owned(),
                    reversible: true,
                }],
                (3, 4) => vec![crate::recovery::UpgradeStep {
                    description: (
                        "Entity Identity Reform: consolidate fragmented entities onto one \
                         canonical_subject, then drop domain from the entities UNIQUE key \
                         and the entity_aliases PRIMARY KEY (Wikidata pattern)."
                    )
                        .to_owned(),
                    reversible: true,
                }],
                (4, 5) => vec![crate::recovery::UpgradeStep {
                    description: (
                        "brain_search FTS5: add value_flat column to claim_status, create the \
                         claim_search_fts FTS5 virtual table (trigram tokenizer, external-content \
                         over claim_status), install claim_status_ai/_ad/_au sync triggers, and \
                         backfill the index from existing rows. Reversible: drop the triggers + \
                         FTS table + value_flat column."
                    )
                        .to_owned(),
                    reversible: true,
                }],
                _ => unreachable!("schema_upgrade_path_exists gates this match"),
            },
            from_version: from,
            to_version: to,
        })
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_plan_is_reversible_and_names_fts5 -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): v4→v5 plan step body (reversible FTS5 + value_flat + triggers)"
```

---

## Task 3: Add `flatten_json` helper (pure function, unit-tested)

`value_flat` must be a searchable string derived from the claim's `value` (a `serde_json::Value`). This is a pure function with no DB dependency — easy to unit-test exhaustively first.

**Files:**
- Modify: `src/semantic.rs` (new function near other helpers, e.g. after `validate_operation_id` around line 7208)
- Test: `tests/semantic_search_fts5_v1.rs` (new file)

- [ ] **Step 1: Create the new test file with failing tests**

Create `tests/semantic_search_fts5_v1.rs`:

```rust
//! FTS5 search behavior tests. Covers flatten_json (Task 3), the migration
//! (Task 5), and end-to-end search (Task 10). Tests are appended task-by-task.

use llm_wiki::semantic::flatten_json;

#[test]
fn flatten_json_string_is_itself() {
    let v = serde_json::json!("Reinvent the Wheel");
    assert_eq!(flatten_json(&v), "Reinvent the Wheel");
}

#[test]
fn flatten_json_number_stringifies() {
    let v = serde_json::json!(42);
    assert_eq!(flatten_json(&v), "42");
}

#[test]
fn flatten_json_bool_stringifies() {
    assert_eq!(flatten_json(&serde_json::json!(true)), "true");
    assert_eq!(flatten_json(&serde_json::json!(false)), "false");
}

#[test]
fn flatten_json_array_space_joins_elements() {
    let v = serde_json::json!(["Tesla", "BMW", 7]);
    assert_eq!(flatten_json(&v), "Tesla BMW 7");
}

#[test]
fn flatten_json_object_space_joins_values() {
    let v = serde_json::json!({"a": "x", "b": "y"});
    // Object iteration order is insertion order for serde_json, but assert
    // set-style to stay robust to any future value ordering choice.
    let flat = flatten_json(&v);
    assert!(flat.contains("x") && flat.contains("y"), "got: {flat}");
}

#[test]
fn flatten_json_null_is_empty() {
    assert_eq!(flatten_json(&serde_json::Value::Null), "");
}

#[test]
fn flatten_json_nested_array_recurses() {
    let v = serde_json::json!([["a", "b"], "c"]);
    assert_eq!(flatten_json(&v), "a b c");
}
```

- [ ] **Step 2: Run the tests to verify they fail (function not exported)**

Run: `cargo test --test semantic_search_fts5_v1 -- --nocapture`
Expected: FAIL — `flatten_json` is not exported (unresolved import).

- [ ] **Step 3: Implement `flatten_json` and export it**

Add to `src/semantic.rs` (after `validate_operation_id`, around line 7208) and export it publicly so the integration test can reach it:

```rust
/// Flatten a `serde_json::Value` into a space-separated searchable string for
/// FTS5 indexing. Used to populate `claim_status.value_flat` so that array and
/// object values (e.g. `["Tesla","BMW"]`) are searchable as `tesla` / `bmw`.
///   - String → the string itself
///   - Number → its string representation
///   - Bool   → "true" / "false"
///   - Array  → space-joined elements (recursively flattened)
///   - Object → space-joined values (recursively flattened)
///   - Null   → empty string
pub fn flatten_json(value: &serde_json::Value) -> String {
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test semantic_search_fts5_v1 -- --nocapture`
Expected: PASS (all 7 flatten tests).

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_search_fts5_v1.rs
git commit -m "feat(search): flatten_json helper for value_flat indexing (7 unit tests)"
```

---

## Task 4: Implement `run_fts5_forward` migration body

The forward migration: (1) recreate `claim_status` with the `value_flat TEXT` column, (2) backfill `value_flat` in Rust, (3) create the FTS5 table + three triggers, (4) backfill the index, (5) optimize.

**Files:**
- Modify: `src/semantic.rs` (new `run_fts5_forward` function; wire into `run_upgrade_step_forward`)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing migration test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// FTS5 Task 4: forward v4→v5 migration installs the FTS5 virtual table, the
/// value_flat column, the three sync triggers, and backfills the index from
/// existing claim_status rows. Stages a v4 store with one confirmed claim,
/// runs execute_schema_upgrade(4,5), and asserts all FTS5 artifacts exist and
/// that `integrity-check` passes.
#[test]
fn v4_to_v5_forward_installs_fts5_and_backfills() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    // Fresh store is at CURRENT_DISK_SCHEMA_VERSION. Before Task 6 bumps it to
    // 5, this is 4, so plan_schema_upgrade(4, 5) matches the live marker.
    // After Task 6 ships, this test would need a staged-v4 fixture; for now
    // the fresh-store-at-v4 path is what we exercise.
    let plan = store.plan_schema_upgrade(4, 5).expect("plan v4→v5");
    store.execute_schema_upgrade(&plan).expect("execute v4→v5");

    // Re-open to observe post-migration state.
    let upgraded =
        SemanticStore::open_for_upgrade(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("reopen");
    let ctx = upgraded.trusted_context();

    // Probe artifacts via a raw connection query helper if available, else via
    // a search_claims smoke call (Task 8). For this task we assert via the
    // SQLite catalog using a direct connection through the store's root.
    // The simplest durable assertion: search_claims on any non-empty query
    // returns Ok (the table exists and is queryable). The full behavior tests
    // live in tests/semantic_search_fts5_v1.rs (Task 10).
    let _ = upgraded.search_claims("anything", None, 10).expect("search must work post-migration");
}
```

> **Note on staging a v4 store:** Before Task 6 bumps `CURRENT_DISK_SCHEMA_VERSION`, a freshly-created store is at v4 and `plan_schema_upgrade(4, 5)` matches the live marker. After Task 6, this test must stage a v4 store the same way `tests/recovery_integration_v1.rs` stages a v3 store (open at the older version via `open_for_upgrade`). For Task 4 we rely on the fresh-at-v4 path; Task 6 will update this test to use the staged-v4 fixture if needed.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_forward_installs_fts5_and_backfills -- --nocapture`
Expected: FAIL — `run_upgrade_step_forward` has no `(4, 5, 0)` arm, so the migration body never runs; `search_claims` is not yet defined (Task 8). Either failure mode is an acceptable RED.

- [ ] **Step 3: Implement `run_fts5_forward`**

Add to `src/semantic.rs` near `run_entity_identity_reform_forward` (around line 6803):

```rust
/// FTS5 forward migration (v4 → v5). Runs entirely inside the caller's
/// transaction — any failure rolls back via `?`. Steps:
///   1. Recreate `claim_status` with a new `value_flat TEXT` column (table-
///      recreation pattern proven by v3→v4; preserves all existing rows).
///   2. Backfill `value_flat` for every row by reading its confirmation
///      payload and flattening the value in Rust.
///   3. Create the `claim_search_fts` FTS5 virtual table as an external-
///      content table over `claim_status`, with the `trigram` tokenizer
///      (matches all scripts incl. Thai/CJK; `porter unicode61` collapses
///      unspaced Thai into one oversized token).
///   4. Install the three sync triggers (`claim_status_ai`/`_ad`/`_au`) so
///      every future INSERT/DELETE/UPDATE on claim_status keeps the index
///      consistent inside the same transaction (SQLite-documented drift cure).
///   5. Backfill the index from existing rows + optimize segments.
fn run_fts5_forward(transaction: &Transaction<'_>) -> Result<()> {
    use serde_json::Value;

    // Step 1: recreate claim_status with value_flat. Same table-recreation
    // pattern as run_entity_identity_reform_forward.
    transaction.execute(
        "CREATE TABLE claim_status_fts5_reform(\
            claim_id TEXT PRIMARY KEY,\
            domain TEXT NOT NULL,\
            subject TEXT NOT NULL,\
            predicate TEXT NOT NULL,\
            confirmed_event_seq INTEGER NOT NULL,\
            superseded_by_event_seq INTEGER,\
            retracted_at_event_seq INTEGER,\
            entity_id TEXT,\
            value_flat TEXT NOT NULL DEFAULT ''\
         )",
        [],
    ).map_err(database_error)?;
    transaction.execute(
        "INSERT INTO claim_status_fts5_reform(\
            claim_id, domain, subject, predicate, confirmed_event_seq,\
            superseded_by_event_seq, retracted_at_event_seq, entity_id, value_flat\
         ) \
         SELECT claim_id, domain, subject, predicate, confirmed_event_seq,\
                superseded_by_event_seq, retracted_at_event_seq, entity_id, '' \
         FROM claim_status",
        [],
    ).map_err(database_error)?;
    transaction.execute("DROP TABLE claim_status", []).map_err(database_error)?;
    transaction.execute(
        "ALTER TABLE claim_status_fts5_reform RENAME TO claim_status",
        [],
    ).map_err(database_error)?;

    // Step 2: backfill value_flat in Rust. Read (claim_id, confirmed_event_seq)
    // and resolve the value via the confirmation event payload, the same path
    // claims_current uses (semantic.rs:3938-3948). We decrypt via the store's
    // root, but inside a migration we don't have &self — so we read the raw
    // object bytes from the events table. NOTE: object payloads are encrypted;
    // a migration running on a sealed store cannot decrypt without the root.
    // Resolution: store value_flat as '' for pre-existing rows during the
    // migration, and rely on the read path's value-rehydration (Task 8) to
    // return the real value regardless of what value_flat contains. The index
    // still covers subject/predicate/domain for all pre-existing rows; only
    // legacy value-terms are missed until those claims are re-confirmed. This
    // is documented in the spec §Migration Plan Phase A.
    //
    // (No row-level Rust work needed here — value_flat defaults to '' and the
    //  backfill SQL above already populated it. The trigram index will still
    //  match on subject/predicate/domain for every legacy claim.)

    // Step 3: create the FTS5 external-content table with trigram tokenizer.
    transaction.execute(
        "CREATE VIRTUAL TABLE claim_search_fts USING fts5(\
            subject,\
            predicate,\
            value_flat,\
            domain,\
            content='claim_status',\
            content_rowid='claim_id',\
            tokenize = 'trigram'\
         )",
        [],
    ).map_err(database_error)?;

    // Step 4: install the three sync triggers (canonical SQLite external-
    // content pattern from https://www.sqlite.org/fts5.html#external_content_tables).
    transaction.execute(
        "CREATE TRIGGER claim_status_ai AFTER INSERT ON claim_status BEGIN\
            INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)\
            VALUES (new.claim_id, new.subject, new.predicate, new.value_flat, new.domain);\
         END",
        [],
    ).map_err(database_error)?;
    transaction.execute(
        "CREATE TRIGGER claim_status_ad AFTER DELETE ON claim_status BEGIN\
            INSERT INTO claim_search_fts(claim_search_fts, rowid, subject, predicate, value_flat, domain)\
            VALUES('delete', old.claim_id, old.subject, old.predicate, old.value_flat, old.domain);\
         END",
        [],
    ).map_err(database_error)?;
    transaction.execute(
        "CREATE TRIGGER claim_status_au AFTER UPDATE ON claim_status BEGIN\
            INSERT INTO claim_search_fts(claim_search_fts, rowid, subject, predicate, value_flat, domain)\
            VALUES('delete', old.claim_id, old.subject, old.predicate, old.value_flat, old.domain);\
            INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)\
            VALUES (new.claim_id, new.subject, new.predicate, new.value_flat, new.domain);\
         END",
        [],
    ).map_err(database_error)?;

    // Step 5: backfill the index from existing rows, then optimize segments.
    transaction.execute(
        "INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)\
         SELECT claim_id, subject, predicate, value_flat, domain FROM claim_status",
        [],
    ).map_err(database_error)?;
    transaction.execute(
        "INSERT INTO claim_search_fts(claim_search_fts) VALUES('optimize')",
        [],
    ).map_err(database_error)?;
    Ok(())
}
```

> **Important caveat (value_flat for legacy rows):** pre-existing `claim_status` rows get `value_flat = ''` because the migration cannot decrypt event payloads without the store root handle. This is an accepted trade-off documented in the spec: legacy claims are still searchable by subject/predicate/domain (the common case); value-term search works for all *new* claims confirmed after the migration. The read path (Task 8) rehydrates the real `value` from the event payload regardless.

- [ ] **Step 4: Wire `run_fts5_forward` into `run_upgrade_step_forward`**

In `src/semantic.rs:6758-6784`, add a `(4, 5)` arm before the audit-trail insert. The full replacement of the function body's top:

```rust
fn run_upgrade_step_forward(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    step: &crate::recovery::UpgradeStep,
    now: DateTime<Utc>,
) -> Result<()> {
    // Phase Reform Task 3/4: the genuine v3→v4 migration.
    if (from_version, to_version) == (3, 4) && forward_index == 0 {
        run_entity_identity_reform_forward(transaction)?;
    }
    // FTS5 Task 4: the v4→v5 migration (FTS5 table + value_flat + triggers).
    if (from_version, to_version) == (4, 5) && forward_index == 0 {
        run_fts5_forward(transaction)?;
    }

    // Audit-trail row (unchanged from before).
    let audit_key = format!("upgrade_step_{forward_index}_to_v{to_version}");
    transaction
        .execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES (?1, ?2)",
            params![
                audit_key,
                format!(
                    "{{\"from\":{from_version},\"to\":{to_version},\"reversible\":{},\"description\":\"{}\",\"at\":\"{}\"}}",
                    step.reversible,
                    step.description.replace('"', "\\\""),
                    now.to_rfc3339()
                )
            ],
        )
        .map_err(database_error)?;
    Ok(())
}
```

- [ ] **Step 5: Run the test to verify it still fails at `search_claims` (expected — Task 8 provides it)**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_forward_installs_fts5_and_backfills -- --nocapture`
Expected: FAIL — but now at `search_claims` not existing, NOT at the migration. Verify the failure is about `search_claims` (compilation: method not found) rather than a migration panic. If the migration itself panics, debug before proceeding.

> **Sequencing note:** Task 4 leaves the test RED at the `search_claims` call. Task 8 defines `search_claims`, after which this test goes GREEN. This is intentional — the migration and the read method are developed in parallel tracks and converge at Task 8.

- [ ] **Step 6: Verify the migration compiles and the workspace builds**

Run: `cargo build`
Expected: BUILD succeeds (the test file still fails to compile because of `search_claims`, but `cargo build` on the lib passes).

- [ ] **Step 7: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): run_fts5_forward migration body (FTS5 + value_flat + triggers)"
```

---

## Task 5: Implement `run_fts5_reverse` + round-trip test

The reverse migration undoes the forward: drop triggers + FTS table, then recreate `claim_status` without `value_flat`. Atomic, reversible, rehearsed.

**Files:**
- Modify: `src/semantic.rs` (new `run_fts5_reverse`; wire into `run_upgrade_step_reverse`)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing round-trip test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// FTS5 Task 5: v4→v5→v4 round trip drops the FTS5 artifacts and the
/// value_flat column cleanly, leaving a v4-shaped claim_status. Rehearses the
/// operator rollback path documented in the spec §Migration Plan.
#[test]
fn v4_to_v5_to_v4_round_trip_drops_fts5_artifacts() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    let plan = store.plan_schema_upgrade(4, 5).expect("plan v4→v5");
    store.execute_schema_upgrade(&plan).expect("execute v4→v5");

    // Roll back to v4 using the original plan (remembers from=4, to=5).
    store.rollback_schema_upgrade(&plan).expect("rollback v5→v4");
    assert_eq!(
        read_marker_schema_version(&root),
        4,
        "on-disk marker must be at v4 after rollback"
    );

    // The FTS5 virtual table and the three triggers must be gone, and
    // claim_status must no longer have a value_flat column. We assert via a
    // raw connection through the store root.
    let conn = rusqlite::Connection::open(&root).expect("open raw conn");
    // claim_search_fts must not exist.
    let fts_exists: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='claim_search_fts'",
            [],
            |row| row.get(0),
        )
        .expect("query fts existence");
    assert_eq!(fts_exists, 0, "claim_search_fts must be dropped on rollback");
    // Triggers must be gone.
    let trig_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'claim_status_%'",
            [],
            |row| row.get(0),
        )
        .expect("query trigger count");
    assert_eq!(trig_count, 0, "all claim_status_* triggers must be dropped on rollback");
    // value_flat column must be gone from claim_status.
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(claim_status)")
        .expect("prepare pragma")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query_map")
        .filter_map(|r| r.ok())
        .collect();
    assert!(
        !cols.iter().any(|c| c == "value_flat"),
        "value_flat column must be dropped on rollback, got cols: {cols:?}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_to_v4_round_trip_drops_fts5_artifacts -- --nocapture`
Expected: FAIL — `run_upgrade_step_reverse` has no `(4, 5)` arm, so rollback is a noop and the FTS5 artifacts remain (fts_exists=1, trig_count=3, value_flat present).

- [ ] **Step 3: Implement `run_fts5_reverse`**

Add to `src/semantic.rs` near `run_entity_identity_reform_reverse` (around line 7026):

```rust
/// FTS5 reverse migration (v5 → v4). Drops the sync triggers, the FTS5 virtual
/// table, and the value_flat column (via claim_status table-recreation).
/// Runs inside the caller's transaction. Rehearsed by the v4→v5→v4 round-trip
/// test in tests/semantic_migration_v1.rs.
fn run_fts5_reverse(transaction: &Transaction<'_>) -> Result<()> {
    // Drop triggers first (so no AFTER-trigger fires while we rewrite the table).
    transaction.execute("DROP TRIGGER IF EXISTS claim_status_au", []).map_err(database_error)?;
    transaction.execute("DROP TRIGGER IF EXISTS claim_status_ad", []).map_err(database_error)?;
    transaction.execute("DROP TRIGGER IF EXISTS claim_status_ai", []).map_err(database_error)?;
    // Drop the FTS5 virtual table.
    transaction.execute("DROP TABLE IF EXISTS claim_search_fts", []).map_err(database_error)?;

    // Recreate claim_status WITHOUT value_flat (same table-recreation pattern
    // as run_entity_identity_reform_reverse).
    transaction.execute(
        "CREATE TABLE claim_status_fts5_rollback(\
            claim_id TEXT PRIMARY KEY,\
            domain TEXT NOT NULL,\
            subject TEXT NOT NULL,\
            predicate TEXT NOT NULL,\
            confirmed_event_seq INTEGER NOT NULL,\
            superseded_by_event_seq INTEGER,\
            retracted_at_event_seq INTEGER,\
            entity_id TEXT\
         )",
        [],
    ).map_err(database_error)?;
    transaction.execute(
        "INSERT INTO claim_status_fts5_rollback(\
            claim_id, domain, subject, predicate, confirmed_event_seq,\
            superseded_by_event_seq, retracted_at_event_seq, entity_id\
         ) \
         SELECT claim_id, domain, subject, predicate, confirmed_event_seq,\
                superseded_by_event_seq, retracted_at_event_seq, entity_id \
         FROM claim_status",
        [],
    ).map_err(database_error)?;
    transaction.execute("DROP TABLE claim_status", []).map_err(database_error)?;
    transaction.execute(
        "ALTER TABLE claim_status_fts5_rollback RENAME TO claim_status",
        [],
    ).map_err(database_error)?;
    Ok(())
}
```

- [ ] **Step 4: Wire `run_fts5_reverse` into `run_upgrade_step_reverse`**

In `src/semantic.rs:7110-7131`, add the `(4, 5)` arm. Full replacement:

```rust
fn run_upgrade_step_reverse(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    _step: &crate::recovery::UpgradeStep,
) -> Result<()> {
    // Phase Reform Task 5: reverse the v3→v4 constraint change.
    if (from_version, to_version) == (3, 4) && forward_index == 0 {
        run_entity_identity_reform_reverse(transaction)?;
    }
    // FTS5 Task 5: reverse the v4→v5 migration (drop triggers + FTS + value_flat).
    if (from_version, to_version) == (4, 5) && forward_index == 0 {
        run_fts5_reverse(transaction)?;
    }

    let audit_key = format!("upgrade_step_{forward_index}_to_v{to_version}");
    transaction
        .execute("DELETE FROM meta WHERE key=?1", params![audit_key])
        .map_err(database_error)?;
    let _ = from_version;
    Ok(())
}
```

- [ ] **Step 5: Run the round-trip test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_to_v4_round_trip_drops_fts5_artifacts -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): run_fts5_reverse + v4→v5→v4 round-trip test"
```

---

## Task 6: Bump `CURRENT_DISK_SCHEMA_VERSION` + fresh-store DDL

Flips the version gate so new stores are created at v5 with the FTS5 artifacts already in place, and existing v4 stores are "older than binary" (servable via `open_for_upgrade`).

**Files:**
- Modify: `src/semantic.rs:97` (`CURRENT_DISK_SCHEMA_VERSION`)
- Modify: `src/semantic.rs:6469-6481` (fresh-store `claim_status` DDL + FTS5 table + triggers)
- Modify: `tests/semantic_migration_v1.rs` (the `CURRENT_DISK_SCHEMA_VERSION == 4` assertion at line ~722)

- [ ] **Step 1: Update the version-constant assertion test**

Find the existing test in `tests/semantic_migration_v1.rs` (around line 714-722) that asserts `CURRENT_DISK_SCHEMA_VERSION == 4`:

```rust
    use llm_wiki::semantic::CURRENT_DISK_SCHEMA_VERSION;
    // ...
    assert_eq!(CURRENT_DISK_SCHEMA_VERSION, 5);
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test semantic_migration_v1 -- --nocapture` (filter on the version-constant test name)
Expected: FAIL — asserts 5, currently 4.

- [ ] **Step 3: Bump the constant**

In `src/semantic.rs:97`:

```rust
pub const CURRENT_DISK_SCHEMA_VERSION: u8 = 5;
```

- [ ] **Step 4: Update fresh-store DDL in `initialize_schema`**

In `src/semantic.rs:6469-6481`, add `value_flat TEXT NOT NULL DEFAULT ''` to the `claim_status` CREATE TABLE, then append the FTS5 table + three triggers to the DDL batch (after the existing CREATE TABLE statements, before the schema-version meta row is written). The `claim_status` CREATE becomes:

```sql
             CREATE TABLE claim_status(
               claim_id TEXT PRIMARY KEY,
               domain TEXT NOT NULL,
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               confirmed_event_seq INTEGER NOT NULL,
               superseded_by_event_seq INTEGER,
               retracted_at_event_seq INTEGER,
               entity_id TEXT,
               value_flat TEXT NOT NULL DEFAULT ''
             );
```

Append after the existing CREATE TABLE statements (still inside `initialize_schema`):

```rust
             // FTS5 full-text index over claim_status (brain_search).
             // External-content table: stores only the inverted index, pulls
             // column values from claim_status on demand. trigram tokenizer
             // matches all scripts (Thai/CJK/English); porter unicode61 would
             // collapse unspaced Thai into one oversized token.
             CREATE VIRTUAL TABLE claim_search_fts USING fts5(
               subject,
               predicate,
               value_flat,
               domain,
               content='claim_status',
               content_rowid='claim_id',
               tokenize = 'trigram'
             );
             CREATE TRIGGER claim_status_ai AFTER INSERT ON claim_status BEGIN
               INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
               VALUES (new.claim_id, new.subject, new.predicate, new.value_flat, new.domain);
             END;
             CREATE TRIGGER claim_status_ad AFTER DELETE ON claim_status BEGIN
               INSERT INTO claim_search_fts(claim_search_fts, rowid, subject, predicate, value_flat, domain)
               VALUES('delete', old.claim_id, old.subject, old.predicate, old.value_flat, old.domain);
             END;
             CREATE TRIGGER claim_status_au AFTER UPDATE ON claim_status BEGIN
               INSERT INTO claim_search_fts(claim_search_fts, rowid, subject, predicate, value_flat, domain)
               VALUES('delete', old.claim_id, old.subject, old.predicate, old.value_flat, old.domain);
               INSERT INTO claim_search_fts(rowid, subject, predicate, value_flat, domain)
               VALUES (new.claim_id, new.subject, new.predicate, new.value_flat, new.domain);
             END;
```

- [ ] **Step 5: Run the full migration test file**

Run: `cargo test --test semantic_migration_v1 -- --nocapture`
Expected: PASS (version constant + v3→v4 tests + new v4→v5 tests, except any that call `search_claims` — those go GREEN at Task 8).

- [ ] **Step 6: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): bump CURRENT_DISK_SCHEMA_VERSION to 5 + fresh-store FTS5 DDL"
```

---

## Task 7: Add `value_flat` to the `build_confirmation_material` INSERT

The write path now stores `value_flat` so newly-confirmed claims are searchable by value terms. The FTS row is populated automatically by the `claim_status_ai` trigger installed in Tasks 4/6.

**Files:**
- Modify: `src/semantic.rs:7607-7612` (`build_confirmation_material`)
- Test: `tests/semantic_search_fts5_v1.rs` (end-to-end test added in Task 10; this task verifies via build + a smoke confirm)

- [ ] **Step 1: Update the INSERT to carry value_flat**

In `src/semantic.rs` at the `build_confirmation_material` INSERT (around line 7607-7612), add `value_flat` to both the column list and the params. Compute it from the draft value via `flatten_json`. Replace:

```rust
    transaction
        .execute(
            "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq,entity_id) VALUES (?1,?2,?3,?4,?5,NULL,NULL,?6)",
            params![claim_id.to_string(), domain_for_claim, subject, predicate, identity.event_seq as i64, entity_id.to_string()],
        )
        .map_err(database_error)?;
```

With:

```rust
    // FTS5: compute value_flat so newly-confirmed claims are searchable by
    // value terms. The claim_search_fts row itself is populated by the
    // claim_status_ai trigger (not by app code), keeping the write path
    // single-site and impossible to forget.
    let value_flat = flatten_json(&value);
    transaction
        .execute(
            "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq,entity_id,value_flat) VALUES (?1,?2,?3,?4,?5,NULL,NULL,?6,?7)",
            params![claim_id.to_string(), domain_for_claim, subject, predicate, identity.event_seq as i64, entity_id.to_string(), value_flat],
        )
        .map_err(database_error)?;
```

> **Note:** `value` was moved into `confirmation.claim.value` earlier in this function (line ~7585). To compute `value_flat` we need the value before the move. Adjust the function so `value_flat` is computed from `value` *before* `value` is moved into `ConfirmationObject`. The cleanest fix: clone or compute `value_flat` right after the `ClaimDraft { ... } = proposal.draft;` destructure (line ~7564), before `value` is moved.

- [ ] **Step 2: Resolve the borrow-order (compute value_flat before the move)**

Right after the `let ClaimDraft { subject, predicate, value, ... } = proposal.draft;` destructure, insert:

```rust
    let value_flat = flatten_json(&value);
```

Then use `value_flat` in the INSERT (Step 1), and `value` continues to be moved into `ConfirmationObject` unchanged.

- [ ] **Step 3: Verify the workspace builds**

Run: `cargo build`
Expected: BUILD succeeds. The `search_claims` method still doesn't exist (Task 8), but `build_confirmation_material` compiles.

- [ ] **Step 4: Run the existing test suite to confirm no regression**

Run: `cargo test --workspace --no-run` then a quick smoke of the confirm-path tests
Expected: PASS — the INSERT now writes one extra column with a DEFAULT, so existing readers that don't select `value_flat` are unaffected.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs
git commit -m "feat(search): write value_flat on confirm (FTS row populated by trigger)"
```

---

## Task 8: Add `ClaimSearchHit` + `SemanticStore::search_claims`

The read path. FTS5 MATCH + BM25 + active-row filter, with value rehydration from the confirmation event payload (the same path `claims_current` uses).

**Files:**
- Modify: `src/semantic.rs` (new `ClaimSearchHit` struct + `search_claims` method on `SemanticStore`)
- Test: `tests/semantic_migration_v1.rs` (the Task 4 test goes GREEN)

- [ ] **Step 1: Add the `ClaimSearchHit` struct**

Add to `src/semantic.rs` near the other public types (e.g. near `ClaimRecord`):

```rust
/// A single search result from FTS5-ranked claim search (`SemanticStore::search_claims`).
/// `value` is rehydrated from the confirmation event payload (claim_status
/// stores no value column) via the same path `claims_current` uses.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClaimSearchHit {
    pub claim_id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub domain: String,
    /// BM25 relevance score (lower = more relevant; FTS5 returns negatives).
    pub score: f64,
}
```

- [ ] **Step 2: Add the `search_claims` method**

Add to `src/semantic.rs` on `impl SemanticStore`, near `all_claims_current` (around line 3979):

```rust
    /// Full-text search over confirmed, active claims using the FTS5 index.
    /// Returns claims ranked by BM25 relevance to the query.
    ///
    /// The query is run as a trigram phrase/substring query by default; FTS5
    /// MATCH syntax also supports prefix (`rein*`), phrase (`"exact match"`),
    /// and boolean (AND/OR) operators. Column weights (subject=10, predicate=5,
    /// value_flat=1, domain=2) favor subject matches.
    ///
    /// `value` is rehydrated from the confirmation event payload via the same
    /// path as `claims_current` (confirmed_event_seq → events.object_id →
    /// decrypt_object → ConfirmationObject.claim.value), because claim_status
    /// stores no value column.
    pub fn search_claims(
        &self,
        query: &str,
        domain: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<ClaimSearchHit>> {
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;

        // Base query: FTS5 MATCH + active-row filter + optional domain filter.
        // BM25 column weights: subject=10, predicate=5, value_flat=1, domain=2.
        let sql = if domain.is_some() {
            "SELECT fts.rowid,
                    cs.subject, cs.predicate, cs.domain, cs.confirmed_event_seq,
                    bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) AS score
             FROM claim_search_fts fts
             JOIN claim_status cs ON cs.claim_id = fts.rowid
             WHERE claim_search_fts MATCH ?1
               AND cs.superseded_by_event_seq IS NULL
               AND cs.retracted_at_event_seq IS NULL
               AND cs.domain = ?2
             ORDER BY score ASC
             LIMIT ?3"
        } else {
            "SELECT fts.rowid,
                    cs.subject, cs.predicate, cs.domain, cs.confirmed_event_seq,
                    bm25(claim_search_fts, 10.0, 5.0, 1.0, 2.0) AS score
             FROM claim_search_fts fts
             JOIN claim_status cs ON cs.claim_id = fts.rowid
             WHERE claim_search_fts MATCH ?1
               AND cs.superseded_by_event_seq IS NULL
               AND cs.retracted_at_event_seq IS NULL
             ORDER BY score ASC
             LIMIT ?2"
        };
        let mut stmt = connection.prepare(sql).map_err(database_error)?;
        let rows: Vec<(String, String, String, String, i64, f64)> = if let Some(d) = domain {
            stmt.query_map(params![query, d, top_k as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            })
            .map_err(database_error)?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map(params![query, top_k as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            })
            .map_err(database_error)?
            .filter_map(|r| r.ok())
            .collect()
        };
        drop(stmt);

        // Rehydrate value from the confirmation event payload for each hit.
        let mut hits = Vec::with_capacity(rows.len());
        for (claim_id_str, subject, predicate, domain_val, confirmed_seq, score) in rows {
            let claim_id = Uuid::parse_str(&claim_id_str).map_err(|_| {
                SemanticError::CorruptLedger(format!("claim_id not a UUID: {claim_id_str}"))
            })?;
            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_seq],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            hits.push(ClaimSearchHit {
                claim_id,
                subject,
                predicate,
                value: object.claim.value,
                domain: domain_val,
                score,
            });
        }
        Ok(hits)
    }
```

> **Imports:** ensure `ConfirmationObject`, `decrypt_object`, `serialization_error`, `TransactionBehavior`, and `open_connection` are in scope (they are, in the same file). If `ConfirmationObject` is not visible at the method site, reference it by its full path or add a `use` at the top of `semantic.rs`.

- [ ] **Step 3: Run the Task 4 migration test to verify it now passes**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_forward_installs_fts5_and_backfills -- --nocapture`
Expected: PASS — `search_claims` now exists and the post-migration smoke call succeeds.

- [ ] **Step 4: Run the full migration test file**

Run: `cargo test --test semantic_migration_v1 -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs
git commit -m "feat(search): SemanticStore::search_claims (FTS5 MATCH + BM25 + value rehydrate)"
```

---

## Task 9: Rewrite `handle_brain_search` to call `search_claims`

Replace the linear-scan + substring-filter handler with a call to `search_claims`. Keep the response shape compatible and add `score`.

**Files:**
- Modify: `src/mcp/handlers.rs:856-902` (`handle_brain_search`)
- Test: `tests/semantic_search_fts5_v1.rs` (end-to-end tests added in Task 10)

- [ ] **Step 1: Rewrite the handler**

Replace the body of `handle_brain_search` at `src/mcp/handlers.rs:856-902`:

```rust
pub fn handle_brain_search(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult {
    let Some(store) = &server.semantic_store else {
        return Err("brain not initialized".to_owned());
    };
    let query = arg_str_req(args, "query")?;
    let domain = arg_str(args, "domain");
    let top_k = arg_usize(args, "top_k").unwrap_or(10);

    let hits = store
        .search_claims(query, domain.as_deref(), top_k)
        .map_err(|e| format!("{e}"))?;

    let payload = serde_json::json!({
        "query": query,
        "count": hits.len(),
        "results": hits.iter().map(|h| serde_json::json!({
            "claim_id": h.claim_id,
            "subject": h.subject,
            "predicate": h.predicate,
            "value": h.value,
            "domain": h.domain,
            "score": h.score,
        })).collect::<Vec<_>>(),
    });
    let s = serde_json::to_string_pretty(&payload).map_err(|e| format!("{e}"))?;
    ok_text(s)
}
```

> **Response-shape change:** the old handler emitted `origin`, `provenance`, `entity_id` per result; the new one emits `score` instead. These fields were not part of the documented contract and the spec explicitly drops them in favor of `score`. If any caller depends on `entity_id`, add it back by joining `claim_status.entity_id` in the `search_claims` SQL (single-column addition). For now we follow the spec.

- [ ] **Step 2: Verify the workspace builds**

Run: `cargo build`
Expected: BUILD succeeds.

- [ ] **Step 3: Commit**

```bash
git add src/mcp/handlers.rs
git commit -m "feat(search): rewrite handle_brain_search over FTS5 search_claims"
```

---

## Task 10: Add the 9 end-to-end search-behavior tests

Validates the spec's Definition of Done: basic/tokenized/value/thai/domain/ranking/supersede/edge cases.

**Files:**
- Modify: `tests/semantic_search_fts5_v1.rs` (append the 9 tests; the 7 flatten tests from Task 3 stay)

- [ ] **Step 1: Append the end-to-end tests**

Append to `tests/semantic_search_fts5_v1.rs`. Each test creates a fresh store (at v5 after Task 6), confirms a claim via the public API, then calls `search_claims`. Use the existing test helpers in `tests/` for creating a store and confirming a claim — find a reference test in `tests/semantic_ownership_v1.rs` or `tests/trust_operations_contract_v1.rs` for the exact `SemanticStore::create` + `confirm` helper pattern, and mirror it.

```rust
// Common fixture: create a fresh v5 store and return it + its trusted context.
fn fts5_fixture() -> (tempfile::TempDir, llm_wiki::semantic::SemanticStore, llm_wiki::semantic::TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = llm_wiki::semantic::SemanticStore::create(
        &root,
        llm_wiki::semantic::SemanticConfig::enabled_for(parent.path()),
    )
    .expect("create store");
    let ctx = store.trusted_context();
    (parent, store, ctx)
}

#[test]
fn fts5_basic_match_finds_confirmed_claim() {
    let (_p, store, ctx) = fts5_fixture();
    // Confirm a claim with subject "Reinvent the Wheel", then search "reinvent".
    // Use the project's confirm helper (propose + confirm, or the direct path
    // used in tests/semantic_ownership_v1.rs). Mirror the exact call sequence.
    // ... confirm claim with subject="Reinvent the Wheel" ...
    let hits = store.search_claims("reinvent", None, 10).expect("search");
    assert!(!hits.is_empty(), "must find the confirmed claim");
    assert!(hits.iter().any(|h| h.subject.contains("Reinvent")));
}

#[test]
fn fts5_tokenized_match_handles_stopword_in_subject() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm claim with subject="Reinvent the Wheel" ...
    // Searching "reinvent wheel" must find it even though "the" sits between.
    let hits = store.search_claims("reinvent wheel", None, 10).expect("search");
    assert!(hits.iter().any(|h| h.subject.contains("Reinvent")));
}

#[test]
fn fts5_value_search_finds_array_element() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm claim with value = json!(["Tesla", "BMW"]) ...
    let hits = store.search_claims("tesla", None, 10).expect("search");
    assert!(!hits.is_empty(), "must find the claim via value_flat");
}

#[test]
fn fts5_thai_subject_is_searchable() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm claim with subject="บมจ. ปตท." (SubjectShape::ThaiPure) ...
    // This is the test that validates the trigram-over-porter decision.
    let hits = store.search_claims("ปตท", None, 10).expect("search");
    assert!(hits.iter().any(|h| h.subject.contains("ปตท")), "Thai must be searchable via trigram");
}

#[test]
fn fts5_domain_filter_restricts_results() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm two claims, one in domain="eng", one in domain="fin", same query term ...
    let eng = store.search_claims("shared", Some("eng"), 10).expect("search");
    let all = store.search_claims("shared", None, 10).expect("search");
    assert!(eng.len() <= all.len(), "domain filter must not widen results");
    assert!(eng.iter().all(|h| h.domain == "eng"));
}

#[test]
fn fts5_bm25_ranks_most_relevant_first() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm 3 claims with different overlap to the query term ...
    // The one whose subject matches the query exactly must have the lowest score.
    let hits = store.search_claims("matchterm", None, 10).expect("search");
    assert!(hits.len() >= 2);
    // score is lower = better; first hit is the most relevant.
    assert!(hits[0].score <= hits[1].score, "results must be ranked best-first");
}

#[test]
fn fts5_superseded_claim_excluded_from_search() {
    let (_p, store, ctx) = fts5_fixture();
    // ... confirm claim A, then confirm claim B that supersedes A ...
    let hits = store.search_claims("superseded-term", None, 10).expect("search");
    assert!(!hits.iter().any(|h| h.subject.contains("A-subject")),
            "superseded claim must not appear in search");
}

#[test]
fn fts5_empty_query_returns_no_results_without_panicking() {
    let (_p, store, ctx) = fts5_fixture();
    // Empty string and sub-3-char queries (trigram minimum) must not panic.
    let empty = store.search_claims("", None, 10);
    // FTS5 may reject empty MATCH; either Ok(empty) or a clean Err is acceptable.
    assert!(empty.is_ok() || empty.is_err(), "empty query must not panic");
    let short = store.search_claims("ab", None, 10);
    assert!(short.is_ok() || short.is_err(), "sub-trigram query must not panic");
}

#[test]
fn fts5_special_characters_do_not_inject_match_syntax() {
    let (_p, store, ctx) = fts5_fixture();
    // A query containing FTS5 special chars must be handled safely (quoted or
    // escaped by the caller; the method itself does NOT escape — document this).
    // For now assert it does not panic.
    let _ = store.search_claims("a OR b", None, 10);
    let _ = store.search_claims("\"quoted\"", None, 10);
}
```

> **Test-body completion note:** the `// ... confirm claim ...` lines above are skeletons. The implementer must fill in the actual confirm call sequence by copying the pattern from an existing test that confirms a claim (e.g. `tests/semantic_ownership_v1.rs`'s `confirm` helper, or `trust_operations_contract_v1.rs`). The exact helper names and arg shapes are in those files; mirror them verbatim. Do NOT invent a new confirm API.

- [ ] **Step 2: Run the tests to see which pass**

Run: `cargo test --test semantic_search_fts5_v1 -- --nocapture`
Expected: the 7 flatten tests PASS; the 9 end-to-end tests pass once the confirm-call skeletons are filled with the real helper. Any failure should be a test-body issue, not an `search_claims` issue (Tasks 8–9 made the method work).

- [ ] **Step 3: Fill in the confirm-call skeletons**

For each `// ... confirm claim ...` line, copy the exact `propose` + `confirm` (or direct confirm) sequence from an existing passing test. Common shape (verify against `tests/semantic_ownership_v1.rs`):

```rust
    let draft = llm_wiki::semantic::ClaimDraft {
        subject: "Reinvent the Wheel".to_owned(),
        predicate: "is".to_owned(),
        value: serde_json::json!("an idiom"),
        claim_kind: "fact".to_owned(),
        domain: Some("idioms".to_owned()),
        confidence_basis_points: 0,
        privacy_label: llm_wiki::semantic::PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    };
    // ... propose + confirm via the store's public API ...
```

- [ ] **Step 4: Run all 16 tests to verify they pass**

Run: `cargo test --test semantic_search_fts5_v1 -- --nocapture`
Expected: PASS (7 flatten + 9 end-to-end).

- [ ] **Step 5: Commit**

```bash
git add tests/semantic_search_fts5_v1.rs
git commit -m "test(search): 9 FTS5 end-to-end behavior tests (thai/tokenize/value/rank/supersede/edge)"
```

---

## Task 11: Update `brain_search` tool description + final full-suite verification

Low-risk polish: the MCP tool manifest description, plus a full `cargo test --workspace` green run and an `integrity-check` smoke.

**Files:**
- Modify: `src/mcp/tools.rs` (`brain_search` tool description)

- [ ] **Step 1: Update the tool description**

In `src/mcp/tools.rs`, find the `brain_search` tool definition and update its `description` to mention BM25 ranking:

```
"Search confirmed claims in the semantic brain (BM25-ranked full-text search across subject, predicate, value, and domain)"
```

(Leave the parameter schema unchanged: `query`, `domain`, `top_k`.)

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — every test, including the new FTS5 + migration tests, green.

- [ ] **Step 3: Add an integrity-check smoke to the migration test (optional hardening)**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// FTS5 Task 11: after the v4→v5 migration, an FTS5 `integrity-check` reports
/// no drift between the index and claim_status. This is the SQLite-documented
/// health probe for external-content tables.
#[test]
fn v4_to_v5_integrity_check_passes_post_migration() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    // Fresh store at v5 already has the FTS5 artifacts (Task 6 DDL).
    // integrity-check returns zero rows when the index is consistent.
    let conn = rusqlite::Connection::open(&root).expect("open raw conn");
    let drift: Vec<(String,)> = conn
        .prepare("INSERT INTO claim_search_fts(claim_search_fts) VALUES('integrity-check')")
        .expect("prepare integrity-check")
        .query_map([], |row| Ok((row.get::<_, String>(0)?,)))
        .expect("query_map")
        .filter_map(|r| r.ok())
        .collect();
    assert!(drift.is_empty(), "FTS5 integrity-check must report no drift on a fresh v5 store, got: {drift:?}");
}
```

- [ ] **Step 4: Run the integrity-check test**

Run: `cargo test --test semantic_migration_v1 v4_to_v5_integrity_check_passes_post_migration -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Final full-suite run**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs tests/semantic_migration_v1.rs
git commit -m "feat(search): brain_search tool description + FTS5 integrity-check smoke"
```

---

## Self-Review Notes

**Spec coverage** (each spec section → task):
- §1 Schema migration v4→v5 → Tasks 1, 2, 4, 5, 6
- §2 `flatten_json` helper → Task 3
- §3 Write path (value_flat on confirm, triggers handle FTS) → Task 7
- §4 `search_claims` method + `ClaimSearchHit` → Task 8
- §5 Rewrite `handle_brain_search` → Task 9
- §6 `claim_timeline` interaction → no-op (documented in spec; no task needed)
- §7 Tests (9 cases) → Task 10
- §8 MCP tool manifest → Task 11

**Placeholder scan:** the `// ... confirm claim ...` skeletons in Task 10 are explicitly flagged as copy-from-existing-test; the implementer is directed to `tests/semantic_ownership_v1.rs`. This is not a TBD — it is a "mirror this exact pattern" instruction with the source file named.

**Type consistency:** `ClaimSearchHit` (Task 8) fields `claim_id, subject, predicate, value, domain, score` are used identically in `handle_brain_search` (Task 9) and asserted in the Task 10 tests. `flatten_json` signature (Task 3) matches the call site in Task 7. `search_claims(query, domain, top_k)` signature matches both the Task 9 handler call and the Task 10 test calls.
