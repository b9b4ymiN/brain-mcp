//! Task 2.2 — projection ownership (RED stage).
//!
//! Extends the Task 2.1 projection contract with the ownership dimensions
//! GOAL-vNext §13 Task 2.2 mandates:
//!
//! 1. Each generated page's frontmatter carries `origin` (`human-authored` or
//!    `agent-proposed`), `provenance` (the ADR Decision 6 variant name), and
//!    `entity_id` (the stable UUIDv7 the claim resolves to). This replaces
//!    Task 2.1's deferred `type: entity`-for-everything simplification with a
//!    `claim_kind → page type` mapping.
//! 2. Editing a file inside the generated wiki root is NOT a silent state
//!    mutation: the next `rebuild_projection` deletes and recreates the whole
//!    generated tree (it is `.projection-owned`, hence replaceable), and the
//!    hand-edit is discarded. Only the canonical semantic layers can move
//!    state; the projection is a materialized view (ADR Decision 1).
//!
//! Like `semantic_ownership_v1.rs`, this file targets the post-Task-2.2 API
//! (`ClaimView.origin`, `ClaimView.entity_id`, new frontmatter keys) and is
//! the intended RED state.

use std::path::{Path, PathBuf};

use chrono::Utc;
use llm_wiki::index_manager::SpaceIndexManager;
use llm_wiki::index_schema::IndexSchema;
use llm_wiki::projection::rebuild_projection;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, OriginClass, PrivacyLabel, ProposeCommand,
    SemanticConfig, SemanticStore, TrustedContext,
};
use llm_wiki::space_builder;
use llm_wiki::type_registry::SpaceTypeRegistry;
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

fn evidence_draft(subject: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: Some("stocks".to_owned()),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn preference_draft(subject: &str, value: &str) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "preference".to_owned(),
        value: json!(value),
        claim_kind: "preference".to_owned(),
        domain: Some("projects".to_owned()),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Evidence-backed confirm (Task 0.3 path).
fn confirm_evidence(
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
                draft: evidence_draft(subject, 58),
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

/// User-assertion confirm (Task 2.2 path) — the human-authored variant.
fn confirm_user_assertion(
    store: &SemanticStore,
    context: &TrustedContext,
    tag: &str,
    subject: &str,
    value: &str,
) -> String {
    store
        .propose_user_assertion(
            context,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: format!("assert-{tag}"),
                utterance: format!("utterance {tag}").into_bytes(),
                draft: preference_draft(subject, value),
            },
        )
        .unwrap();
    let confirm_op = format!("confirm-{tag}");
    store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: confirm_op.clone(),
                proposal_operation_id: format!("assert-{tag}"),
            },
        )
        .unwrap();
    confirm_op
}

fn index_setup(dir: &Path) -> (SpaceIndexManager, IndexSchema, SpaceTypeRegistry) {
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

/// Read the frontmatter of the first generated page under `generated_root`.
fn first_page_frontmatter(generated_root: &Path) -> serde_yaml::Value {
    let claims_dir = generated_root.join("claims");
    let mut entries: Vec<_> = std::fs::read_dir(&claims_dir)
        .unwrap_or_else(|_| panic!("claims dir exists at {}", claims_dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    entries.sort();
    let page = entries.into_iter().next().expect("at least one page");
    let body = std::fs::read_to_string(&page).unwrap();
    let frontmatter = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(fm, _)| fm)
        .unwrap_or_else(|| panic!("page {page:?} has frontmatter"));
    serde_yaml::from_str(frontmatter).expect("frontmatter parses as YAML")
}

fn yaml_str(value: &serde_yaml::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_default()
}

// =============================================================================
// DoD 1 — frontmatter carries origin / provenance / entity_id
// =============================================================================

#[test]
fn generated_page_carries_origin_provenance_and_entity_id_for_a_user_assertion() {
    let (parent, store, context) = fixture();
    confirm_user_assertion(&store, &context, "a", "project:brain", "docker-compose");

    let (mgr, schema, registry) = index_setup(parent.path());
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());

    rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("rebuild");

    let fm = first_page_frontmatter(&generated_root);
    assert_eq!(yaml_str(&fm, "origin"), "human-authored");
    assert_eq!(yaml_str(&fm, "provenance"), "user_assertion");
    // entity_id is present and non-empty (a UUIDv7 string).
    let entity_id = yaml_str(&fm, "entity_id");
    assert!(!entity_id.is_empty(), "entity_id must be populated");
    assert!(
        entity_id.len() >= 32,
        "entity_id looks like a UUID: {entity_id}"
    );
}

#[test]
fn generated_page_carries_agent_proposed_origin_for_an_evidence_backed_claim() {
    let (parent, store, context) = fixture();
    confirm_evidence(&store, &context, "ev", "GULF");

    let (mgr, schema, registry) = index_setup(parent.path());
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());

    rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("rebuild");

    let fm = first_page_frontmatter(&generated_root);
    assert_eq!(yaml_str(&fm, "origin"), "agent-proposed");
    assert_eq!(yaml_str(&fm, "provenance"), "evidence");
    assert!(!yaml_str(&fm, "entity_id").is_empty());
}

