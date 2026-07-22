//! Task 2.3 — Backfill and cutover (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 2.3 DoD:
//! - migration report แสดง migrated/skipped/ambiguous/error ทุก record
//! - LLM-backfilled claims เริ่มเป็น proposed ไม่ใช่ confirmed
//! - old/new read parity ผ่าน acceptance corpus ก่อน cutover
//! - rollback และ rerun migration idempotent
//!
//! Task 2.2 left `claim_status.entity_id` nullable precisely so a legacy
//! store (or a row that lost its entity binding) could be backfilled without
//! a destructive migration. This file proves that backfill path.

use std::fs;
use std::path::Path;

use chrono::Utc;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticError, SemanticStore, TrustedContext,
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

fn capture(operation_id: &str, bytes: &[u8]) -> CaptureCommand {
    CaptureCommand {
        operation_id: operation_id.to_owned(),
        bytes: bytes.to_vec(),
        media_type: "text/plain; charset=utf-8".to_owned(),
    }
}

fn stock_draft(subject: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Confirm one evidence-backed claim, returning the confirm operation id.
fn confirm_evidence(
    store: &SemanticStore,
    context: &TrustedContext,
    tag: &str,
    subject: &str,
    value: i64,
) -> String {
    store
        .capture(
            context,
            capture(&format!("cap-{tag}"), format!("evidence {tag}").as_bytes()),
        )
        .unwrap();
    store
        .propose(
            context,
            ProposeCommand {
                operation_id: format!("prop-{tag}"),
                capture_operation_id: format!("cap-{tag}"),
                draft: stock_draft(subject, value),
            },
        )
        .unwrap();
    let confirm_op = format!("confirm-{tag}");
    store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: confirm_op.clone(),
                proposal_operation_id: format!("prop-{tag}"),
            },
        )
        .unwrap();
    confirm_op
}

// =============================================================================
// DoD bullet 1 — migration report classifies every record
// =============================================================================

/// A dry run over a store with two confirmed claims (both already bound to
/// entities by Task 2.2's confirm path) reports zero migrations needed and
/// classifies both rows as `skipped` (already have an entity_id). Nothing is
/// mutated.
#[test]
fn dry_run_classifies_already_bound_rows_as_skipped() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);
    confirm_evidence(&store, &context, "b", "PTT", 160);

    let report = store
        .backfill_entity_ids(&context, true)
        .expect("dry run backfill");

    assert_eq!(report.migrated, 0, "dry run must not migrate");
    assert_eq!(report.skipped, 2, "both rows already have entity_id");
    assert_eq!(report.ambiguous, 0);
    assert_eq!(report.error, 0);
    assert_eq!(report.records.len(), 2);
    for record in &report.records {
        assert_eq!(
            record.outcome, "skipped",
            "every already-bound row is skipped"
        );
    }
}

/// A store with one legacy row whose `entity_id` is NULL (simulated by
/// clearing it) is migrated on apply, and the report records it as
/// `migrated`. The row ends up with a non-null entity_id afterwards.
#[test]
fn apply_migrates_legacy_null_entity_id_rows() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);

    // Simulate a legacy row by nulling its entity_id directly.
    store.null_entity_id_for_test("stocks", "GULF", "target_price");

    let report = store
        .backfill_entity_ids(&context, false)
        .expect("apply backfill");

    assert_eq!(report.migrated, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.error, 0);
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].outcome, "migrated");

    // The row now resolves to a real entity.
    let view = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(view.active.len(), 1);
    assert!(
        view.active[0].entity_id.is_some(),
        "entity_id must be populated after backfill"
    );
}

/// A row whose subject cannot be resolved to any entity (corrupt/manual
/// insert with no alias) is classified as `ambiguous`, not `error` — the
/// migration does not guess, and does not abort the whole run.
#[test]
fn unresolvable_row_is_ambiguous_not_error() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);

    // Insert a second claim_status row with a subject that has no entity and
    // no alias, simulating an orphan from a partial import.
    store.insert_orphan_claim_status_for_test("stocks", "GHOST", "target_price");

    let report = store
        .backfill_entity_ids(&context, false)
        .expect("apply backfill");

    let ghost = report
        .records
        .iter()
        .find(|r| r.subject == "GHOST")
        .expect("ghost row reported");
    assert_eq!(ghost.outcome, "ambiguous");
    // The legitimate row still migrated/skipped fine — one bad row doesn't
    // abort the batch.
    assert_eq!(report.error, 0);
}

// =============================================================================
// DoD bullet 2 — LLM-backfilled claims start as proposed
// =============================================================================

