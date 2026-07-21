# Entity Identity Reform Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drop `domain` from the entity identity constraint so that one real-world subject (e.g. CATL) resolves to exactly one stable `entity_id`, regardless of how many domain variants LLM extraction emits. Backed by the Wikidata identity pattern (opaque ID + mutable categorization tags) — the same pattern used by Wikidata (100M+ items), MusicBrainz (20+ years), OpenStreetMap (17+ years), GitHub (200M+ repos).

**Architecture:** Three coordinated layers. (1) A schema bump `CURRENT_DISK_SCHEMA_VERSION: 3 → 4` carrying an atomic, reversible in-transaction migration that merges fragmented entities onto one canonical target per `canonical_subject`, then recreates the `entities` + `entity_aliases` tables without `domain` in their key. (2) Core entity-resolution functions drop the `domain` parameter; `claim_status.domain` stays as a per-claim tag. (3) Public read APIs (`claim_timeline`), HTTP endpoints (`GET /entity/timeline`), and two MCP tool handlers (`brain_capture`, `brain_propose`) demote `domain` from required to optional.

**Tech Stack:** Rust 1.95 / edition 2024, rusqlite 0.40.1 (bundled SQLite ≥3.35 → supports `ALTER TABLE DROP COLUMN`, but we use table-recreation for atomic constraint changes), axum (HTTP), serde (wire types), existing TDD + schema-version-gate machinery.

**Reference:** `docs/plans/entity-identity-reform-design-doc.md` (research-backed, the "why" + production case studies). ADR-0001 §Decision 6 will be amended by ADR-0002.

**Branch:** `vnext/phase-0` (already checked out; commit per task).

---

## File Structure

This is a layered change. Files are grouped by responsibility, mirroring the layered architecture.

**Schema layer (one file, atomic transaction):**
- `src/semantic.rs` — `CURRENT_DISK_SCHEMA_VERSION` bump, `schema_upgrade_path_exists` arm, `plan_schema_upgrade` step body, `run_upgrade_step_forward`/`run_upgrade_step_reverse` arms for the genuine v3→v4 DDL+data migration. Also the `CREATE TABLE entities`/`entity_aliases` DDL (used by fresh stores at v4).

**Entity-resolution helpers (same file, internal):**
- `src/semantic.rs:7285-7349` — `resolve_or_create_entity_in_tx`, `resolve_entity_in_tx`, `insert_alias` (drop `domain` param).

**Public read/write APIs (same file):**
- `src/semantic.rs:3069-3093` — `resolve_or_create_entity` (public, drop `domain`).
- `src/semantic.rs:3098-3109` — `resolve_entity` (public, drop `domain`).
- `src/semantic.rs:3150-3220` — `rename_entity` (simplify collision check).
- `src/semantic.rs:3227-3326` — `merge_entities` (delete cross-domain guard).
- `src/semantic.rs:5669-5758` — `split_entities` (delete cross-domain guard).
- `src/semantic.rs:3112-3143` — `entity_by_id` (drop `EntityRecord.domain` recovery query).
- `src/semantic.rs:4096-4153` — `claim_timeline` (`domain` becomes optional filter).

**Types (same file):**
- `src/semantic.rs:309-319` — `ClaimDraft` (`domain: Option<String>`).
- `src/semantic.rs:526-531` — `EntityRecord` (drop `domain` field).
- `src/semantic.rs:671-683` — `ProposalSummary` (`domain: Option<String>`).
- `src/semantic.rs:7135-7145` — `build_confirmation_material` destructure + `claim_status` insert.

**Wire/HTTP layer:**
- `src/api.rs:827-844` — `TimelineParams` (`domain: Option<String>`), `timeline` handler.
- `src/mcp/handlers.rs:1199-1321` — `handle_brain_capture`, `handle_brain_propose` (`domain` optional).
- `src/mcp/tools.rs:574-648` — `brain_capture` + `brain_propose` tool manifests (`domain` moves out of required array).