/// `claim_kind` drives the frontmatter `type` (closing Task 2.1's deferred
/// "every claim is type: entity" simplification). A `preference` claim must
/// render as a distinct page type, not as `entity`.
#[test]
fn generated_page_type_reflects_claim_kind_not_a_hardcoded_entity() {
    let (parent, store, context) = fixture();
    confirm_user_assertion(&store, &context, "pref", "project:brain", "docker-compose");

    let (mgr, schema, registry) = index_setup(parent.path());
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());

    rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("rebuild");

    let fm = first_page_frontmatter(&generated_root);
    let page_type = yaml_str(&fm, "type");
    assert_ne!(
        page_type, "entity",
        "preference claim must not collapse to the generic entity type"
    );
    assert!(
        !page_type.is_empty(),
        "type must be derived from claim_kind"
    );
}

/// `ClaimView` exposes `origin` and `entity_id` so projection adapters don't
/// have to re-derive them. The projection reads them directly.
#[test]
fn claim_view_exposes_origin_and_entity_id_fields() {
    let (_parent, store, context) = fixture();
    confirm_user_assertion(&store, &context, "x", "project:brain", "value");

    let view = store
        .all_claims_current(store.ledger_head().unwrap(), Utc::now())
        .unwrap();
    let claim = view.active.first().expect("one active claim");
    assert_eq!(claim.origin, OriginClass::HumanAuthored);
    assert!(
        claim.entity_id.is_some(),
        "ClaimView.entity_id must be populated for a confirmed claim"
    );
}

// =============================================================================
// DoD 2 — generated edits never silently mutate semantic state
// =============================================================================

/// Hand-editing a file inside the generated wiki root and then rebuilding must
/// NOT carry that edit forward: rebuild wipes and regenerates the whole tree
/// from canonical claims. The edit is discarded, not persisted as state. This
/// is the "generated edit ไม่เปลี่ยน state เงียบ" half of DoD bullet 2.
#[test]
fn hand_editing_a_generated_page_does_not_survive_the_next_rebuild() {
    let (parent, store, context) = fixture();
    confirm_evidence(&store, &context, "a", "GULF");

    let (mgr, schema, registry) = index_setup(parent.path());
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());

    let first = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("first rebuild");

    // Tamper with a generated page — this must NOT become semantic state.
    let claims_dir = generated_root.join("claims");
    let tampered = claims_dir.join("__tamper.md");
    std::fs::write(
        &tampered,
        "---\ntitle: \"INJECTED\"\n---\n\nmalicious hand edit",
    )
    .unwrap();

    let second = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("second rebuild");

    // The tampered page is gone — rebuild owns the directory.
    assert!(
        !tampered.exists(),
        "generated hand-edit must be wiped on rebuild, not persisted"
    );
    // Same canonical state ⇒ same composite checksum (determinism preserved).
    assert_eq!(
        first.checkpoint.composite_checksum,
        second.checkpoint.composite_checksum
    );
}

/// A rename that lands AFTER the first rebuild must be reflected in the next
/// rebuild's `entity_id` frontmatter (entity_id is stable; canonical_subject
/// changes). This ties DoD bullet 3 (rename preserves stable IDs) to the
/// projection: the page's `entity_id` stays constant across the rename, only
/// the `title` changes.
#[test]
fn entity_id_is_stable_across_a_rename_visible_in_projection() {
    let (parent, store, context) = fixture();
    confirm_user_assertion(&store, &context, "a", "GULF-old", "docker-compose");

    let (mgr, schema, registry) = index_setup(parent.path());
    let (repo_root, generated_root) = repo_and_generated_root(parent.path());

    let _first = rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("first rebuild");
    let fm_before = first_page_frontmatter(&generated_root);
    let entity_id_before = yaml_str(&fm_before, "entity_id");
    assert!(!entity_id_before.is_empty());

    // Resolve the entity, rename it, rebuild.
    let entity_id = store
        .resolve_entity(&context, "GULF-old")
        .expect("resolve");
    assert_eq!(entity_id.to_string(), entity_id_before);

    store
        .rename_entity(
            &context,
            llm_wiki::semantic::RenameEntityCommand {
                operation_id: "rename-proj".to_owned(),
                entity_id,
                new_subject: "GULF".to_owned(),
            },
        )
        .expect("rename");

    rebuild_projection(
        &store,
        Utc::now(),
        &generated_root,
        &repo_root,
        &mgr,
        &schema,
        &registry,
    )
    .expect("second rebuild");

    let fm_after = first_page_frontmatter(&generated_root);
    // Stable ID survives the rename.
    assert_eq!(yaml_str(&fm_after, "entity_id"), entity_id_before);
    // The title (canonical subject) reflects the new name.
    assert_eq!(yaml_str(&fm_after, "title"), "GULF");
}