/// The backfill path only populates `entity_id` on ALREADY-confirmed claims;
/// it never creates new confirmed claims. New LLM-derived claims must still
/// go through `propose_inference`, which emits `claim_proposed` (status
/// `proposed`), never `claim_confirmed`. This is the §5 Memory Policy rule
/// "AI inference/reflection → Proposed" enforced at the API surface, and
/// backfill does not bypass it.
#[test]
fn backfill_never_promotes_an_llm_claim_to_confirmed() {
    let (_parent, store, context) = fixture();

    // An LLM worker proposes an inference (no evidence → unsupported).
    let outcome = store
        .propose_inference(
            &context,
            llm_wiki::semantic::ProposeInferenceCommand {
                operation_id: "llm-1".to_owned(),
                evidence_capture_operation_ids: vec![],
                method: "llm_extract".to_owned(),
                model: Some("zai-glm".to_owned()),
                prompt_version: Some("v1".to_owned()),
                subject_validator_version: None,
                draft: stock_draft("GULF", 60),
            },
        )
        .expect("propose_inference");

    assert_eq!(outcome.event.event_type, "claim_proposed");

    // The unsupported inference cannot be confirmed (TM-002 gate).
    assert!(matches!(
        store.confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-llm".to_owned(),
                proposal_operation_id: "llm-1".to_owned(),
            },
        ),
        Err(SemanticError::UnsupportedInference)
    ));

    // Backfill operates only on confirmed claims — the unconfirmed LLM
    // proposal is not in claim_status at all, so it is invisible to
    // backfill. Nothing gets promoted.
    let report = store
        .backfill_entity_ids(&context, false)
        .expect("backfill");
    assert_eq!(report.migrated, 0);
    assert_eq!(report.skipped, 0);
}

// =============================================================================
// DoD bullet 3 — old/new read parity
// =============================================================================

/// Read parity: the claim set returned by `all_claims_current` before and
/// after a backfill is identical in every field EXCEPT the newly-populated
/// `entity_id`. The bitemporal bucketing, value, subject, predicate, and
/// provenance are unchanged — backfill is a metadata fix, not a semantic
/// mutation.
#[test]
fn read_parity_holds_across_backfill() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);
    confirm_evidence(&store, &context, "b", "PTT", 160);

    let before = store
        .all_claims_current(store.ledger_head().unwrap(), Utc::now())
        .unwrap();

    // Wipe entity bindings to force a real migration.
    store.null_all_entity_ids_for_test();
    let report = store.backfill_entity_ids(&context, false).unwrap();
    assert_eq!(report.migrated, 2);

    let after = store
        .all_claims_current(store.ledger_head().unwrap(), Utc::now())
        .unwrap();

    assert_eq!(before.active.len(), after.active.len());
    for (b, a) in before.active.iter().zip(after.active.iter()) {
        assert_eq!(b.claim_id, a.claim_id);
        assert_eq!(b.subject, a.subject, "subject unchanged");
        assert_eq!(b.predicate, a.predicate);
        assert_eq!(b.value, a.value, "value unchanged");
        assert_eq!(b.claim_kind, a.claim_kind);
        assert_eq!(b.provenance_kind, a.provenance_kind);
        assert_eq!(b.origin, a.origin);
        // entity_id was None before (nulled) and Some after (migrated).
        assert!(b.entity_id.is_none() || b.entity_id == a.entity_id);
        assert!(a.entity_id.is_some(), "entity_id populated after backfill");
    }
}

// =============================================================================
// DoD bullet 4 — rollback + rerun idempotent
// =============================================================================

/// Rerunning backfill is idempotent: a second run finds every row already
/// bound and reports them all as `skipped`, with zero new migrations.
#[test]
fn rerun_backfill_is_idempotent() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);
    store.null_entity_id_for_test("stocks", "GULF", "target_price");

    let first = store.backfill_entity_ids(&context, false).unwrap();
    assert_eq!(first.migrated, 1);

    let second = store.backfill_entity_ids(&context, false).unwrap();
    assert_eq!(second.migrated, 0, "second run migrates nothing");
    assert_eq!(second.skipped, 1, "the row is now skipped");
}