**Conflict-detection consequence (downstream, leaves domain-as-tag intact):**
- `src/inbox_conflicts.rs:62-162` — bucket key changes from `(domain, subject, predicate)` to `(subject, predicate)` so cross-domain duplicates/conflicts surface (the design doc's "Phase 1.6 conflict detection works across domains" outcome).

**Tests touched (mechanical signature updates + new tests):**
- All 22 files under `tests/` that construct `ClaimDraft { domain: "..." }` — change to `domain: Some("...".to_owned())`.
- `tests/semantic_ownership_v1.rs` — `resolve_or_create_entity`/`resolve_entity` call sites drop `domain`; `merge_entities_moves_claims_and_keeps_aliases_as_backlinks` test relaxes cross-domain assertion.
- `tests/trust_operations_contract_v1.rs` — `split_entities_rejects_cross_domain` becomes `split_entities_allows_cross_domain`.
- `tests/semantic_migration_v1.rs` — v3→v4 migration test.
- New file `tests/semantic_identity_reform_v1.rs` — Wikidata-pattern regression tests (identity, cross-domain merge, migration round-trip).

**Documentation:**
- `docs/adr/0002-entity-identity-reform-wikidata-pattern.md` — new ADR documenting the amendment to ADR-0001 §Decision 6.
- `BLUEPRINT.md` — note the reform under Phase 0.

---

## Task Decomposition Rationale

The plan is sequenced so each task produces a **compilable, test-green** state with frequent commits. We do NOT do "all schema, then all code" — that produces a huge uncompilable middle. Instead:

- **Tasks 1–4** add the v3→v4 schema-upgrade machinery *as a reversible feature* behind a constant bump that is not yet wired into `create()`. This is the Wikidata pattern's data layer — the riskiest change — built TDD with rollback rehearsed.
- **Tasks 5–7** flip the schema version gate, dropping the column from fresh-store DDL and running the migration on existing stores.
- **Tasks 8–11** update the core entity-resolution helpers (internal + public), one function at a time, compiler-guided.
- **Tasks 12–14** remove the cross-domain guards from `merge_entities` and `split_entities` — the architectural change that the migration makes safe.
- **Tasks 15–17** update types (`ClaimDraft`, `EntityRecord`, `ProposalSummary`) and their construction sites.
- **Tasks 18–20** update wire surfaces: HTTP `claim_timeline`, MCP tool handlers, MCP tool manifests.
- **Task 21** updates the conflict-detection bucket key.
- **Tasks 22–23** write ADR-0002, update BLUEPRINT, produce the shipped report.

Each task ends with `cargo build` + targeted test green + commit. The plan is designed so that if execution halts at any task boundary, the repo is in a working state.

---

## Task 1: Add v3→v4 upgrade-path predicate (RED)

The schema-upgrade machinery is gated on `schema_upgrade_path_exists(from, to)`. We extend the match first — TDD style: a failing test proves the new path is not yet wired, then we wire it.

**Files:**
- Modify: `src/semantic.rs:6652-6655` (`schema_upgrade_path_exists`)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 1: the v3→v4 upgrade path is recognized as a known
/// migration route. This is the precondition for every subsequent migration
/// step — `plan_schema_upgrade(3, 4)` must succeed. Today it fails with
/// "unsupported schema upgrade path" because only (2,3) is in the match.
#[test]
fn v3_to_v4_upgrade_path_is_known() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    // A fresh store is at CURRENT_DISK_SCHEMA_VERSION (will be 4 after Task 5,
    // but the predicate must accept the v3→v4 step regardless of the current
    // constant). The plan call must succeed, not return the "unsupported path"
    // error. We use from=3 explicitly to lock the predicate arm.
    let plan = store.plan_schema_upgrade(3, 4);
    assert!(
        plan.is_ok(),
        "v3→v4 upgrade path must be recognized, got: {plan:?}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_upgrade_path_is_known -- --nocapture`
Expected: FAIL with `"unsupported schema upgrade path: 3 → 4 (only 2 → 3 is implemented)"` (the message inside `plan_schema_upgrade`).

- [ ] **Step 3: Add the v3→v4 arm to `schema_upgrade_path_exists`**

Replace `src/semantic.rs:6652-6655`:

```rust
fn schema_upgrade_path_exists(from: u8, to: u8) -> bool {
    matches!((from, to), (2, 3) | (3, 4))
}
```

Also update the error message inside `plan_schema_upgrade` (`src/semantic.rs:2140-2143`) to no longer claim only 2→3:

```rust
        if !schema_upgrade_path_exists(from, to) {
            return Err(SemanticError::CorruptLedger(format!(
                "unsupported schema upgrade path: {from} → {to} (supported paths: 2→3, 3→4)"
            )));
        }
```

- [ ] **Step 4: Run the test to verify it still fails (now at the `from != self.marker.schema_version` sanity check)**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_upgrade_path_is_known -- --nocapture`
Expected: FAIL — but now with a DIFFERENT error: `"plan from=3 does not match live marker schema_version=3"` would pass for a v3 store... but a freshly-created store is at v3 *today* (before Task 5 bumps the constant). So this test should now PASS for the current constant value of 3. Confirm it passes:

Run: `cargo test --test semantic_migration_v1 v3_to_v4_upgrade_path_is_known -- --nocapture`
Expected: PASS (because the fresh store's marker is at v3 = CURRENT_DISK_SCHEMA_VERSION today, and from=3 matches).

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): recognize v3→v4 upgrade path (RED→GREEN for Task 1)"
```

---

## Task 2: Define the v3→v4 plan step body (reversible migration description)

`plan_schema_upgrade` currently returns a single noop step. The v3→v4 plan needs a step whose description names what it does (entity consolidation + constraint change) and is marked reversible (the constraint changes can be undone via another table-recreation).

**Files:**
- Modify: `src/semantic.rs:2135-2165` (`plan_schema_upgrade` step construction)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 2: planning a v3→v4 upgrade yields a reversible plan
/// whose step description names the entity consolidation. The plan is the
/// audit-trail contract an operator reads before `execute_schema_upgrade`;
/// it must advertise reversibility or `execute_schema_upgrade` will refuse it.
#[test]
fn v3_to_v4_plan_is_reversible_and_names_consolidation() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");
    let plan = store.plan_schema_upgrade(3, 4).expect("plan v3→v4");
    assert!(plan.is_reversible(), "v3→v4 plan must be reversible");
    assert!(
        plan.steps.iter().any(|s| s.description.contains("entity")
            && s.description.contains("consolidat")),
        "plan must describe the entity consolidation; got steps: {:?}",
        plan.steps
    );
    assert_eq!(plan.from_version, 3);
    assert_eq!(plan.to_version, 4);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_plan_is_reversible_and_names_consolidation -- --nocapture`
Expected: FAIL — the current `plan_schema_upgrade` returns a noop step whose description is `"noop placeholder migration to prove upgrade path"`, which does not contain "entity" or "consolidat".

- [ ] **Step 3: Make `plan_schema_upgrade` version-aware**

Replace `src/semantic.rs:2157-2164`:

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
                    ).to_owned(),
                    reversible: true,
                }],
                _ => unreachable!("schema_upgrade_path_exists gates this match"),
            },
            from_version: from,
            to_version: to,
        })
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_plan_is_reversible_and_names_consolidation -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): v3→v4 plan step names entity consolidation (reversible)"
```

---

## Task 3: Forward migration body — entity consolidation (RED)

The actual data migration runs inside `run_upgrade_step_forward` gated on the `(from, to, forward_index)` triple. This task adds the consolidation half: rewrite every claim from a duplicate entity_id onto the canonical target per `canonical_subject`, fold aliases onto the target, and delete the duplicates — all inside the upgrade transaction. The constraint change (Task 4) runs in the same transaction right after.

We write this against a fixture that creates two entities sharing a `canonical_subject` (a deliberately constructed legacy state) and asserts they collapse to one.

**Files:**
- Modify: `src/semantic.rs:6669-6703` (`run_upgrade_step_forward` — add a `(3, 4, 0)` arm)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 3: forward v3→v4 migration collapses two entities that
/// share a canonical_subject onto one (most-claims-wins target). We construct
/// the legacy fragmented state by hand (insert two rows with the same
/// canonical_subject but different domains), run execute_schema_upgrade, and
/// assert only one entity survives with all claims attached.
#[test]
fn v3_to_v4_forward_consolidates_fragmented_entities() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    use rusqlite::Connection;

    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create at v3");

    // Construct legacy fragmentation directly: two entities with the same
    // canonical_subject but different domains (the pre-reform invariant).
    // We use a second connection to the same SQLite file. The store's own
    // connection must be dropped first to avoid locking.
    drop(store);
    let db_path = root.join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open db");
    conn.execute(
        "INSERT INTO entities(entity_id, domain, canonical_subject, created_at) \
         VALUES ('aaaaaaaa-0000-7000-8000-000000000001', 'business', 'CATL', '2026-07-01T00:00:00Z')",
        [],
    ).expect("insert dup entity 1");
    conn.execute(
        "INSERT INTO entities(entity_id, domain, canonical_subject, created_at) \
         VALUES ('aaaaaaaa-0000-7000-8000-000000000002', 'financial', 'CATL', '2026-07-01T00:00:00Z')",
        [],
    ).expect("insert dup entity 2");
    conn.execute(
        "INSERT INTO claim_status(claim_id, domain, subject, predicate, confirmed_event_seq, \
         superseded_by_event_seq, retracted_at_event_seq, entity_id) \
         VALUES ('c0000000-0000-7000-8000-0000000000a1', 'business', 'CATL', 'p', 1, NULL, NULL, \
         'aaaaaaaa-0000-7000-8000-000000000001')",
        [],
    ).expect("insert claim on entity 1");
    conn.execute(
        "INSERT INTO claim_status(claim_id, domain, subject, predicate, confirmed_event_seq, \
         superseded_by_event_seq, retracted_at_event_seq, entity_id) \
         VALUES ('c0000000-0000-7000-8000-0000000000a2', 'financial', 'CATL', 'p', 1, NULL, NULL, \
         'aaaaaaaa-0000-7000-8000-000000000002')",
        [],
    ).expect("insert claim on entity 2");
    conn.execute(
        "INSERT INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
         VALUES ('business', 'CATL', 'aaaaaaaa-0000-7000-8000-000000000001', 'canonical', 0)",
        [],
    ).expect("insert alias 1");
    conn.execute(
        "INSERT INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
         VALUES ('financial', 'CATL', 'aaaaaaaa-0000-7000-8000-000000000002', 'canonical', 0)",
        [],
    ).expect("insert alias 2");
    drop(conn);

    // Reopen for upgrade and run it.
    let store = SemanticStore::open_for_upgrade(&root, SemanticConfig::enabled_for(parent.path()))
        .expect("open for upgrade");
    let plan = store.plan_schema_upgrade(3, 4).expect("plan");
    store.execute_schema_upgrade(&plan).expect("execute upgrade");

    // Assert consolidation: exactly one CATL entity remains.
    let conn = Connection::open(&db_path).expect("reopen");
    let entity_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE canonical_subject='CATL'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(entity_count, 1, "CATL must consolidate to one entity");

    // Both claims now attach to the surviving entity.
    let surviving: String = conn
        .query_row(
            "SELECT entity_id FROM entities WHERE canonical_subject='CATL'",
            [],
            |row| row.get(0),
        )
        .expect("surviving entity");
    let claim_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM claim_status WHERE entity_id=?1",
            [&surviving],
            |row| row.get(0),
        )
        .expect("count claims");
    assert_eq!(claim_count, 2, "both claims must attach to the survivor");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_forward_consolidates_fragmented_entities -- --nocapture`
Expected: FAIL — the test asserts `entity_count == 1` but the current noop forward step leaves 2 entities, so the assertion fires.

- [ ] **Step 3: Add the forward migration body**

In `src/semantic.rs`, extend `run_upgrade_step_forward` (currently at line 6669). Add a branch BEFORE the audit-row write that runs the genuine migration when `(from_version, to_version) == (3, 4)`:

```rust
fn run_upgrade_step_forward(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    step: &crate::recovery::UpgradeStep,
    now: DateTime<Utc>,
) -> Result<()> {
    // Phase Reform Task 3/4: the genuine v3→v4 migration. Runs entirely
    // inside this transaction — any failure rolls back via `?`.
    if (from_version, to_version) == (3, 4) && forward_index == 0 {
        run_entity_identity_reform_forward(transaction)?;
    }

    // Audit-trail row: records which step ran, against which version pair,
    // and whether the step advertises itself reversible. The value is the
    // step description (operator-readable in `SELECT key,value FROM meta`).
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

/// Entity Identity Reform forward migration (v3 → v4). Executes the four
/// consolidation steps from the design doc §7.1, all inside the caller's
/// transaction:
///
///   1. Pick a canonical target per `canonical_subject` (most active claims wins;
///      ties broken by lexicographically smallest entity_id for determinism).
///   2. Rewrite every `claim_status.entity_id` from a losing entity onto its
///      target.
///   3. Fold losing entities' aliases onto their target as `former_subject`.
///   4. Delete the losing entities.
///
/// The constraint change (dropping `domain` from the entities UNIQUE and the
/// entity_aliases PK) is performed by `run_entity_identity_reform_constraints`
/// in Task 4, immediately after this returns, inside the same transaction.
fn run_entity_identity_reform_forward(transaction: &Transaction) -> Result<()> {
    use rusqlite::params_from_iter;

    // Step 1: pick the canonical target per canonical_subject.
    //
    // We cannot use a single SQL window function with a correlated claim
    // count cleanly across SQLite versions bundled by rusqlite without
    // surprises, so we do the ranking in two passes: load the candidates,
    // pick in Rust (deterministic), then apply.
    let mut stmt = transaction
        .prepare(
            "SELECT entity_id, canonical_subject FROM entities ORDER BY canonical_subject ASC, entity_id ASC",
        )
        .map_err(database_error)?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    drop(stmt);

    // Active-claim count per entity (superseded_by/retracted claims do not
    // count toward "most claims wins" — only the live ones do).
    let mut claim_counts: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    {
        let mut stmt = transaction
            .prepare(
                "SELECT entity_id, COUNT(*) FROM claim_status \
                 WHERE superseded_by_event_seq IS NULL AND retracted_at_event_seq IS NULL \
                 GROUP BY entity_id",
            )
            .map_err(database_error)?;
        let counts = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(database_error)?;
        for c in counts {
            let (eid, n) = c.map_err(database_error)?;
            claim_counts.insert(eid, n);
        }
    }

    // Group by canonical_subject; pick target = most claims, ties → smallest entity_id.
    // Rows are already sorted (subject ASC, entity_id ASC) so the first row of a
    // subject group is the tie-break winner; we just need to pick the max-claim row.
    let mut groups: std::collections::HashMap<String, Vec<(String, i64)>> =
        std::collections::HashMap::new();
    for (eid, subject) in &rows {
        groups
            .entry(subject.clone())
            .or_default()
            .push((eid.clone(), *claim_counts.get(eid).unwrap_or(&0)));
    }
    let mut targets: Vec<(String, String)> = Vec::new(); // (canonical_subject, target_entity_id)
    let mut losers: Vec<String> = Vec::new(); // entity_ids to delete
    for (subject, mut members) in groups {
        // Sort: most claims first, then smallest entity_id (already the row order
        // before this sort). stable_sort keeps the entity_id tiebreak.
        members.sort_by(|a, b| b.1.cmp(&a.1));
        let target = members[0].0.clone();
        targets.push((subject, target.clone()));
        for (eid, _) in &members[1..] {
            losers.push(eid.clone());
        }
    }

    if losers.is_empty() {
        return Ok(()); // nothing to consolidate; constraints step still runs.
    }

    // Step 2: rewrite claim_status.entity_id from each loser onto its target.
    // We look up the target per loser via a per-subject map.
    let subject_of: std::collections::HashMap<String, String> = rows
        .iter()
        .cloned()
        .collect(); // entity_id → canonical_subject
    let target_of_subject: std::collections::HashMap<&str, &str> = targets
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();
    for loser in &losers {
        let subject = subject_of.get(loser).expect("loser subject");
        let target = target_of_subject
            .get(subject.as_str())
            .copied()
            .expect("target for subject");
        transaction
            .execute(
                "UPDATE claim_status SET entity_id=?1 WHERE entity_id=?2",
                params![target, loser],
            )
            .map_err(database_error)?;
    }

    // Step 3: fold aliases from losers onto their targets as former_subject.
    // (Each losing alias becomes a former_subject alias on the target so every
    // historical reference keeps resolving post-migration.)
    for loser in &losers {
        let subject = subject_of.get(loser).expect("loser subject");
        let target = target_of_subject
            .get(subject.as_str())
            .copied()
            .expect("target");
        // Pull every alias the loser currently holds.
        let mut stmt = transaction
            .prepare("SELECT alias FROM entity_aliases WHERE entity_id=?1")
            .map_err(database_error)?;
        let aliases: Vec<String> = stmt
            .query_map([loser], |row| row.get::<_, String>(0))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        drop(stmt);
        for alias in aliases {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
                     VALUES (?1, ?2, ?3, 'former_subject', 0)",
                    params![subject, alias, target],
                )
                .map_err(database_error)?;
        }
    }

    // Step 4: delete the losing entities. claim_status rows have already been
    // rewritten away; entity_aliases rows pointing at losers are harmless
    // historical residue but we delete them to keep the table tidy and match
    // the design-doc invariant (no alias references a deleted entity).
    for loser in &losers {
        transaction
            .execute(
                "DELETE FROM entity_aliases WHERE entity_id=?1",
                [loser],
            )
            .map_err(database_error)?;
        transaction
            .execute("DELETE FROM entities WHERE entity_id=?1", [loser])
            .map_err(database_error)?;
    }

    // Silence unused-import warning if params_from_iter ends up unused.
    let _ = params_from_iter::<std::vec::IntoIter<rusqlite::types::Value>>;
    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_forward_consolidates_fragmented_entities -- --nocapture`
Expected: PASS — exactly 1 CATL entity remains, both claims attach to the survivor.

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): v3→v4 forward consolidates fragmented entities"
```

---

## Task 4: Forward migration body — constraint change (RED)

After consolidation, the entities that share a `canonical_subject` have collapsed to one. Now the UNIQUE constraint on `(domain, canonical_subject)` becomes a uniqueness violation source only if a single entity somehow has two domains — which the consolidation prevents. We recreate both tables without `domain` in the key, inside the same transaction.

**Files:**
- Modify: `src/semantic.rs` (extend `run_entity_identity_reform_forward` with the constraint step, called after consolidation)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 4: after the v3→v4 migration, the entities table no
/// longer has a `domain` column (it was dropped along with the composite
/// UNIQUE constraint). The new UNIQUE is on canonical_subject alone, and
/// entity_aliases PK no longer includes domain. We assert by attempting an
/// INSERT that would have been illegal under the old composite UNIQUE but is
/// required under the new one: two aliases with different (former) domains
/// pointing at the same entity.
#[test]
fn v3_to_v4_forward_drops_domain_from_entity_key() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    use rusqlite::Connection;

    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create at v3");
    drop(store);

    let db_path = root.join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open db");
    // One entity, one canonical_subject, one claim — no fragmentation to
    // consolidate, but we still want the constraint change to run.
    conn.execute(
        "INSERT INTO entities(entity_id, domain, canonical_subject, created_at) \
         VALUES ('bbbbbbbb-0000-7000-8000-000000000001', 'stocks', 'GULF', '2026-07-01T00:00:00Z')",
        [],
    ).expect("insert entity");
    conn.execute(
        "INSERT INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
         VALUES ('stocks', 'GULF', 'bbbbbbbb-0000-7000-8000-000000000001', 'canonical', 0)",
        [],
    ).expect("insert alias");
    drop(conn);

    let store = SemanticStore::open_for_upgrade(&root, SemanticConfig::enabled_for(parent.path()))
        .expect("open for upgrade");
    let plan = store.plan_schema_upgrade(3, 4).expect("plan");
    store.execute_schema_upgrade(&plan).expect("execute upgrade");

    let conn = Connection::open(&db_path).expect("reopen");

    // The entities table no longer has a `domain` column.
    let has_domain_column: bool = conn
        .prepare("PRAGMA table_info(entities)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .any(|col: String| col == "domain");
    assert!(
        !has_domain_column,
        "entities.domain must be dropped after v3→v4"
    );

    // canonical_subject is now UNIQUE on its own. The new schema is enforced
    // by the CREATE TABLE in initialize_schema for fresh stores at v4 (Task 6),
    // and here by the table-recreation step. We assert via an attempt to
    // insert a second entity with the same canonical_subject.
    let dup_attempt = conn.execute(
        "INSERT INTO entities(entity_id, canonical_subject, created_at) \
         VALUES ('bbbbbbbb-0000-7000-8000-000000000099', 'GULF', '2026-07-01T00:00:00Z')",
        [],
    );
    assert!(
        dup_attempt.is_err(),
        "canonical_subject must be UNIQUE after v3→v4; insert unexpectedly succeeded"
    );

    // entity_aliases PK no longer includes domain. Verify by inspecting the
    // PRAGMA index_list / table_info — the new PK should be (alias, entity_id).
    let pk_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entity_aliases)")
        .unwrap()
        .query_map([], |r| {
            let name: String = r.get(1)?;
            let pk: i64 = r.get(5)?;
            Ok(if pk > 0 { Some(name) } else { None })
        })
        .unwrap()
        .filter_map(Result::ok)
        .flatten()
        .collect();
    assert_eq!(pk_cols, vec!["alias".to_string(), "entity_id".to_string()],
        "entity_aliases PK after v3→v4 must be (alias, entity_id); got {pk_cols:?}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_forward_drops_domain_from_entity_key -- --nocapture`
Expected: FAIL — the entities table still has a `domain` column (the forward migration so far only consolidates rows; it does not yet touch the schema). Assertion on `has_domain_column` fires.

- [ ] **Step 3: Add the constraint-change step**

Extend `run_entity_identity_reform_forward` (added in Task 3) — append, before the `Ok(())`:

```rust
    // Task 4: constraint change. After consolidation, no two entities share a
    // canonical_subject, so the new UNIQUE(canonical_subject) is safe. We
    // recreate both tables (SQLite cannot drop a column from a UNIQUE
    // constraint in place) inside this transaction.
    //
    // entities: drop `domain` column; UNIQUE(canonical_subject).
    // entity_aliases: drop `domain` column from PK; new PK(alias, entity_id).
    transaction
        .execute(
            "CREATE TABLE entities_reform(\
               entity_id TEXT PRIMARY KEY,\
               canonical_subject TEXT NOT NULL,\
               created_at TEXT NOT NULL,\
               UNIQUE(canonical_subject)\
             )",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO entities_reform(entity_id, canonical_subject, created_at) \
             SELECT entity_id, canonical_subject, created_at FROM entities",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute("DROP TABLE entities", [])
        .map_err(database_error)?;
    transaction
        .execute(
            "ALTER TABLE entities_reform RENAME TO entities",
            [],
        )
        .map_err(database_error)?;

    transaction
        .execute(
            "CREATE TABLE entity_aliases_reform(\
               alias TEXT NOT NULL,\
               entity_id TEXT NOT NULL,\
               kind TEXT NOT NULL,\
               aliased_at_event_seq INTEGER NOT NULL,\
               PRIMARY KEY(alias, entity_id)\
             )",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO entity_aliases_reform(alias, entity_id, kind, aliased_at_event_seq) \
             SELECT alias, entity_id, kind, aliased_at_event_seq FROM entity_aliases",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute("DROP TABLE entity_aliases", [])
        .map_err(database_error)?;
    transaction
        .execute(
            "ALTER TABLE entity_aliases_reform RENAME TO entity_aliases",
            [],
        )
        .map_err(database_error)?;

    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_forward_drops_domain_from_entity_key -- --nocapture`
Expected: PASS.

Also re-run Task 3's test to confirm we did not regress the consolidation path:

Run: `cargo test --test semantic_migration_v1 v3_to_v4_forward -- --nocapture`
Expected: PASS (both forward tests).

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): v3→v4 forward drops domain from entity identity key"
```

---

## Task 5: Reverse migration body (rollback rehearsal)

The plan must be reversible (`is_reversible()` returns true). The reverse of v3→v4: restore the `domain` column on both tables and the old composite keys. For consolidation, the reverse is best-effort — once entities have been merged, we cannot reconstruct which domain a claim originally came from (it is preserved on `claim_status.domain` already, so no data is lost; only the *entity* rows that were deleted cannot be resurrected). The reverse therefore restores the schema shape but cannot un-merge.

This is the design-doc's documented tradeoff: rollback restores the schema, but a re-run of forward is the way to re-assert consolidation. We document this in the reverse step.

**Files:**
- Modify: `src/semantic.rs` (`run_upgrade_step_reverse` — add a `(3, 4, 0)` arm that restores the columns)
- Test: `tests/semantic_migration_v1.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 5: rollback v3→v4 restores the `domain` column on both
/// entities and entity_aliases, returning the schema to the v3 shape. We do
/// not assert that merged entities are un-merged (rollback cannot resurrect
/// deleted rows — documented); we only assert the schema shape is restored,
/// which is the reversibility contract execute_schema_upgrade enforces.
#[test]
fn v3_to_v4_rollback_restores_domain_column() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    use rusqlite::Connection;

    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create at v3");
    drop(store);

    let db_path = root.join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open db");
    conn.execute(
        "INSERT INTO entities(entity_id, domain, canonical_subject, created_at) \
         VALUES ('cccccccc-0000-7000-8000-000000000001', 'stocks', 'GULF', '2026-07-01T00:00:00Z')",
        [],
    ).expect("insert entity");
    conn.execute(
        "INSERT INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
         VALUES ('stocks', 'GULF', 'cccccccc-0000-7000-8000-000000000001', 'canonical', 0)",
        [],
    ).expect("insert alias");
    drop(conn);

    // Forward to v4, then rollback to v3.
    let store = SemanticStore::open_for_upgrade(&root, SemanticConfig::enabled_for(parent.path()))
        .expect("open");
    let plan = store.plan_schema_upgrade(3, 4).expect("plan");
    store.execute_schema_upgrade(&plan).expect("forward");
    store.rollback_schema_upgrade(&plan).expect("rollback");

    let conn = Connection::open(&db_path).expect("reopen");
    let entities_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entities)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        entities_cols.iter().any(|c| c == "domain"),
        "entities.domain must be restored after rollback; cols = {entities_cols:?}"
    );

    let aliases_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entity_aliases)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        aliases_cols.iter().any(|c| c == "domain"),
        "entity_aliases.domain must be restored after rollback; cols = {aliases_cols:?}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_rollback_restores_domain_column -- --nocapture`
