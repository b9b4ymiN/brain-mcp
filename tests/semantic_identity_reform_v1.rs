//! Entity Identity Reform — Wikidata pattern regression tests.
//!
//! These tests encode the invariants established by the reform (see
//! `docs/plans/entity-identity-reform-design-doc.md` and ADR-0002):
//!
//! 1. One real-world subject → exactly one entity_id, regardless of domain.
//! 2. Two claims with the same subject but different domains resolve to the
//!    same entity.
//! 3. canonical_subject is UNIQUE on its own.
//! 4. Fresh store has no entities.domain / entity_aliases.domain column.
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
/// entities (the CATL fragmentation bug: 1 company → 6 entities).
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
    let (_parent, store, ctx) = fixture();
    // Create one entity via the public API so the row exists.
    let _ = store.resolve_or_create_entity(&ctx, "UNIQUE_TEST");
    drop(store);

    let db_path = _parent.path().join("semantic-store").join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open");
    // A second entity with the same canonical_subject must fail.
    let dup = conn.execute(
        "INSERT INTO entities(entity_id, canonical_subject, created_at) \
         VALUES ('deadbeef-0000-7000-8000-000000000099', 'UNIQUE_TEST', '2026-07-01T00:00:00Z')",
        [],
    );
    assert!(
        dup.is_err(),
        "second insert with same canonical_subject must fail (UNIQUE constraint)"
    );
}

/// Fresh store has no `domain` column on entities or entity_aliases. This
/// catches a regression where someone re-adds the column to the fresh-store
/// DDL.
#[test]
fn fresh_store_has_no_domain_column_on_entity_tables() {
    use rusqlite::Connection;
    let (_parent, store, _ctx) = fixture();
    drop(store);

    let db_path = _parent.path().join("semantic-store").join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open");

    let entity_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(entities)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        !entity_cols.iter().any(|c| c == "domain"),
        "entities must not have a domain column post-reform; cols = {entity_cols:?}"
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

/// claim_status STILL has a `domain` column — it is a per-claim categorization
/// tag, NOT entity identity. This test guards against someone accidentally
/// dropping it from claim_status.
#[test]
fn claim_status_keeps_domain_as_per_claim_tag() {
    use rusqlite::Connection;
    let (_parent, store, ctx) = fixture();

    // Confirm a claim with a domain tag so the row exists.
    store
        .propose_user_assertion(
            &ctx,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "a-tagged".to_owned(),
                utterance: b"t".to_vec(),
                draft: draft("GULF", "price", 48, "stocks"),
            },
        )
        .expect("propose");
    confirm(&store, &ctx, "c-tagged", "a-tagged");
    drop(store);

    let db_path = _parent.path().join("semantic-store").join("semantic.sqlite3");
    let conn = Connection::open(&db_path).expect("open");
    let has_domain: bool = conn
        .prepare("PRAGMA table_info(claim_status)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .any(|col: String| col == "domain");
    assert!(
        has_domain,
        "claim_status.domain must remain as a per-claim tag"
    );

    // And the tag value is preserved.
    let stored_domain: String = conn
        .query_row(
            "SELECT domain FROM claim_status WHERE subject='GULF' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("query domain tag");
    assert_eq!(stored_domain, "stocks", "domain tag must be preserved on the claim");
}