/// Rollback: clearing the entity bindings back to NULL (simulating a revert
/// of the migration) does not corrupt the store — claims remain queryable by
/// subject string, and a fresh backfill restores the bindings. The claim
/// payload (subject/predicate/value/provenance) is never touched by
/// backfill, so reverting only the entity_id column is lossless.
#[test]
fn rollback_to_null_bindings_is_lossless_and_recoverable() {
    let (_parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF", 58);

    store.backfill_entity_ids(&context, false).unwrap();
    let populated = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert!(populated.active[0].entity_id.is_some());

    // "Roll back" the migration by nulling bindings again.
    store.null_entity_id_for_test("stocks", "GULF", "target_price");
    let rolled_back = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    // Claim is still fully readable — only entity_id is None.
    assert_eq!(rolled_back.active[0].subject, "GULF");
    assert_eq!(rolled_back.active[0].value, json!(58));
    assert!(rolled_back.active[0].entity_id.is_none());

    // Re-running backfill recovers the binding.
    store.backfill_entity_ids(&context, false).unwrap();
    let recovered = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert!(recovered.active[0].entity_id.is_some());
    // And the entity_id is stable across rollback/recover (same entity).
    assert_eq!(populated.active[0].entity_id, recovered.active[0].entity_id);
}

// =============================================================================
// Phase Reform Task 1 — v3→v4 upgrade-path predicate
// =============================================================================

/// Reads the schema_version field out of `<root>/store.marker.json`. Mirrors
/// the helper in `tests/recovery_integration_v1.rs` so this file's v3→v4 tests
/// can stage a v3 store independently of the fresh-store DDL version (which
/// bumps to v4 in Task 6). Without staging, a test that calls
/// `plan_schema_upgrade(3, 4)` against a fresh store would pass today only
/// because `CURRENT_DISK_SCHEMA_VERSION == 3`, and break the moment Task 6
/// bumps it — with a confusing from-mismatch error rather than a real failure.
fn read_marker_schema_version(root: &Path) -> u8 {
    let bytes = fs::read(root.join("store.marker.json")).expect("read marker");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("marker parses");
    value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .expect("marker has schema_version") as u8
}

/// Rewrites the on-disk marker's `schema_version` to `to`. The in-memory
/// `SemanticStore` instance is unaffected (its `marker` was read on open);
/// the next `open_for_upgrade()` picks up the rewritten marker. Used to stage
/// a v3 store for the v3→v4 upgrade tests — the marker is rewritten AFTER the
/// store is dropped so the SQLite file is not contended.
fn rewrite_marker_schema_version(root: &Path, to: u8) {
    let path = root.join("store.marker.json");
    let bytes = fs::read(&path).expect("read marker");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("marker parses");
    value["schema_version"] = json!(to);
    let rewritten = serde_json::to_vec(&value).expect("serialize marker");
    fs::write(&path, rewritten).expect("rewrite marker");
}

/// Stage a v3 store: `fixture()` creates a fresh store (at whatever
/// `CURRENT_DISK_SCHEMA_VERSION` is), close it, then rewrite its on-disk
/// marker to schema_version=3. Returns the parent tempdir + the store root so
/// the test can `open_for_upgrade()` against the staged v3 state. Pinning the
/// marker to 3 here (rather than relying on the fresh-store version) keeps the
/// v3→v4 tests truthful regardless of the constant's value.
fn fixture_at_v3() -> (TempDir, std::path::PathBuf) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (_store, _admin) =
        SemanticStore::create(&root, enabled(parent.path())).expect("create");
    // `_store` dropped here — closes the connection so the marker file is
    // not contended when we rewrite it.
    drop(_store);
    rewrite_marker_schema_version(&root, 3);
    assert_eq!(
        read_marker_schema_version(&root),
        3,
        "test setup: marker staged at v3"
    );
    (parent, root)
}

/// Phase Reform Task 1: the v3→v4 upgrade path is recognized as a known
/// migration route. Pre-Task-1 this returned `unsupported schema upgrade
/// path` because only `(2, 3)` was in the predicate's match; the `(3, 4)` arm
/// now makes `plan_schema_upgrade(3, 4)` succeed. The store is staged at v3
/// via [`fixture_at_v3`] so this test asserts what its name claims and
/// survives the Task 6 bump of `CURRENT_DISK_SCHEMA_VERSION` to 4.
#[test]
fn v3_to_v4_upgrade_path_is_known() {
    let (_parent, root) = fixture_at_v3();
    let store =
        SemanticStore::open_for_upgrade(&root, enabled(_parent.path())).expect("open staged v3");
    assert_eq!(store.schema_version(), 3, "live marker is at v3 pre-upgrade");

    let plan = store.plan_schema_upgrade(3, 4).expect("plan v3→v4");
    assert!(plan.is_reversible(), "v3→v4 plan must be reversible");
    assert_eq!(plan.from_version, 3);
    assert_eq!(plan.to_version, 4);
    assert_eq!(plan.steps.len(), 1, "v3→v4 is one step");
}

/// Phase Reform Task 2: planning a v3→v4 upgrade yields a reversible plan
/// whose step description names the entity consolidation. The plan is the
/// audit-trail contract an operator reads before `execute_schema_upgrade`;
/// it must advertise reversibility or `execute_schema_upgrade` will refuse it.
#[test]
fn v3_to_v4_plan_is_reversible_and_names_consolidation() {
    let (_parent, root) = fixture_at_v3();
    let store =
        SemanticStore::open_for_upgrade(&root, enabled(_parent.path())).expect("open staged v3");
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

/// Phase Reform Task 3: forward v3→v4 migration collapses two entities that
/// share a canonical_subject onto one (most-claims-wins target). We construct
/// the legacy fragmented state by hand (insert two rows with the same
/// canonical_subject but different domains), run execute_schema_upgrade, and
/// assert they collapse to one.
#[test]
fn v3_to_v4_forward_consolidates_fragmented_entities() {
    use rusqlite::Connection;

    let (_parent, root) = fixture_at_v3();
    // Construct legacy fragmentation directly: two entities with the same
    // canonical_subject but different domains (the pre-reform invariant).
    // We use a second connection to the same SQLite file. The store's own
    // connection must be dropped first to avoid locking — fixture_at_v3
    // already drops the store before returning, so the file is free.
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
    let store = SemanticStore::open_for_upgrade(&root, enabled(_parent.path()))
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