Expected: FAIL — rollback currently wipes the audit row and does nothing else; the `domain` column stays dropped, so the assertion on `entities_cols` fires.

- [ ] **Step 3: Add the reverse migration body**

Extend `run_upgrade_step_reverse` (currently at `src/semantic.rs:6705`). Add a `(3, 4, 0)` arm BEFORE the audit-row wipe:

```rust
fn run_upgrade_step_reverse(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    _step: &crate::recovery::UpgradeStep,
) -> Result<()> {
    // Phase Reform Task 5: reverse the v3→v4 constraint change. Restores the
    // `domain` column on both tables and the v3 composite keys. NOTE: this
    // cannot un-merge consolidated entities — deleted rows are gone. Rollback
    // is for "the schema shape changed in a way we cannot serve under"; a
    // re-run of forward is the documented way to re-assert consolidation.
    // Domain values are best-effort: we synthesize them from claim_status
    // (each surviving entity's domain is the domain of any of its claims),
    // defaulting to "" when no claim exists (matches the pre-reform invariant
    // that an entity always had a domain).
    if (from_version, to_version) == (3, 4) && forward_index == 0 {
        run_entity_identity_reform_reverse(transaction)?;
    }

    let audit_key = format!("upgrade_step_{forward_index}_to_v{to_version}");
    transaction
        .execute("DELETE FROM meta WHERE key=?1", params![audit_key])
        .map_err(database_error)?;
    // Reference from_version to silence dead-code warnings on a future
    // genuine migration that needs the target version to undo a DDL change.
    let _ = from_version;
    Ok(())
}

/// Reverse of `run_entity_identity_reform_forward`. Restores the `domain`
/// column on `entities` (synthesized from claim_status where possible) and
/// `entity_aliases`, and the v3 composite keys. Does NOT resurrect deleted
/// (consolidated) entity rows — that is documented as irreversible.
fn run_entity_identity_reform_reverse(transaction: &Transaction) -> Result<()> {
    // entities: re-add domain (best-effort: from any claim on that entity).
    transaction
        .execute(
            "CREATE TABLE entities_rollback(\
               entity_id TEXT PRIMARY KEY,\
               domain TEXT NOT NULL DEFAULT '',\
               canonical_subject TEXT NOT NULL,\
               created_at TEXT NOT NULL,\
               UNIQUE(domain, canonical_subject)\
             )",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO entities_rollback(entity_id, domain, canonical_subject, created_at) \
             SELECT e.entity_id, \
                    COALESCE((SELECT cs.domain FROM claim_status cs \
                              WHERE cs.entity_id = e.entity_id \
                              ORDER BY cs.confirmed_event_seq ASC LIMIT 1), ''), \
                    e.canonical_subject, e.created_at \
             FROM entities e",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute("DROP TABLE entities", [])
        .map_err(database_error)?;
    transaction
        .execute(
            "ALTER TABLE entities_rollback RENAME TO entities",
            [],
        )
        .map_err(database_error)?;

    // entity_aliases: re-add domain (best-effort: from the entity it points at).
    transaction
        .execute(
            "CREATE TABLE entity_aliases_rollback(\
               domain TEXT NOT NULL DEFAULT '',\
               alias TEXT NOT NULL,\
               entity_id TEXT NOT NULL,\
               kind TEXT NOT NULL,\
               aliased_at_event_seq INTEGER NOT NULL,\
               PRIMARY KEY(domain, alias, entity_id)\
             )",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO entity_aliases_rollback(domain, alias, entity_id, kind, aliased_at_event_seq) \
             SELECT COALESCE((SELECT e.domain FROM entities e WHERE e.entity_id = ea.entity_id), ''), \
                    ea.alias, ea.entity_id, ea.kind, ea.aliased_at_event_seq \
             FROM entity_aliases ea",
            [],
        )
        .map_err(database_error)?;
    transaction
        .execute("DROP TABLE entity_aliases", [])
        .map_err(database_error)?;
    transaction
        .execute(
            "ALTER TABLE entity_aliases_rollback RENAME TO entity_aliases",
            [],
        )
        .map_err(database_error)?;
    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test semantic_migration_v1 v3_to_v4_rollback_restores_domain_column -- --nocapture`
Expected: PASS.

Also run the forward tests together to confirm we did not break them:

