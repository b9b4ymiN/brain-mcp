//! Task 2.1: projection adapters. Proves that Tantivy, Petgraph, and
//! generated Markdown can be built from the canonical semantic layers
//! (event ledger + claim snapshots) alone, that deleting and rebuilding the
//! projection reproduces the same composite checksum, and that the
//! generated-wiki output never touches a human-authored wiki root.

use std::path::{Path, PathBuf};

use chrono::Utc;
use llm_wiki::index_manager::SpaceIndexManager;
use llm_wiki::projection::rebuild_projection;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, RetractCommand,
    SemanticConfig, SemanticStore, SupersedeCommand, TrustedContext,
};
use llm_wiki::space_builder;
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

fn draft(subject: &str, value: i64) -> ClaimDraft {
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

/// Capture -> propose -> confirm one claim under a unique operation-id tag.
/// Returns the confirm operation_id (needed to retract/supersede later).
fn confirm_claim(
    store: &SemanticStore,
    context: &TrustedContext,
    tag: &str,
    subject: &str,
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
                draft: draft(subject, 58),
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

fn index_setup(
    dir: &Path,
) -> (
    SpaceIndexManager,
    llm_wiki::index_schema::IndexSchema,
    llm_wiki::type_registry::SpaceTypeRegistry,
) {
    let (registry, schema) = space_builder::build_space_from_embedded("en_stem");
    let mgr = SpaceIndexManager::new("test", dir.join("index-store"));
    (mgr, schema, registry)
}

fn repo_and_generated_root(parent: &Path) -> (PathBuf, PathBuf) {
    let repo_root = parent.join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();
    let generated_root = repo_root.join("generated-wiki");
    (repo_root, generated_root)
}

// ── rebuild from canonical layers ───────────────────────────────────────────

#[test]
fn rebuild_projects_active_claims_into_index_and_graph() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");
    confirm_claim(&store, &context, "b", "PTT");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let outcome = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_eq!(outcome.index_report.pages_indexed, 2);
    assert_eq!(outcome.graph_node_count, 2);
    assert_eq!(outcome.checkpoint.ledger_head, store.ledger_head().unwrap());
    assert_eq!(outcome.checkpoint.claims_projected, 2);
    assert!(!outcome.checkpoint.composite_checksum.is_empty());
}

#[test]
fn rebuild_with_no_claims_produces_an_empty_projection() {
    let (parent, store, _context) = fixture();
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let outcome = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_eq!(outcome.index_report.pages_indexed, 0);
    assert_eq!(outcome.graph_node_count, 0);
    assert_eq!(outcome.checkpoint.claims_projected, 0);
}

// ── delete + rebuild determinism ────────────────────────────────────────────

#[test]
fn delete_then_rebuild_reproduces_the_same_composite_checksum() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");
    confirm_claim(&store, &context, "b", "PTT");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let first = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    // Delete every projection artifact: generated markdown and the search index.
    std::fs::remove_dir_all(&generated_root).unwrap();
    std::fs::remove_dir_all(mgr.index_path()).unwrap();

    let second = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_eq!(
        first.checkpoint.composite_checksum,
        second.checkpoint.composite_checksum
    );
    assert_eq!(first.checkpoint.ledger_head, second.checkpoint.ledger_head);
    assert_eq!(first.graph_node_count, second.graph_node_count);
}

#[test]
fn a_new_confirmed_claim_changes_the_composite_checksum_on_next_rebuild() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let first = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    confirm_claim(&store, &context, "b", "PTT");

    let second = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_ne!(
        first.checkpoint.composite_checksum,
        second.checkpoint.composite_checksum
    );
    assert!(second.checkpoint.ledger_head > first.checkpoint.ledger_head);
}

// ── superseded / retracted claims are excluded ──────────────────────────────

#[test]
fn superseded_claim_is_excluded_and_only_the_new_claim_is_projected() {
    let (parent, store, context) = fixture();
    let confirm_a = confirm_claim(&store, &context, "a", "GULF");

    store
        .capture(&context, capture("cap-b", b"revised evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft("GULF", 62),
            },
        )
        .unwrap();
    store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-a".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec![confirm_a],
            },
        )
        .unwrap();

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let outcome = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_eq!(outcome.checkpoint.claims_projected, 1);
    assert_eq!(outcome.graph_node_count, 1);
}

#[test]
fn retracted_claim_is_excluded_from_projection() {
    let (parent, store, context) = fixture();
    let confirm_a = confirm_claim(&store, &context, "a", "GULF");

    store
        .retract(
            &context,
            RetractCommand {
                operation_id: "retract-a".to_owned(),
                claim_operation_id: confirm_a,
            },
        )
        .unwrap();

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let outcome = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    assert_eq!(outcome.checkpoint.claims_projected, 0);
    assert_eq!(outcome.graph_node_count, 0);
}

// ── isolation from human-authored content ───────────────────────────────────

#[test]
fn generated_projection_never_touches_the_human_authored_wiki_root() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let human_wiki_root = repo_root.join("wiki");
    std::fs::create_dir_all(human_wiki_root.join("concepts")).unwrap();
    let human_page = human_wiki_root.join("concepts/moe.md");
    let human_content =
        "---\ntitle: \"MoE\"\ntype: concept\nstatus: active\n---\n\nHuman-authored.\n";
    std::fs::write(&human_page, human_content).unwrap();

    let (mgr, schema, registry) = index_setup(parent.path());

    rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();

    let after = std::fs::read_to_string(&human_page).unwrap();
    assert_eq!(after, human_content);
    assert!(!generated_root.starts_with(&human_wiki_root));
    assert!(!human_wiki_root.starts_with(&generated_root));
}

#[test]
fn rebuild_refuses_to_delete_a_directory_it_does_not_own() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    // Simulate a caller mistake: pointing rebuild_projection at a directory
    // that already has real content and no ownership marker.
    std::fs::create_dir_all(&generated_root).unwrap();
    let unmarked_page = generated_root.join("not-mine.md");
    let unmarked_content =
        "---\ntitle: \"Not Mine\"\ntype: concept\nstatus: active\n---\n\nPre-existing.\n";
    std::fs::write(&unmarked_page, unmarked_content).unwrap();

    let (mgr, schema, registry) = index_setup(parent.path());

    let result = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    );

    assert!(result.is_err());
    let after = std::fs::read_to_string(&unmarked_page).unwrap();
    assert_eq!(after, unmarked_content);
}

// ── checkpoint lag visibility ────────────────────────────────────────────────

#[test]
fn checkpoint_lag_is_zero_immediately_after_rebuild_and_positive_after_a_new_claim() {
    let (parent, store, context) = fixture();
    confirm_claim(&store, &context, "a", "GULF");

    let (repo_root, generated_root) = repo_and_generated_root(parent.path());
    let (mgr, schema, registry) = index_setup(parent.path());

    let outcome = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .unwrap();
    assert_eq!(
        llm_wiki::projection::checkpoint_lag(&outcome.checkpoint, &store).unwrap(),
        0
    );

    confirm_claim(&store, &context, "b", "PTT");
    assert!(llm_wiki::projection::checkpoint_lag(&outcome.checkpoint, &store).unwrap() > 0);
}