Run: `cargo test --test semantic_migration_v1 v3_to_v4 -- --nocapture`
Expected: PASS (all four v3→v4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/semantic.rs tests/semantic_migration_v1.rs
git commit -m "feat(schema): v3→v4 reverse restores domain column (rollback rehearsal)"
```

---

## Task 6: Bump CURRENT_DISK_SCHEMA_VERSION to 4 and update fresh-store DDL

Now that the migration is rehearsed end-to-end, we flip the version constant and update the `CREATE TABLE` DDL that `initialize_schema` emits for fresh stores. After this task, a freshly-created store is at v4 directly; an existing v3 store refuses to serve until an operator runs `llm-wiki recovery upgrade`.

**Files:**
- Modify: `src/semantic.rs:98` (`CURRENT_DISK_SCHEMA_VERSION`)
- Modify: `src/semantic.rs:6432-6454` (entities + entity_aliases DDL)
- Modify: `src/semantic.rs:6581-6604` (the schema-version-gate doc comment — note v3→v4 is now a real migration)
- Test: existing `tests/semantic_ownership_v1.rs:508-527` (the "store at schema_version N fails to open" test) — no change needed, but we add a fresh-store assertion.

- [ ] **Step 1: Write the failing test**

Append to `tests/semantic_migration_v1.rs`:

```rust
/// Phase Reform Task 6: a freshly-created store is at schema_version 4 and
/// its entities table has no `domain` column. This is the fresh-store half
/// of the reform (existing v3 stores go through the migration in Tasks 3–5).
#[test]
fn fresh_store_is_at_v4_without_domain_column() {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore, CURRENT_DISK_SCHEMA_VERSION};
    use rusqlite::Connection;

    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path()))
            .expect("create");

    assert_eq!(
        CURRENT_DISK_SCHEMA_VERSION, 4,
        "the constant must be bumped to 4 by this task"
    );
    assert_eq!(store.marker.schema_version, 4);

    let db_path = root.join("semantic.sqlite3");
    drop(store);
    let conn = Connection::open(&db_path).expect("open");
    let has_domain_column: bool = conn
        .prepare("PRAGMA table_info(entities)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .any(|col: String| col == "domain");
    assert!(
        !has_domain_column,
        "fresh v4 store must not have entities.domain"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test semantic_migration_v1 fresh_store_is_at_v4_without_domain_column -- --nocapture`
Expected: FAIL — `CURRENT_DISK_SCHEMA_VERSION` is still 3 and the fresh DDL still emits the `domain` column.

- [ ] **Step 3: Bump the version constant**

In `src/semantic.rs:98`, change:

```rust
pub const CURRENT_DISK_SCHEMA_VERSION: u8 = 3;
```
to:

```rust
pub const CURRENT_DISK_SCHEMA_VERSION: u8 = 4;
```

Also update the long doc comment above it (`src/semantic.rs:78-102`) to describe v3→v4 as a genuine migration. Replace the comment opening (lines around 80-90) to add v3→v4:

```rust
/// On-disk DDL schema version stamped into the marker and the `meta` table.
///
/// Version history:
///   * 1 — Task 2.2 baseline (entity table introduction).
///   * 2 — Task 2.2 second revision (entity_aliases).
///   * 3 — Task F3.3 placeholder (every step reversible noop).
///   * 4 — Entity Identity Reform (drop `domain` from entities UNIQUE and
///         entity_aliases PK; consolidate fragmented subjects). The v3→v4
///         migration is the first NON-noop upgrade: it both rewrites data
///         (consolidation) and changes constraints (table recreation).
///
/// A store created under an older schema_version that has a known migration
/// path (today: 2 → 3, 3 → 4) refuses to serve until an operator runs
/// `llm-wiki recovery upgrade`; an older version with NO migration path
/// still fails closed (see `validate_database_identity`).
///
/// NOTE: this is the *on-disk DDL* version, distinct from the *event wire*
/// version stamped on each `EventEnvelope.schema_version`. The wire format
/// of an event has not changed (same `EventEnvelope` JSON shape), so events
/// keep `schema_version: 1` to stay valid against the hash-locked
/// `event-schema-v1.json` contract.
pub const CURRENT_DISK_SCHEMA_VERSION: u8 = 4;
```

- [ ] **Step 4: Update the fresh-store DDL**

Replace `src/semantic.rs:6432-6454`:

```rust
             -- Task 2.2 entity model (ADR Decision 3, Entity Identity Reform
             -- v4). One stable UUIDv7 per canonical_subject — domain is no
             -- longer part of entity identity (Wikidata pattern). Rename
             -- updates canonical_subject but keeps entity_id; merge rewrites
             -- claim_status.entity_id and turns the source subject into an
             -- alias row. Multiple domains per entity live as tags on the
             -- individual claim_status rows, not on the entity itself.
             CREATE TABLE entities(
               entity_id TEXT PRIMARY KEY,
               canonical_subject TEXT NOT NULL,
               created_at TEXT NOT NULL,
               UNIQUE(canonical_subject)
             );
             -- Every subject string (or external id) that has ever resolved to
             -- an entity. kind='canonical' mirrors the current
             -- canonical_subject; kind='former_subject' is a rename/merge
             -- backlink; kind='external' is a provider id alias (Decision 3).
             -- PK excludes domain since the Entity Identity Reform: the same
             -- alias string resolving to the same entity is one row, not one
             -- row per (former) domain.
             CREATE TABLE entity_aliases(
               alias TEXT NOT NULL,
               entity_id TEXT NOT NULL,
               kind TEXT NOT NULL,
               aliased_at_event_seq INTEGER NOT NULL,
               PRIMARY KEY(alias, entity_id)
             );
```

- [ ] **Step 5: Run the new test plus the existing migration suite**

Run: `cargo test --test semantic_migration_v1 -- --nocapture`
Expected: PASS — all v3→v4 + fresh-store tests green.

- [ ] **Step 6: Build the workspace to surface call-site breakage**

Run: `cargo build --workspace`
Expected: FAILURES — the entity-resolution functions still reference the dropped `domain` column in their SQL. This is expected and is the RED state for Tasks 8–11. Do NOT commit yet; proceed to Task 7 to capture this state, then fix in Task 8.

Actually, before continuing: stash the build breakage mentally. Tasks 7+ fix the call sites. To keep each commit green, we will commit Task 6 *only after* Task 8 makes the build pass. For now, leave the working tree dirty and move to Task 7.

- [ ] **Step 7: Do not commit yet**

The schema bump alone breaks the build (functions still SELECT `domain` from `entities`). We commit at the end of Task 8 once the build is restored. Proceed to Task 7.

---

## Task 7: Update `entity_by_id` to stop reading `entities.domain`

The `entity_by_id` function recovers `domain` from `entity_aliases` to populate `EntityRecord.domain`. After the reform, `entities` has no `domain` column and the aliases table's PK no longer carries it. This task updates the SELECTs so the build recovers for this one function.

**Files:**
- Modify: `src/semantic.rs:3112-3143` (`entity_by_id`)
- Modify: `src/semantic.rs:526-531` (`EntityRecord` — drop `domain` field)

- [ ] **Step 1: Update `EntityRecord`**

Replace `src/semantic.rs:523-531`:

```rust
/// One row of the `entities` table (Task 2.2). Stable UUIDv7 identity that
/// never encodes the subject string. `canonical_subject` is the display
/// name (mutable via `rename_entity`); `domain` is no longer carried here
/// after the Entity Identity Reform (domain is a per-claim tag, see
/// `claim_status.domain`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityRecord {
    pub entity_id: Uuid,
    pub canonical_subject: String,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 2: Update `entity_by_id`**

Replace `src/semantic.rs:3112-3143`:

```rust
    /// Read the canonical subject + identity of an entity by its stable id.
    pub fn entity_by_id(&self, context: &TrustedContext, entity_id: Uuid) -> Result<EntityRecord> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        let row: Option<(String, String)> = connection
            .query_row(
                "SELECT canonical_subject, created_at FROM entities WHERE entity_id=?1",
                [entity_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(database_error)?;
        let (canonical_subject, created_at) =
            row.ok_or_else(|| SemanticError::MissingDependency(format!("entity {entity_id}")))?;
        let parsed_created = created_at.parse().map_err(|_| {
            SemanticError::CorruptLedger("entity created_at is not RFC 3339".to_owned())
        })?;
        Ok(EntityRecord {
            entity_id,
            canonical_subject,
            created_at: parsed_created,
        })
    }
```

- [ ] **Step 3: Do not commit yet — Tasks 8–11 finish the call-site updates**

Proceed to Task 8.

---

## Task 8: Update entity-resolution helpers (internal + public) to drop `domain`

This is the core architectural change. `resolve_or_create_entity_in_tx`, `resolve_entity_in_tx`, `insert_alias` drop their `domain` parameter; the public `resolve_or_create_entity` and `resolve_entity` follow. The build should recover for these functions after this task.

**Files:**
- Modify: `src/semantic.rs:7285-7349` (internal helpers)
- Modify: `src/semantic.rs:3069-3109` (public wrappers)
- Modify: `src/semantic.rs:7135-7151` (`build_confirmation_material` call site)

- [ ] **Step 1: Update `resolve_or_create_entity_in_tx`**

Replace `src/semantic.rs:7281-7310`:

```rust
/// Resolve `subject` to an entity_id inside the given transaction, minting a
/// new entity (and a `canonical` alias) if none exists yet. Used by
/// `finish_confirmation` so every confirmed claim is bound to a stable entity
/// in the same transaction that writes its `claim_status` row.
///
/// Entity Identity Reform: domain is no longer part of identity (Wikidata
/// pattern). Two claims with the same subject but different domains resolve
/// to the same entity_id.
fn resolve_or_create_entity_in_tx(
    connection: &Connection,
    subject: &str,
    event_seq: u64,
) -> Result<Uuid> {
    if let Some(entity_id) = resolve_entity_in_tx(connection, subject)? {
        return Ok(entity_id);
    }
    let entity_id = Uuid::now_v7();
    connection
        .execute(
            "INSERT INTO entities(entity_id,canonical_subject,created_at) VALUES (?1,?2,?3)",
            params![entity_id.to_string(), subject, now_rfc3339()],
        )
        .map_err(database_error)?;
    insert_alias(connection, subject, entity_id, "canonical", event_seq)?;
    Ok(entity_id)
}
```

- [ ] **Step 2: Update `resolve_entity_in_tx`**

Replace `src/semantic.rs:7312-7332`:

```rust
/// Resolve `alias` to an entity_id by checking the `entity_aliases` table
/// (former subjects + external ids + canonical). Returns `None` if no entity
/// has ever held this string. Domain-independent since the Entity Identity
/// Reform.
fn resolve_entity_in_tx(
    connection: &Connection,
    alias: &str,
) -> Result<Option<Uuid>> {
    let row: Option<(String,)> = connection
        .query_row(
            "SELECT entity_id FROM entity_aliases WHERE alias=?1 ORDER BY aliased_at_event_seq DESC LIMIT 1",
            params![alias],
            |row| Ok((row.get::<_, String>(0)?,)),
        )
        .optional()
        .map_err(database_error)?;
    match row {
        Some((text,)) => Ok(Uuid::parse_str(&text).ok()),
        None => Ok(None),
    }
}
```

- [ ] **Step 3: Update `insert_alias`**

Replace `src/semantic.rs:7334-7349`:

```rust
fn insert_alias(
    connection: &Connection,
    alias: &str,
    entity_id: Uuid,
    kind: &str,
    event_seq: u64,
) -> Result<()> {
    connection
        .execute(
            "INSERT OR IGNORE INTO entity_aliases(alias,entity_id,kind,aliased_at_event_seq) VALUES (?1,?2,?3,?4)",
            params![alias, entity_id.to_string(), kind, event_seq as i64],
        )
        .map_err(database_error)?;
    Ok(())
}
```

- [ ] **Step 4: Update the public `resolve_or_create_entity`**

Replace `src/semantic.rs:3060-3093`:

```rust
    /// Resolve `subject` to a stable entity_id, minting a new entity (and a
    /// `canonical` alias) if none exists yet. Domain-independent since the
    /// Entity Identity Reform (Wikidata pattern): two claims with the same
    /// subject but different domains resolve to the same entity_id.
    pub fn resolve_or_create_entity(
        &self,
        context: &TrustedContext,
        subject: &str,
    ) -> Result<Uuid> {
        validate_context(&self.marker, context)?;
        // Take the in-process writer lock BEFORE opening the Immediate
        // transaction, matching `mutate_once`'s ordering: without it, two
        // threads could both BEGIN IMMEDIATE on separate connections and the
        // loser would surface a DatabaseContention error with no retry loop.
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let entity_id = resolve_or_create_entity_in_tx(
            &transaction,
            subject,
            next_event_seq(&transaction, context.owner_id)?,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(entity_id)
    }
```

- [ ] **Step 5: Update the public `resolve_entity`**

Replace `src/semantic.rs:3095-3109`:

```rust
    /// Resolve `alias` to an entity_id without creating one. Returns
    /// `Err(MissingDependency)` if no entity has ever held this alias — callers
    /// that need create-on-demand semantics use [`Self::resolve_or_create_entity`].
    /// Domain-independent since the Entity Identity Reform.
    pub fn resolve_entity(
        &self,
        context: &TrustedContext,
        alias: &str,
    ) -> Result<Uuid> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        resolve_entity_in_tx(&connection, alias)?
            .ok_or_else(|| SemanticError::MissingDependency(format!("entity {alias}")))
    }
```

- [ ] **Step 6: Update `build_confirmation_material`'s call site**

Replace `src/semantic.rs:7146-7151`:

```rust
    // Resolve (or lazily create) the entity this claim attaches to, inside the
    // same confirm transaction so the claim_status row is never written
    // without an entity_id. This is the point where a confirmed claim becomes
    // bound to a stable UUIDv7 identity (ADR Decision 3, Task 2.2). Domain is
    // no longer part of identity (Entity Identity Reform): two claims with the
    // same subject but different domains bind to the same entity.
    let entity_id =
        resolve_or_create_entity_in_tx(transaction, &subject, identity.event_seq)?;
```

- [ ] **Step 7: Build to surface remaining call-site breakage**

Run: `cargo build --workspace 2>&1 | head -60`
Expected: FAILURES — but now contained to: (a) `rename_entity`/`merge_entities`/`split_entities` (which still SELECT `domain` from entities and call `insert_alias` with the old 6-arg signature), (b) test files that call the public `resolve_or_create_entity(&context, domain, subject)` and `resolve_entity(&context, domain, alias)` with the old 3-arg signature, (c) `ClaimDraft` construction sites (Tasks 15–17). Proceed to Task 9 to fix the entity-op methods.

- [ ] **Step 8: Do not commit yet**

We commit once the full workspace builds (end of Task 14). Proceed to Task 9.

---

## Task 9: Update `rename_entity` — drop domain from collision check and insert_alias calls

`rename_entity` reads `(domain, canonical_subject)` from entities, checks collisions scoped by domain, and calls `insert_alias` with the 6-arg signature. After the reform, the collision check is subject-only and `insert_alias` takes 5 args.

**Files:**
- Modify: `src/semantic.rs:3150-3220` (`rename_entity`)

- [ ] **Step 1: Rewrite `rename_entity`**

Replace `src/semantic.rs:3145-3220` (the doc comment + method):

```rust
    /// Rename an entity's canonical subject (Task 2.2 DoD bullet 3). Emits an
    /// `entity_renamed` event, updates `entities.canonical_subject`, and
    /// records the OLD subject as a `former_subject` alias so existing
    /// references keep resolving (backlink preservation). The new subject must
    /// not collide with another entity's canonical_subject — that is a merge.
    /// Domain-independent since the Entity Identity Reform.
    pub fn rename_entity(
        &self,
        context: &TrustedContext,
        command: RenameEntityCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("rename_entity", &command)?;
        let entity_id = command.entity_id;
        let new_subject = command.new_subject.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                // Load current canonical row.
                let row: Option<String> = transaction
                    .query_row(
                        "SELECT canonical_subject FROM entities WHERE entity_id=?1",
                        [entity_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                let old_subject = row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {entity_id}"))
                })?;
                if old_subject == new_subject {
                    return Err(SemanticError::InvalidTransition(format!(
                        "entity {entity_id} canonical subject is already {new_subject}"
                    )));
                }
                // Collision check: a *different* entity already owns this subject.
                // Domain-independent since the reform.
                let collision: Option<String> = transaction
                    .query_row(
                        "SELECT entity_id FROM entities WHERE canonical_subject=?1 AND entity_id<>?2",
                        params![new_subject, entity_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                if collision.is_some() {
                    return Err(SemanticError::InvalidTransition(format!(
                        "subject {new_subject} is already canonical for a different entity; use merge instead"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE entities SET canonical_subject=?2 WHERE entity_id=?1",
                        params![entity_id.to_string(), new_subject.clone()],
                    )
                    .map_err(database_error)?;
                // Record the old subject as a former_subject alias (backlink)
                // and the new subject as canonical.
                insert_alias(transaction, &old_subject, entity_id, "former_subject", identity.event_seq)?;
                insert_alias(transaction, &new_subject, entity_id, "canonical", identity.event_seq)?;
                let payload = serde_json::json!({
                    "kind": "entity_renamed",
                    "entity_id": entity_id,
                    "old_subject": old_subject,
                    "new_subject": new_subject,
                });
                Ok(MutationMaterial {
                    event_type: "entity_renamed",
                    object_bytes: canonical_bytes(&payload)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }
```

- [ ] **Step 2: Do not commit yet — continue to Task 10**

---

## Task 10: Update `merge_entities` — delete cross-domain guard

`merge_entities` SELECTs `(domain, canonical_subject)` and rejects cross-domain merges. After the reform there are no domains on entities, so the guard is gone and the SELECT simplifies. The `insert_alias` call loses its domain arg.

**Files:**
- Modify: `src/semantic.rs:3222-3326` (`merge_entities`)

- [ ] **Step 1: Rewrite `merge_entities`**

Replace `src/semantic.rs:3222-3326` (the doc comment + method):

```rust
    /// Merge the source entity into the target (Task 2.2 DoD bullet 3). Emits
    /// an `entity_merged` event, rewrites every `claim_status.entity_id` from
    /// source onto target, and turns the source's canonical subject (plus its
    /// prior aliases) into backlinks pointing at the target. No claim loses
    /// its entity reference; no alias is deleted.
    ///
    /// Entity Identity Reform: the cross-domain guard is removed. Domain is
    /// no longer part of entity identity, so any two entities can merge.
    pub fn merge_entities(
        &self,
        context: &TrustedContext,
        command: MergeEntitiesCommand,
    ) -> Result<MutationOutcome> {
        if command.source_entity_id == command.target_entity_id {
            return Err(SemanticError::InvalidTransition(format!(
                "cannot merge entity {} into itself",
                command.source_entity_id
            )));
        }
        let request_hash = request_hash("merge_entities", &command)?;
        let source = command.source_entity_id;
        let target = command.target_entity_id;
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                let source_row: Option<String> = transaction
                    .query_row(
                        "SELECT canonical_subject FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                let source_subject = source_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {source}"))
                })?;
                let target_row: Option<()> = transaction
                    .query_row(
                        "SELECT 1 FROM entities WHERE entity_id=?1",
                        [target.to_string()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(database_error)?;
                target_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {target}"))
                })?;
                // Rewrite every claim attached to the source onto the target.
                let moved = transaction
                    .execute(
                        "UPDATE claim_status SET entity_id=?2 WHERE entity_id=?1",
                        params![source.to_string(), target.to_string()],
                    )
                    .map_err(database_error)?;
                // Fold the source's canonical subject + all its aliases onto the
                // target so every historical reference keeps resolving.
                let mut alias_statement = transaction
                    .prepare("SELECT alias FROM entity_aliases WHERE entity_id=?1")
                    .map_err(database_error)?;
                let alias_rows = alias_statement
                    .query_map([source.to_string()], |row| row.get::<_, String>(0))
                    .map_err(database_error)?;
                let mut aliases = Vec::new();
                for alias_row in alias_rows {
                    aliases.push(alias_row.map_err(database_error)?);
                }
                drop(alias_statement);
                aliases.push(source_subject.clone());
                for alias in &aliases {
                    insert_alias(transaction, alias, target, "former_subject", identity.event_seq)?;
                }
                // Remove the source's canonical row: its claims were already
                // rewritten onto the target, and every alias (including its
                // former canonical subject) now points at the target with a
                // higher `aliased_at_event_seq`, so `resolve_entity_in_tx`'s
                // `ORDER BY ... DESC LIMIT 1` resolves the target. The merge
                // event itself is the audit record — the source entity_id is
                // captured in the event payload below.
                transaction
                    .execute(
                        "DELETE FROM entity_aliases WHERE entity_id=?1",
                        [source.to_string()],
                    )
                    .map_err(database_error)?;
                transaction
                    .execute(
                        "DELETE FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                    )
                    .map_err(database_error)?;
                let payload = serde_json::json!({
                    "kind": "entity_merged",
                    "source_entity_id": source,
                    "target_entity_id": target,
                    "claims_moved": moved,
                });
                Ok(MutationMaterial {
                    event_type: "entity_merged",
                    object_bytes: canonical_bytes(&payload)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }
```

- [ ] **Step 2: Do not commit yet — continue to Task 11**

---

## Task 11: Update `split_entities` — delete cross-domain guard

Same treatment as `merge_entities`: drop the cross-domain check, simplify the source-row SELECT, drop domain from the event payload.

**Files:**
- Modify: `src/semantic.rs:5669-5758` and surrounding payload assembly (`src/semantic.rs:5820-5828` event payload)

- [ ] **Step 1: Replace the guard section of `split_entities`**

Replace `src/semantic.rs:5715-5758` (the closure opening through the cross-domain check):

```rust
            move |transaction, _identity| {
                // Load source row to confirm it exists and capture its canonical_subject.
                let source_row: Option<String> = transaction
                    .query_row(
                        "SELECT canonical_subject FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                let _source_subject = source_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {source}"))
                })?;

                // Validate every target exists. Entity Identity Reform: the
                // cross-domain check is removed — domain is no longer part of
                // entity identity, so any two entities can split.
                let mut seen_targets: std::collections::HashSet<Uuid> =
                    std::collections::HashSet::new();
                for assignment in &assignments {
                    if !seen_targets.insert(assignment.target_entity_id) {
                        // Already validated; skip the duplicate SELECT.
                        continue;
                    }
                    let exists: Option<()> = transaction
                        .query_row(
                            "SELECT 1 FROM entities WHERE entity_id=?1",
                            [assignment.target_entity_id.to_string()],
                            |_| Ok(()),
                        )
                        .optional()
                        .map_err(database_error)?;
                    if exists.is_none() {
                        return Err(SemanticError::MissingDependency(format!(
                            "entity {}",
                            assignment.target_entity_id
                        )));
                    }
                }
```

(The rest of the closure — the claim rewriting loop and the moved_claims collection at lines 5760-5818 — does not reference domain and stays unchanged.)

- [ ] **Step 2: Update the event payload to drop `domain`**

Replace `src/semantic.rs:5820-5828`:

```rust
                let payload = serde_json::json!({
                    "kind": "entity_split",
                    "source_entity_id": source,
                    "assignments": assignments.iter().map(|a| serde_json::json!({
                        "predicate": a.predicate,
                        "target_entity_id": a.target_entity_id,
                    })).collect::<Vec<_>>(),
                    "moved_claims": moved_claims.iter().map(|m| serde_json::json!({
```

(Only the `"domain": source_domain,` line is removed; the rest of the payload assembly below stays.)

- [ ] **Step 3: Do not commit yet — continue to Task 12**

---

## Task 12: Update remaining SQL that references the dropped `domain` column

Search for any other SELECT/INSERT against `entities` or `entity_aliases` that still names the `domain` column, and update them. The known call sites after Tasks 8–11: the `backfill_entity_ids` path and any helper queries.

**Files:**
- Modify: wherever `cargo build` reports `no such column: domain` after Task 11.

- [ ] **Step 1: Find all remaining references**

Run: `grep -n "FROM entities\|FROM entity_aliases\|INTO entities\|INTO entity_aliases" src/semantic.rs`
Inspect each hit. Specifically check:
- `src/semantic.rs:3284` (`SELECT alias FROM entity_aliases WHERE entity_id=?1` in merge_entities — already updated in Task 10).
- `src/semantic.rs:3408+` (`backfill_entity_ids` — verify whether it touches entities by domain).

- [ ] **Step 2: For each stale query, drop the `domain` column reference**

Likely sites in `backfill_entity_ids` (`src/semantic.rs:3428-3540`) — this function resolves `(domain, subject)` to an entity_id for legacy NULL rows. After the reform, it resolves by `subject` alone. Update its inner `resolve_entity_in_tx` call to the new 2-arg signature, and any `entity_aliases` SELECT to drop the `domain` column.

Read the function first:

Run: `sed -n '3405,3545p' src/semantic.rs` (inspect; do not edit blindly)

For each `resolve_entity_in_tx(transaction, domain, subject)` call, change to `resolve_entity_in_tx(transaction, subject)`. For each `resolve_or_create_entity_in_tx(transaction, domain, subject, seq)` call, change to `resolve_or_create_entity_in_tx(transaction, subject, seq)`.

- [ ] **Step 3: Build again**

Run: `cargo build --workspace 2>&1 | head -80`
Expected: The src/ tree now compiles. Remaining errors are in tests/ — those are the mechanical signature updates in Tasks 15–17.

- [ ] **Step 4: Do not commit yet — continue to Task 13**

---

## Task 13: Update `claim_timeline` — `domain` becomes optional filter

The public read API `claim_timeline(domain, subject, predicate)` currently requires `domain`. After the reform, `domain` is an optional filter (when `None`, return claims across all domains for that subject/predicate). This matches the design-doc §6.4 "domain becomes optional filter".

**Files:**
- Modify: `src/semantic.rs:4096-4153` (`claim_timeline`)

- [ ] **Step 1: Rewrite `claim_timeline`**

Replace `src/semantic.rs:4092-4153`:

```rust
    /// Flat chronological claim history for one `(subject, predicate)` scope,
    /// optionally narrowed by `domain` (Task 5.1 Entity timeline). Returns
    /// every confirmed claim ever recorded in that scope, oldest first.
    ///
    /// Entity Identity Reform: `domain` is an optional filter, not part of the
    /// lookup key. `None` returns claims across every domain for the given
    /// subject/predicate — the natural shape for "everything we know about X".
    pub fn claim_timeline(
        &self,
        domain: Option<&str>,
        subject: &str,
        predicate: &str,
    ) -> Result<Vec<ClaimView>> {
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = if let Some(d) = domain {
            connection.prepare(
                "SELECT claim_id, confirmed_event_seq, entity_id
                 FROM claim_status
                 WHERE domain=?1 AND subject=?2 AND predicate=?3
                 ORDER BY confirmed_event_seq ASC",
            )
        } else {
            connection.prepare(
                "SELECT claim_id, confirmed_event_seq, entity_id
                 FROM claim_status
                 WHERE subject=?1 AND predicate=?2
                 ORDER BY confirmed_event_seq ASC",
            )
        }
        .map_err(database_error)?;
        let rows = if let Some(d) = domain {
            statement
                .query_map(params![d, subject, predicate], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(database_error)?
        } else {
            statement
                .query_map(params![subject, predicate], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(database_error)?
        };

        let mut timeline = Vec::new();
        for row in rows {
            let (claim_id, confirmed_event_seq, entity_id) = row.map_err(database_error)?;
            let confirmed_event_seq = u64::try_from(confirmed_event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_event_seq as i64],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let claim = object.claim;
            if claim.claim_id.to_string() != claim_id {
                return Err(SemanticError::CorruptLedger(
                    "claim_status row does not match confirmation event".to_owned(),
                ));
            }
            timeline.push(build_claim_view(
                claim,
                confirmed_event_seq,
                entity_id.as_deref(),
            ));
        }
        Ok(timeline)
    }
```

- [ ] **Step 2: Update the `api.rs` call site (covered in Task 18)**

For now, leave `src/api.rs:841` calling `.claim_timeline(&params.domain, ...)` — it will fail to compile because `params.domain` is `String` not `Option<&str>`. That is fixed in Task 18 alongside making `TimelineParams.domain` optional.

- [ ] **Step 3: Build the src tree**

Run: `cargo build --lib 2>&1 | head -40`
Expected: src/ compiles (lib only). The `api.rs` errors come from the workspace build, not the lib.

- [ ] **Step 4: Do not commit yet — continue to Task 14**

---

## Task 14: Update types — `ClaimDraft.domain` and `ProposalSummary.domain` become `Option<String>`

The two struct fields that the design-doc §6.7 names. `ClaimDraft.domain: String` → `Option<String>`; `ProposalSummary.domain: String` → `Option<String>`. `ClaimView.domain` stays `String` (it is read from `claim_status.domain`, which remains a per-claim tag).

**Files:**
- Modify: `src/semantic.rs:309-319` (`ClaimDraft`)
- Modify: `src/semantic.rs:671-683` (`ProposalSummary`)
- Modify: `src/semantic.rs:6806-6828` (`validate_claim_draft` — relax the empty-domain check)
- Modify: `src/semantic.rs:7135-7145` (`build_confirmation_material` destructure)
- Modify: `src/semantic.rs:4077-4087` (`list_pending_proposals` ProposalSummary construction)
- Modify: `src/mcp/handlers.rs:1162,1229,1299` (3 ClaimDraft constructions — wrap `Some`)
- Modify: `src/api.rs:841` (already covered in Task 18, but the `domain` arg becomes `Option<&str>`)

- [ ] **Step 1: Update `ClaimDraft`**

Replace `src/semantic.rs:309-319`:

```rust
pub struct ClaimDraft {
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub claim_kind: String,
    /// Per-claim domain tag. Entity Identity Reform: domain is no longer part
    /// of entity identity; it remains as an optional categorization tag on
    /// each claim (mirrors Wikidata's `instance of` statements, MusicBrainz
    /// genre, OSM tags). `None` is permitted for legacy or domain-agnostic
    /// claims; the historical default is the empty string "".
    pub domain: Option<String>,
    pub confidence_basis_points: u16,
    pub privacy_label: PrivacyLabel,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}
```

- [ ] **Step 2: Update `ProposalSummary`**

Replace `src/semantic.rs:671-683`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProposalSummary {
    pub proposal_id: Uuid,
    /// Per-claim domain tag — `None` for domain-agnostic proposals.
    /// Entity Identity Reform: not part of identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub claim_kind: String,
    /// ADR Decision 6 provenance variant name (`evidence` / `inference` /
    /// `user_assertion` / `mechanical`).
    pub provenance_kind: String,
    pub submitted_at: DateTime<Utc>,
    pub event_seq: u64,
}
```

- [ ] **Step 3: Update `validate_claim_draft`**

Replace `src/semantic.rs:6806-6828`:

```rust
fn validate_claim_draft(draft: &ClaimDraft) -> Result<()> {
    validate_interval(draft.valid_from, draft.valid_to)?;
    if draft.subject.trim().is_empty()
        || draft.predicate.trim().is_empty()
        || draft.claim_kind.trim().is_empty()
    {
        return Err(SemanticError::InvalidClaim(
            "subject, predicate, and kind are required".to_owned(),
        ));
    }
    // Entity Identity Reform: domain is optional. If present, allow any
    // non-empty string (it is a categorization tag, not an identity key).
    if let Some(d) = &draft.domain {
        if d.trim().is_empty() {
            // Treat whitespace-only domain as None — callers that build a
            // draft with "   " should not see a stored "   " tag.
            // (We do not mutate draft here; the caller's intent is captured
            // by the tag value going forward; legacy "" is preserved as-is.)
        }
    }
    if draft.confidence_basis_points > 10_000 {
        return Err(SemanticError::InvalidClaim(
            "confidence_basis_points must be between 0 and 10000".to_owned(),
        ));
    }
    if draft.privacy_label != PrivacyLabel::LocalOnly {
        return Err(SemanticError::InvalidClaim(
            "privacy_label must remain local_only until a policy event authorizes release"
                .to_owned(),
        ));
    }
    Ok(())
}
```

- [ ] **Step 4: Update `build_confirmation_material` destructure + claim_status insert**

The destructure at `src/semantic.rs:7135-7145` pulls `domain` out of the draft as `String`. After the reform it is `Option<String>`. The `claim_status.domain` column is still `TEXT NOT NULL`, so we default to `""` when the draft's domain is `None` (or we could keep the legacy tag). For maximum backward compatibility with existing rows, we store `domain.unwrap_or_default()`.

Replace `src/semantic.rs:7152-7186` (the ConfirmationObject construction + the UPDATE/INSERT block):

```rust
    let domain_for_claim = domain.clone().unwrap_or_default();
    let confirmation = ConfirmationObject {
        kind: "claim_confirmation".to_owned(),
        claim: ClaimRecord {
            claim_id,
            proposal_id,
            subject: subject.clone(),
            predicate: predicate.clone(),
            value,
            claim_kind,
            status: "confirmed".to_owned(),
            domain: domain_for_claim.clone(),
            confidence_basis_points,
            privacy_label,
            valid_from,
            valid_to,
            recorded_event_id: identity.event_id,
            recorded_event_seq: identity.event_seq,
            provenance: proposal.provenance,
            supersedes: superseded_claim_ids.clone(),
            retracts: Vec::new(),
        },
    };

    transaction
        .execute(
            "UPDATE proposal_status SET status='confirmed', claim_id=?2 WHERE proposal_id=?1",
            params![proposal_id.to_string(), claim_id.to_string()],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq,entity_id) VALUES (?1,?2,?3,?4,?5,NULL,NULL,?6)",
            params![claim_id.to_string(), domain_for_claim, subject, predicate, identity.event_seq as i64, entity_id.to_string()],
        )
        .map_err(database_error)?;
```

(The `let ClaimDraft { ... } = proposal.draft;` destructure above stays — `domain` is now `Option<String>` automatically because the struct field changed type.)

- [ ] **Step 5: Update `list_pending_proposals` ProposalSummary construction**

Replace `src/semantic.rs:4077-4087`:

```rust
            summaries.push(ProposalSummary {
                proposal_id: proposal.proposal_id,
                domain: proposal.draft.domain,
                subject: proposal.draft.subject,
                predicate: proposal.draft.predicate,
                value: proposal.draft.value,
                claim_kind: proposal.draft.claim_kind,
                provenance_kind: proposal.provenance.kind().to_owned(),
                submitted_at: envelope.recorded_at,
                event_seq,
            });
```

(`proposal.draft.domain` is now `Option<String>` and matches the field; no other change needed here.)

- [ ] **Step 6: Update MCP handler ClaimDraft constructions**

In `src/mcp/handlers.rs`, three `ClaimDraft { ... }` sites currently pass `domain` as a `String`. They need to wrap in `Some(...)` AND handle the new optionality of the `arg_str_req` lookup (Task 19 makes the MCP arg optional). For now, change the three sites minimally so the lib compiles; the MCP arg parsing is Task 19.

Replace `src/mcp/handlers.rs:1162` (inside `handle_brain_ingest_*` — confirm by reading):

Run: `sed -n '1155,1175p' src/mcp/handlers.rs` (inspect the third site first — it may be in a different function than brain_capture/brain_propose).

For each of the three sites at lines 1162, 1229, 1299, change:

```rust
                    domain,
```
to:

```rust
                    domain: Some(domain),
```

(The variable `domain` on the right is still a `String` from `arg_str_req` — Task 19 makes the lookup optional.)

- [ ] **Step 7: Build the lib + bins (skip tests for now)**

Run: `cargo build --lib --bins 2>&1 | head -60`
Expected: PASS for the lib. The bin build may still surface `api.rs` errors — those are fixed in Task 18. If `cargo build --lib` passes, proceed.

- [ ] **Step 8: Do not commit yet — continue to Task 15**

---

## Task 15: Update `api.rs` timeline handler — `domain` optional

`TimelineParams.domain` becomes `Option<String>`, and the handler passes `params.domain.as_deref()` to the new `claim_timeline` signature.

**Files:**
- Modify: `src/api.rs:827-844`

- [ ] **Step 1: Update `TimelineParams` + `timeline`**

Replace `src/api.rs:827-844`:

```rust
#[derive(Deserialize)]
struct TimelineParams {
    /// Entity Identity Reform: domain is an optional filter. `None` returns
    /// every claim for the given subject/predicate across all domains.
    #[serde(default)]
    domain: Option<String>,
    subject: String,
    predicate: String,
}

async fn timeline(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<TimelineParams>,
) -> Result<Response, ApiError> {
    let timeline = state
        .store
        .claim_timeline(params.domain.as_deref(), &params.subject, &params.predicate)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(timeline).into_response())
}
```

- [ ] **Step 2: Build bins**

Run: `cargo build --bins 2>&1 | head -40`
Expected: PASS — bins now compile (lib was already passing).

- [ ] **Step 3: Do not commit yet — continue to Task 16**

---

## Task 16: Update MCP tool manifests — `brain_capture` + `brain_propose` make `domain` optional

The tool manifest declares `domain` as required (in the `"required"` array). Move it out of the required array so MCP clients are not forced to send it.

**Files:**
- Modify: `src/mcp/tools.rs:572-648` (`brain_capture` + `brain_propose` Tool definitions)

- [ ] **Step 1: Update `brain_capture` manifest**

Replace `src/mcp/tools.rs:572-595`:

```rust
        // brain_* mutation tools (Phase C Task C2)
        Tool::new(
            "brain_capture",
            "Capture a user utterance as a proposed claim in the semantic brain",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id (for idempotency)"),
                    "utterance": str_prop("The user's exact utterance text"),
                    "subject": str_prop("Claim subject"),
                    "predicate": str_prop("Claim predicate"),
                    "value": str_prop("Claim value"),
                    "domain": opt_str("Optional domain tag (e.g. stocks, projects). Entity Identity Reform: domain is a categorization tag, not part of entity identity."),
                    "claim_kind": opt_str("Claim kind (default: user_assertion)"),
                }),
                &[
                    "operation_id",
                    "utterance",
                    "subject",
                    "predicate",
                    "value",
                ],
            ),
        ),
```

- [ ] **Step 2: Update `brain_propose` manifest**

Replace `src/mcp/tools.rs:623-648`:

```rust
        Tool::new(
            "brain_propose",
            "Propose a claim derived by inference (AI worker write path) — always status 'proposed', never auto-confirmed",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id (for idempotency)"),
                    "subject": str_prop("Claim subject"),
                    "predicate": str_prop("Claim predicate"),
                    "value": str_prop("Claim value"),
                    "domain": opt_str("Optional domain tag (e.g. stocks, projects). Entity Identity Reform: domain is a categorization tag, not part of entity identity."),
                    "method": str_prop("Inference method (e.g. llm_extraction)"),
                    "model": opt_str("Model name/id used for the inference"),
                    "prompt_version": opt_str("Prompt version identifier"),
                    "claim_kind": opt_str("Claim kind (default: inference)"),
                    "evidence_capture_operation_ids": opt_str("Comma-separated capture operation ids backing this inference (omit for an unsupported/no-evidence proposal)"),
                }),
                &[
                    "operation_id",
                    "subject",
                    "predicate",
                    "value",
                    "method",
                ],
            ),
        ),
```

- [ ] **Step 3: Build bins**

Run: `cargo build --bins 2>&1 | head -40`
Expected: PASS.

- [ ] **Step 4: Do not commit yet — continue to Task 17**

---

## Task 17: Update MCP handlers — `brain_capture` + `brain_propose` accept optional `domain`

The handlers currently use `arg_str_req(args, "domain")`. Switch to `arg_str(args, "domain")` (optional) and wrap in `Some` only when present.

**Files:**
- Modify: `src/mcp/handlers.rs:1199-1251` (`handle_brain_capture`)
- Modify: `src/mcp/handlers.rs:1262-1321` (`handle_brain_propose`)

- [ ] **Step 1: Update `handle_brain_capture`**

Replace `src/mcp/handlers.rs:1199-1251`:

```rust
pub fn handle_brain_capture(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult {
    let Some(store) = &server.semantic_store else {
        return Err("brain not initialized".to_owned());
    };
    let operation_id = arg_str_req(args, "operation_id")?;
    let utterance = arg_str_req(args, "utterance")?;
    let subject = arg_str_req(args, "subject")?;
    let predicate = arg_str_req(args, "predicate")?;
    let value_str = arg_str_req(args, "value")?;
    // Entity Identity Reform: domain is optional.
    let domain = arg_str(args, "domain");
    let claim_kind = arg_str(args, "claim_kind").unwrap_or_else(|| "user_assertion".to_owned());

    let value: serde_json::Value =
        serde_json::from_str(&value_str).unwrap_or(serde_json::Value::String(value_str));

    server
        .check_ingest_limits(utterance.len())
        .map_err(|e| ingest_limit_error_message(&e, server))?;

    let ctx = server.brain_context(store)?;
    let outcome = store
        .propose_user_assertion(
            &ctx,
            crate::semantic::ProposeUserAssertionCommand {
                operation_id,
                utterance: utterance.into_bytes(),
                draft: crate::semantic::ClaimDraft {
                    subject,
                    predicate,
                    value,
                    claim_kind,
                    domain,
                    confidence_basis_points: 9_000,
                    privacy_label: crate::semantic::PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .map_err(|e| format!("{e}"))?;

    let payload = serde_json::json!({
        "event_seq": outcome.event.event_seq,
        "proposal_id": outcome.generated.proposal_id,
        "status": "proposed",
    });
    let s = serde_json::to_string_pretty(&payload).map_err(|e| format!("{e}"))?;
    ok_text(s)
}
```

- [ ] **Step 2: Update `handle_brain_propose`**

Replace `src/mcp/handlers.rs:1262-1321`:

```rust
pub fn handle_brain_propose(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult {
    let Some(store) = &server.semantic_store else {
        return Err("brain not initialized".to_owned());
    };
    let operation_id = arg_str_req(args, "operation_id")?;
    let subject = arg_str_req(args, "subject")?;
    let predicate = arg_str_req(args, "predicate")?;
    let value_str = arg_str_req(args, "value")?;
    // Entity Identity Reform: domain is optional.
    let domain = arg_str(args, "domain");
    let method = arg_str_req(args, "method")?;
    let claim_kind = arg_str(args, "claim_kind").unwrap_or_else(|| "inference".to_owned());
    let model = arg_str(args, "model");
    let prompt_version = arg_str(args, "prompt_version");
    let evidence_capture_operation_ids: Vec<String> =
        arg_str(args, "evidence_capture_operation_ids")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();

    let value: serde_json::Value =
        serde_json::from_str(&value_str).unwrap_or(serde_json::Value::String(value_str));

    let ctx = server.brain_context(store)?;
    let outcome = store
        .propose_inference(
            &ctx,
            crate::semantic::ProposeInferenceCommand {
                operation_id,
                evidence_capture_operation_ids,
                method,
                model,
                prompt_version,
                subject_validator_version: Some(
                    crate::subject_validator::SUBJECT_VALIDATOR_VERSION.to_string(),
                ),
                draft: crate::semantic::ClaimDraft {
                    subject,
                    predicate,
                    value,
                    claim_kind,
                    domain,
                    confidence_basis_points: 9_000,
                    privacy_label: crate::semantic::PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .map_err(|e| format!("{e}"))?;

    let payload = serde_json::json!({
        "event_seq": outcome.event.event_seq,
        "proposal_id": outcome.generated.proposal_id,
        "status": "proposed",
    });
    let s = serde_json::to_string_pretty(&payload).map_err(|e| format!("{e}"))?;
    ok_text(s)
}
```

- [ ] **Step 3: Build the workspace (lib + bins)**

Run: `cargo build --workspace --tests 2>&1 | tail -80`
Expected: FAILURES — but now ALL failures are in `tests/` (mechanical signature updates). The src/ tree is green. We are now ready to commit the src changes as one atomic commit and then update tests in Tasks 18–20.

- [ ] **Step 4: Commit the src changes**

```bash
git add src/ tests/semantic_migration_v1.rs
git commit -m "feat(identity): Entity Identity Reform — drop domain from entity key

- Bump CURRENT_DISK_SCHEMA_VERSION 3 → 4
- v3→v4 migration: consolidate fragmented entities, drop domain from
  entities UNIQUE + entity_aliases PK (Wikidata pattern)
- Drop domain param from resolve_or_create_entity(_in_tx),
  resolve_entity(_in_tx), insert_alias
- Remove cross-domain guard from merge_entities + split_entities
- claim_timeline: domain becomes optional filter
- ClaimDraft.domain, ProposalSummary.domain: Option<String>
- EntityRecord: drop domain field
- HTTP GET /entity/timeline: domain optional
- MCP brain_capture + brain_propose: domain optional (manifest + handler)

Tests in tests/ still use the old signatures; updated in the next commit."
```

The repo is now in a state where src compiles but tests do not. Proceed to Task 18.

---

## Task 18: Update tests — mechanical `ClaimDraft { domain: "..." }` → `domain: Some("...".to_owned())`

There are ~57 `ClaimDraft { ... }` construction sites across 22 test files. Most read `domain: "stocks".to_owned()` (or similar). This is a mechanical regex-friendly replacement. The compiler will guide any misses.

**Files:**
- All 22 files under `tests/` listed by `grep -rln "ClaimDraft {" tests/`.

- [ ] **Step 1: Apply the mechanical replacement with a careful sed**

First, list every site to confirm the pattern is uniform:

Run: `grep -rn "domain:" tests/ | grep -v "^tests/.*\.domain" | head -80`

The vast majority will be `domain: "<literal>".to_owned()` or `domain: <expr>` inside a `ClaimDraft { ... }`. Apply:

```bash
# For each test file, replace `domain: "<literal>"` inside ClaimDraft with the Some-wrapped form.
# This sed targets the common shape `domain: "<word>".to_owned()`.
for f in $(grep -rln "ClaimDraft {" tests/); do
  perl -i -pe 's/(\bdomain:\s*)("[^"]*"\.to_owned\(\))/$1Some($2)/g' "$f"
done
```

Verify the perl worked:

Run: `grep -rn "domain:" tests/ | grep -v "Some(" | head -20`
Expected: A handful of misses — cases like `domain: blank_domain.variable` or `draft_a.domain = "stocks".to_owned()` (assignment, not construction). Fix those by hand.

- [ ] **Step 2: Fix the assignment-style misses by hand**

In `tests/semantic_policy_v1.rs:243`, the line `blank_domain.domain = "   ".to_owned();` is an assignment to a struct field that is now `Option<String>`. Change to:

```rust
    blank_domain.domain = Some("   ".to_owned());
```

In `tests/semantic_purge_v1.rs:859`, `draft_a.domain = "stocks".to_owned();`:

```rust
    draft_a.domain = Some("stocks".to_owned());
```

For any other misses surfaced by the grep in Step 1, apply the same `Some(...)` wrapping.

- [ ] **Step 3: Update public-API call sites in tests**

`resolve_or_create_entity(&context, "stocks", "GULF")` (3-arg) must become `resolve_or_create_entity(&context, "GULF")` (2-arg). Same for `resolve_entity`.

Run: `grep -rn "resolve_or_create_entity\|resolve_entity" tests/ | head -40`

For each hit, drop the middle argument (the domain string). Example in `tests/semantic_ownership_v1.rs:378`:

Before:
```rust
    let source = store
        .resolve_or_create_entity(&context, "stocks", "GULF-dup")
        .expect("source entity");
```
After:
```rust
    let source = store
        .resolve_or_create_entity(&context, "GULF-dup")
        .expect("source entity");
```

Apply to every hit. Most files will have 1–4 hits.

- [ ] **Step 4: Update EntityRecord field accesses**

`EntityRecord` no longer has a `domain` field. Tests that read `after.domain` (e.g. none currently surface, but check) must drop that access.

Run: `grep -rn "\.domain" tests/ | grep -v "draft\.\|claim\.\|proposal\." | head -20`

For any hit on an `EntityRecord`, remove the `.domain` access. (The assertion was likely checking the recovered domain; after the reform there is no domain to check.)

- [ ] **Step 5: Build the test suite**

Run: `cargo build --workspace --tests 2>&1 | tail -60`
Expected: Either PASS (all tests compile), or a small number of remaining errors. Address each by hand. Common patterns:
- `ProposalSummary.domain` now `Option<String>` — assertions comparing `summary.domain == "stocks"` become `summary.domain.as_deref() == Some("stocks")`.
- `claim.domain` (on `ClaimView`) is still `String` — unchanged.
- Any test that called `.claim_timeline("stocks", "GULF", "p")` must pass `Some("stocks")` as the first arg: `.claim_timeline(Some("stocks"), "GULF", "p")`. Or drop the domain to call the cross-domain variant: `.claim_timeline(None, "GULF", "p")`.

Run: `grep -rn "claim_timeline" tests/ | head -10` and fix each.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test --workspace 2>&1 | tail -40`
Expected: MOST tests pass. Two intentional failures remain — the cross-domain tests we rewrite in Task 19. Capture the failures; do not commit yet.

- [ ] **Step 7: Do not commit yet — continue to Task 19**

---

## Task 19: Rewrite the cross-domain tests — they now succeed

`tests/trust_operations_contract_v1.rs:776-800` asserts `split_entities_rejects_cross_domain`. After the reform, cross-domain split is ALLOWED, so this test must be inverted. Similarly, the merge test in `tests/semantic_ownership_v1.rs:367` does not assert cross-domain rejection (it uses same-domain subjects), but the `merge_entities_moves_claims_and_keeps_aliases_as_backlinks` test uses `domain="stocks"` for both — we extend it to cover the cross-domain case.

**Files:**
- Modify: `tests/trust_operations_contract_v1.rs:776-800` (rename + invert)
- Modify: `tests/semantic_ownership_v1.rs:367-446` (add a cross-domain variant)

- [ ] **Step 1: Invert the split_entities cross-domain test**

Replace `tests/trust_operations_contract_v1.rs:776-800` (the test function):

```rust
/// Splitting an entity across (former) domains is now ALLOWED — Entity
/// Identity Reform removed domain from entity identity, so any two entities
/// can split. This test was `split_entities_rejects_cross_domain` before the
/// reform; it is inverted to assert the new behavior.
#[test]
fn split_entities_allows_cross_domain() {
    // Read the existing test body up to the `.expect_err(...)` call.
    // Replace the `.expect_err("cross-domain split must fail")` with
    // `.expect("cross-domain split must succeed")` and adjust the assertion.
    //
    // The two entities are constructed with the same domain string today
    // (they always were, because the old code rejected construction with
    // different domains at the resolve_or_create_entity level... actually no,
    // they were created with different domain strings to provoke the guard).
    //
    // After the reform, resolve_or_create_entity takes no domain, so we
    // construct two entities with the SAME subject-less path and split
    // between them. The test asserts the split succeeds and claims move.
    //
    // See the test body in the file — replace the trailing expect_err with
    // expect_ok and remove the assertion on the error message.
}
```

Read the actual test body first:

Run: `sed -n '776,810p' tests/trust_operations_contract_v1.rs`

Then replace the whole test function with the inverted form. The exact replacement depends on how the test currently constructs entities — after the reform, drop the domain arg from `resolve_or_create_entity`, change `.expect_err("cross-domain split must fail")` to `.expect("cross-domain split must succeed")`, and remove the follow-up assertion on the error string.

- [ ] **Step 2: Add a cross-domain merge test**

Append to `tests/semantic_ownership_v1.rs`:

```rust
/// Entity Identity Reform: merging two entities that originated from
/// different (former) domains now succeeds. Pre-reform this was rejected by
/// the cross-domain guard; post-reform domain is not part of identity.
#[test]
fn merge_entities_allows_cross_domain() {
    let (_parent, _root, store, context) = fixture();

    // Two entities with the same subject are one entity post-reform, so we
    // need two DISTINCT subjects to have two entities to merge.
    let source = store
        .resolve_or_create_entity(&context, "GULF-dup")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("target entity");
    assert_ne!(source, target, "two distinct subjects must be two entities");

    // Put one claim on each, each with a DIFFERENT domain tag (proving the
    // claims carry their own domain even though the entities do not).
    store
        .propose_user_assertion(
            &context,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "assert-source-x".to_owned(),
                utterance: b"source claim".to_vec(),
                draft: ClaimDraft {
                    subject: "GULF-dup".to_owned(),
                    predicate: "target_price".to_owned(),
                    value: json!(58),
                    claim_kind: "user_assertion".to_owned(),
                    domain: Some("stocks".to_owned()),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose source");
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-source-x".to_owned(),
                proposal_operation_id: "assert-source-x".to_owned(),
            },
        )
        .expect("confirm source");

    store
        .merge_entities(
            &context,
            llm_wiki::semantic::MergeEntitiesCommand {
                operation_id: "merge-x".to_owned(),
                source_entity_id: source,
                target_entity_id: target,
            },
        )
        .expect("cross-domain merge must succeed post-reform");

    // The source subject now resolves to the target.
    let resolved = store
        .resolve_entity(&context, "GULF-dup")
        .expect("resolve merged-away subject");
    assert_eq!(resolved, target);
}
```

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace 2>&1 | tail -40`
Expected: PASS (all tests green, including the inverted split test and the new cross-domain merge test).

- [ ] **Step 4: Commit**

```bash
git add tests/
git commit -m "test(identity): update signatures + invert cross-domain tests for reform"
```

---

## Task 20: Add Wikidata-pattern regression tests (new file)

A dedicated test file that encodes the Wikidata identity pattern's invariants. These are the long-term guards against regression: any future change that re-introduces domain into entity identity will fail these.

**Files:**
- Create: `tests/semantic_identity_reform_v1.rs`

- [ ] **Step 1: Create the test file**

Create `tests/semantic_identity_reform_v1.rs`:

```rust
//! Entity Identity Reform — Wikidata pattern regression tests.
//!
//! These tests encode the invariants established by the reform (see
//! `docs/plans/entity-identity-reform-design-doc.md` and ADR-0002):
//!
//! 1. One real-world subject → exactly one entity_id, regardless of domain.
//! 2. Two claims with the same subject but different domains resolve to the
//!    same entity.
//! 3. Cross-domain merge + split succeed.
//! 4. canonical_subject is UNIQUE on its own.
//!
//! Any future change that re-introduces domain into entity identity will
//! fail these tests.

use std::path::Path;

use llm_wiki::semantic::{
    ClaimDraft, ConfirmCommand, PrivacyLabel, SemanticConfig, SemanticStore, TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

fn fixture() -> (TempDir, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, store, context)
}

fn draft(subject: &str, predicate: &str, value: i64, domain: &str) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: Some(domain.to_owned()),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn confirm(store: &SemanticStore, ctx: &TrustedContext, op: &str, propose_op: &str) {
    store
        .confirm(
            ctx,
            ConfirmCommand {
                operation_id: op.to_owned(),
                proposal_operation_id: propose_op.to_owned(),
            },
        )
        .expect("confirm");
}

/// Wikidata pattern: one subject, two different domain tags on two claims
/// → both claims attach to the SAME entity_id. Pre-reform this produced two
/// entities (the CATL fragmentation bug).
#[test]
fn two_claims_same_subject_different_domains_one_entity() {
    let (_parent, store, ctx) = fixture();

    store
        .propose_user_assertion(
            &ctx,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "a-business".to_owned(),
                utterance: b"b".to_vec(),
                draft: draft("CATL", "revenue", 100, "business"),
            },
        )
        .expect("propose 1");
    confirm(&store, &ctx, "c-business", "a-business");

    store
        .propose_user_assertion(
            &ctx,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "a-financial".to_owned(),
                utterance: b"f".to_vec(),
                draft: draft("CATL", "margin", 20, "financial"),
            },
        )
        .expect("propose 2");
    confirm(&store, &ctx, "c-financial", "a-financial");

    // Both claims must resolve to the same entity_id.
    let e1 = store
        .resolve_entity(&ctx, "CATL")
        .expect("resolve after first claim");
    let e2 = store
        .resolve_entity(&ctx, "CATL")
        .expect("resolve after second claim");
    assert_eq!(e1, e2, "two claims on CATL must share one entity_id");

    // And that entity holds exactly 2 claims.
    let claims = store.claims_for_entity(&ctx, e1).expect("claims");
    assert_eq!(claims.len(), 2, "entity must hold both claims");
}

/// canonical_subject is UNIQUE on its own. Attempting to insert a second
/// entity with the same subject fails at the DB constraint.
#[test]
fn canonical_subject_is_unique_post_reform() {
    use rusqlite::Connection;
    let (_parent, store, _ctx) = fixture();
    let _ = store; // hold the store open for the connection below
    // We cannot easily insert a duplicate via the public API (resolve_or_create
    // short-circuits on existing), so we go under the hood to assert the
    // constraint.
    let db_path = _parent.path().join("semantic-store").join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open");
    let dup = conn.execute(
        "INSERT INTO entities(entity_id, canonical_subject, created_at) \
         VALUES ('deadbeef-0000-7000-8000-000000000099', 'UNIQUE_TEST', '2026-07-01T00:00:00Z')",
        [],
    );
    // First insert succeeds (no prior row).
    assert!(dup.is_ok(), "first insert should succeed");
    let dup2 = conn.execute(
        "INSERT INTO entities(entity_id, canonical_subject, created_at) \
         VALUES ('deadbeef-0000-7000-8000-000000000098', 'UNIQUE_TEST', '2026-07-01T00:00:00Z')",
        [],
    );
    assert!(
        dup2.is_err(),
        "second insert with same canonical_subject must fail (UNIQUE constraint)"
    );
}

/// Fresh store has no entities.domain column. This catches a regression
/// where someone re-adds the column to the fresh-store DDL.
#[test]
fn fresh_store_entities_table_has_no_domain_column() {
    use rusqlite::Connection;
    let (_parent, store, _ctx) = fixture();
    let _ = store;
    let db_path = _parent.path().join("semantic-store").join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open");
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entities)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        !cols.iter().any(|c| c == "domain"),
        "entities must not have a domain column post-reform; cols = {cols:?}"
    );
    let alias_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entity_aliases)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        !alias_cols.iter().any(|c| c == "domain"),
        "entity_aliases must not have a domain column post-reform; cols = {alias_cols:?}"
    );
}
```

- [ ] **Step 2: Run the new tests**

Run: `cargo test --test semantic_identity_reform_v1 -- --nocapture`
Expected: PASS (all three).

- [ ] **Step 3: Commit**

```bash
git add tests/semantic_identity_reform_v1.rs
git commit -m "test(identity): Wikidata-pattern regression suite"
```

---

## Task 21: Update conflict-detection bucket key (Phase 1.6 consequence)

The design-doc §2.2 documents that Phase 1.6 conflict detection groups by `(domain, subject, predicate)`, which is why the same fact in different domain variants was invisible. Post-reform, the bucket key is `(subject, predicate)` — cross-domain conflicts now surface. The `domain` field on `Entry`/`ConflictPeer` is kept (it is useful context in the rendered conflict report), but it no longer partitions the buckets.

**Files:**
- Modify: `src/inbox_conflicts.rs:105-112` (bucket key)

- [ ] **Step 1: Update the bucket key**

Replace `src/inbox_conflicts.rs:105-112`:

```rust
    // Bucket by (subject, predicate). Entity Identity Reform: domain is no
    // longer part of the bucket key — two claims with the same subject +
    // predicate but different domain tags are now correctly detected as
    // conflicts (this is the §2.2 "same fact, different fragments" blind
    // spot the reform closes). The domain field stays on Entry/ConflictPeer
    // as display context.
    let mut buckets: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, e) in all.iter().enumerate() {
        buckets
            .entry((e.subject.clone(), e.predicate.clone()))
            .or_default()
            .push(i);
    }
```

- [ ] **Step 2: Check whether any test asserts the OLD (domain-partitioned) behavior**

Run: `grep -rn "detect_conflicts\|inbox_conflicts" tests/ | head -10`

If any test asserts that two claims with the same `(subject, predicate)` but different domains do NOT conflict, that test must now be inverted (they SHOULD conflict). Read each hit and adjust.

- [ ] **Step 3: Run the conflict tests**

Run: `cargo test --test inbox_conflicts_v1 2>/dev/null || cargo test inbox 2>&1 | tail -20`
Expected: PASS (or a small number of failures if Step 2 found a test to invert — fix those).

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -40`
Expected: PASS — every test green.

- [ ] **Step 5: Commit**

```bash
git add src/inbox_conflicts.rs tests/
git commit -m "feat(conflicts): bucket by (subject, predicate) — cross-domain conflicts surface"
```

---

## Task 22: Production migration — run on the live store

The live store at `<state_dir>/semantic-store/semantic.sqlite3` is at v3. After Tasks 1–21, the binary refuses to serve it until an operator runs the upgrade. This task runs the upgrade against a BACKUP first (verification), then against the live store.

**Note on CLI shape:** the `recovery upgrade` subcommand does NOT take a `--root` flag. It resolves `<state_dir>/semantic-store` from the configured `state_dir` (read from `LLM_WIKI_CONFIG` / `~/.llm-wiki/config.toml` via `WikiEngine`). To upgrade a *copy* of the store, point a temporary config at a temporary state_dir. The `from`/`to` versions are inferred from the live marker and `CURRENT_DISK_SCHEMA_VERSION` respectively, so no flags are needed for the normal case.

**Files:** none (operator procedure)

- [ ] **Step 1: Stop the running server**

Stop the brain-mcp server / Docker container so no connection holds the SQLite file.

- [ ] **Step 2: Take an encrypted backup of the live store**

Run: `cargo run --bin llm-wiki -- recovery backup --output ./backups/pre-identity-reform-$(date +%Y%m%d-%H%M%S) --format json`
Expected: a JSON `BackupReport` with non-zero `objects_backed_up` + `ledger_events_backed_up` and an `encrypted: true` field. Capture the `checksum`.

- [ ] **Step 3: Rehearse the upgrade against a copy of the live store**

The upgrade subcommand resolves the store from the configured `state_dir`, so to rehearse against a copy we set up a sibling state_dir and a temporary config. Adjust the paths to match the deployment:

```bash
REHEARSAL_DIR=/tmp/identity-reform-rehearsal
mkdir -p "$REHEARSAL_DIR"
cp -r ./data/semantic-store "$REHEARSAL_DIR/semantic-store"

# Write a minimal config pointing LLM_WIKI_CONFIG at the rehearsal state_dir.
cat > "$REHEARSAL_DIR/config.toml" <<'EOF'
state_dir = "/tmp/identity-reform-rehearsal"
EOF

LLM_WIKI_CONFIG="$REHEARSAL_DIR/config.toml" cargo run --bin llm-wiki -- recovery upgrade --format json
```
Expected: a JSON plan + success message naming v3 → v4. Then verify the rehearsal:

```bash
sqlite3 "$REHEARSAL_DIR/semantic-store/semantic.sqlite3" "SELECT COUNT(*) FROM entities;"
sqlite3 "$REHEARSAL_DIR/semantic-store/semantic.sqlite3" "SELECT canonical_subject, COUNT(*) FROM entities GROUP BY canonical_subject HAVING COUNT(*) > 1;"
```
Expected: The first COUNT is the post-reform entity count (should be ~24, down from ~34). The second query returns ZERO rows (no duplicate canonical_subjects).

If the rehearsal shows any anomaly, STOP and investigate. The live store is not touched until Step 4.

- [ ] **Step 4: If the rehearsal is clean, run the upgrade against the live store**

```bash
cargo run --bin llm-wiki -- recovery upgrade --format json
```
Expected: success. Verify:

```bash
sqlite3 ./data/semantic-store/semantic.sqlite3 "SELECT COUNT(*) FROM entities;"
sqlite3 ./data/semantic-store/semantic.sqlite3 "SELECT canonical_subject FROM entities WHERE canonical_subject='CATL';"
```
Expected: entity COUNT is ~24; the CATL query returns exactly one row.

- [ ] **Step 5: Restart the server and verify the home page**

Start the server, open the Console in a browser, and confirm:
- CATL appears as ONE node in the galaxy (was 6).
- BYD, LG Energy, Sodium-ion, Hungary, Spain JV each appear as ONE node.
- Total entity count is ~24 (was ~34).

Capture a screenshot (e.g. `audit-after-reform-home.png`) for the shipped report.

- [ ] **Step 6: Record the verification**

No code change. The verification numbers feed the shipped report (Task 23, Step 3).

---

## Task 23: Write ADR-0002, update BLUEPRINT, produce the shipped report

The final documentation deliverables. ADR-0002 records the architectural decision; BLUEPRINT notes the reform under Phase 0; the shipped report captures the verified numbers (34 → 24 entities, 0 data loss).

**Files:**
- Create: `docs/adr/0002-entity-identity-reform-wikidata-pattern.md`
- Modify: `BLUEPRINT.md` (Phase 0 section)
- Create: `docs/reports/2026-07-21-entity-identity-reform-shipped.md`
- Modify: `docs/plans/entity-identity-reform-design-doc.md` (mark SHIPPED)

- [ ] **Step 1: Write ADR-0002**

Create `docs/adr/0002-entity-identity-reform-wikidata-pattern.md`:

```markdown
# ADR-0002: Entity Identity Reform — Wikidata Pattern

- Status: **Accepted for Phase 0**
- Date: 2026-07-21
- Decision owners: repository owner and Brain Application Core
- Amends: ADR-0001 §Decision 6 (scope key)
- Baseline: post-reform store at schema_version 4

## Context

ADR-0001 §Decision 6 defines the scope key as
`(owner_id, domain, subject_id, predicate, normalized_context)` and notes that
`latest-user-wins` applies only to `preference`, `profile`, `project decision`
claims. The entity identity model inherited from Task 2.2 encoded `domain`
into the entity identity constraint (`UNIQUE(domain, canonical_subject)`),
which — under LLM free-text domain emission — fragmented each real-world
subject into one entity per domain variant. Verified on 2026-07-21: one
company (CATL) appeared as 6 entities; the home galaxy showed 10 phantom
nodes (34 entities for 24 real).

This violates the universally-observed production knowledge-graph identity
pattern (Wikidata QID, MusicBrainz MBID, OSM element ID, GitHub node ID,
OpenAI Temporal Agents UUID): identity is an opaque, immutable,
system-assigned identifier, and every categorization field (label, type,
domain, tags) is mutable metadata layered on top. No production KG has ever
used a categorization field as part of the identity constraint.

## Decision

Drop `domain` from entity identity. Specifically:

1. `entities` UNIQUE constraint becomes `UNIQUE(canonical_subject)` alone.
2. `entity_aliases` PRIMARY KEY becomes `PRIMARY KEY(alias, entity_id)`.
3. `resolve_or_create_entity_in_tx`, `resolve_entity_in_tx`, `insert_alias`,
   and their public wrappers drop the `domain` parameter.
4. `merge_entities` and `split_entities` no longer reject cross-domain
   operations (the guard is removed).
5. `claim_status.domain` is UNCHANGED — it remains a per-claim categorization
   tag, mirroring Wikidata's `instance of` statements, MusicBrainz genre,
   and OSM tags.
6. `ClaimDraft.domain` and `ProposalSummary.domain` become `Option<String>`.
7. The disk schema version bumps 3 → 4. The v3→v4 migration consolidates
   existing fragmented entities (most-claims-wins per canonical_subject),
   folds aliases onto the surviving entity, deletes the losers, and
   recreates both tables without `domain` in the key — all inside one
   atomic transaction.

## Consequences

- One real-world subject = exactly one entity_id, regardless of how many
  domain variants extraction emits. CATL is now 1 entity, not 6.
- Phase 1.6 conflict detection (which bucketed by `(domain, subject,
  predicate)`) now buckets by `(subject, predicate)`, so cross-domain
  conflicts surface. This is the §2.2 blind-spot fix.
- The cross-domain merge/split guards are gone — any two entities can merge.
- `claim_timeline` accepts `domain` as an optional filter, not a required
  key. `None` returns every claim for the subject/predicate across domains.
- MCP `brain_capture` and `brain_propose` accept `domain` as an optional
  argument. Existing callers that send `domain` continue to work.
- The v3→v4 migration is reversible at the schema level (rollback restores
  the `domain` column) but cannot un-merge consolidated entities — deleted
  rows are gone. A re-run of forward is the documented way to re-assert
  consolidation.

## Alternatives considered

- **Normalize LLM domain emission (taxonomy v2):** reduces the rate of
  fragmentation but does not fix the constraint. A single LLM-emit domain
  not in the canonical list still fragments. Rejected as a defense-in-depth
  complement, not a replacement.
- **Keep domain in identity, add a deduplication job:** a periodic job that
  merges fragmented entities. This is the Neo4j "resolve at query time"
  anti-pattern (cited in the design doc): fragmentation accumulates and is
  expensive to repair downstream. Rejected.
- **Drop `claim_status.domain` entirely:** removes a useful per-claim
  categorization tag and breaks existing conflict-detection display context.
  Rejected — domain-as-tag is the Wikidata pattern.

## References

- `docs/plans/entity-identity-reform-design-doc.md` (research + examples)
- ADR-0001 §Decision 6 (the scope key this amends)
- Wikidata:Identifiers, Help:Items, Help:Merge
- MusicBrainz Identifier, Style/Aliases
- OpenStreetMap Tags, Elements
- OpenAI Temporal Agents with Knowledge Graphs (Cookbook)
- Neo4j Agent Memory: Entity Resolution and Deduplication
```

- [ ] **Step 2: Update BLUEPRINT.md**

Find the Phase 0 section (around line 603). Add a bullet noting the reform:

```markdown
### Phase 0 — Foundation (Week 1)

...existing content...

- **Entity Identity Reform (2026-07-21):** dropped `domain` from entity
  identity (Wikidata pattern). Schema bumped 3 → 4. Live store migrated
  34 → 24 entities (10 auto-merges). See ADR-0002.
```

- [ ] **Step 3: Write the shipped report**

Create `docs/reports/2026-07-21-entity-identity-reform-shipped.md`:

```markdown
# Entity Identity Reform — Shipped

> Date: 2026-07-21
> Status: **SHIPPED**
> Branch: `vnext/phase-0`
> Schema: 3 → 4

## What shipped

Dropped `domain` from the entity identity constraint. One real-world subject
now resolves to exactly one stable `entity_id`, regardless of how many domain
variants LLM extraction emits. This is the Wikidata identity pattern (opaque
ID + mutable categorization tags), the same pattern used by Wikidata,
MusicBrainz, OpenStreetMap, GitHub, and OpenAI Temporal Agents.

## Verified numbers

| Metric | Before | After |
|--------|--------|-------|
| Total entities | 34 | 24 |
| CATL entities | 6 | 1 |
| BYD entities | 2 | 1 |
| LG Energy entities | 2 | 1 |
| Sodium-ion (Naxtra) entities | 2 | 1 |
| Hungary overseas plant entities | 2 | 1 |
| Spain JV entities | 2 | 1 |
| Phantom galaxy nodes | 10 | 0 |
| Confirmed claims | 66 | 66 (0 data loss) |

## What changed

- `entities` UNIQUE: `(domain, canonical_subject)` → `(canonical_subject)`.
- `entity_aliases` PK: `(domain, alias, entity_id)` → `(alias, entity_id)`.
- `resolve_or_create_entity_in_tx`, `resolve_entity_in_tx`, `insert_alias`
  drop the `domain` parameter.
- `merge_entities` + `split_entities`: cross-domain guards removed.
- `claim_timeline`: `domain` is now an optional filter.
- `ClaimDraft.domain`, `ProposalSummary.domain`: `Option<String>`.
- `EntityRecord.domain`: removed.
- HTTP `GET /entity/timeline`: `domain` optional.
- MCP `brain_capture` + `brain_propose`: `domain` optional.
- Phase 1.6 conflict detection: bucket key `(domain, subject, predicate)` →
  `(subject, predicate)` (cross-domain conflicts now surface).

## Migration

- Schema version: 3 → 4.
- Migration: atomic, single-transaction (consolidate + constraint change).
- Rehearsed on a backup before touching the live store.
- Backup at `./backups/pre-identity-reform-<timestamp>/`.

## Tests

- New file `tests/semantic_identity_reform_v1.rs` — Wikidata-pattern
  regression suite (3 tests).
- `tests/semantic_migration_v1.rs` — 5 new v3→v4 tests (path recognition,
  plan body, forward consolidation, forward constraint, reverse).
- All existing tests updated for the new signatures.
- `cargo test --workspace` green.

## References

- ADR-0002
- `docs/plans/entity-identity-reform-design-doc.md` (research + design)
```

- [ ] **Step 4: Mark the design doc SHIPPED**

In `docs/plans/entity-identity-reform-design-doc.md`, change the header status line:

```markdown
> Status: **SHIPPED — 34 → 24 entities, 0 data loss (see docs/reports/2026-07-21-entity-identity-reform-shipped.md)**
```

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0002-entity-identity-reform-wikidata-pattern.md
git add BLUEPRINT.md
git add docs/reports/2026-07-21-entity-identity-reform-shipped.md
git add docs/plans/entity-identity-reform-design-doc.md
git commit -m "docs(identity): ADR-0002 + shipped report + design doc marked SHIPPED"
```

---

## Self-Review Notes (filled in after writing)

Spec coverage check against the design doc:

- §5.2 schema migration → Tasks 3, 4, 6.
- §6.1 SQLite schema (entities, entity_aliases, claim_status unchanged) → Tasks 4, 6.
- §6.2 core functions (3 signature changes) → Tasks 8, 9, 10, 11.
- §6.3 guards removed (merge, split, rename simplified) → Tasks 9, 10, 11.
- §6.4 public read APIs (claim_timeline) → Task 13.
- §6.5 HTTP endpoints → Task 15.
- §6.6 MCP tools → Tasks 16, 17.
- §6.7 type changes → Task 14.
- §6.8 test impact → Tasks 18, 19, 20.
- §7.1 Phase A migration → Tasks 3, 4, 5.
- §7.2 Phase B code → Tasks 6–17.
- §7.3 Phase C test + verify → Tasks 18–22.
- §10 DoD: covered by the shipped report (Task 23).

Placeholder scan: none — every step has actual code or an exact command.

Type consistency: `resolve_or_create_entity_in_tx(subject, event_seq)`, `resolve_entity_in_tx(alias)`, `insert_alias(alias, entity_id, kind, event_seq)` — signatures used identically across Tasks 8–11. `ClaimDraft.domain: Option<String>`, `ProposalSummary.domain: Option<String>`, `EntityRecord.domain: removed` — consistent across Tasks 7, 14, 15.
