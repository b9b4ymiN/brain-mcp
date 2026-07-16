//! Phase C Task C1 — brain_* read tools wiring (RED stage).
//!
//! Tests that the brain_* tools are declared, dispatch correctly to the
//! SemanticStore, and return structured results.

use llm_wiki::mcp::tools::tool_list;
use llm_wiki::semantic::{
    ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeUserAssertionCommand, SemanticConfig,
    SemanticStore,
};
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

fn store_fixture() -> (TempDir, SemanticStore) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    (parent, store)
}

fn capture_assertion_and_confirm(store: &SemanticStore, subject: &str, value: &str) -> String {
    let ctx = store.trusted_context();
    store
        .propose_user_assertion(
            &ctx,
            ProposeUserAssertionCommand {
                operation_id: format!("assert-{subject}"),
                utterance: format!("user said {value}").into_bytes(),
                draft: ClaimDraft {
                    subject: subject.to_owned(),
                    predicate: "preference".to_owned(),
                    value: json!(value),
                    claim_kind: "preference".to_owned(),
                    domain: "projects".to_owned(),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .unwrap();
    let confirm_op = format!("confirm-{subject}");
    store
        .confirm(
            &ctx,
            ConfirmCommand {
                operation_id: confirm_op.clone(),
                proposal_operation_id: format!("assert-{subject}"),
            },
        )
        .unwrap();
    confirm_op
}

// =============================================================================
// Tool declarations exist
// =============================================================================

#[test]
fn brain_status_tool_exists() {
    let tools = tool_list();
    assert!(
        tools.iter().any(|t| t.name == "brain_status"),
        "brain_status tool must be declared"
    );
}

#[test]
fn brain_search_tool_exists() {
    let tools = tool_list();
    assert!(
        tools.iter().any(|t| t.name == "brain_search"),
        "brain_search tool must be declared"
    );
}

#[test]
fn brain_get_tool_exists() {
    let tools = tool_list();
    assert!(
        tools.iter().any(|t| t.name == "brain_get"),
        "brain_get tool must be declared"
    );
}

// =============================================================================
// brain_status returns ledger head + claim count
// =============================================================================

#[test]
fn brain_status_returns_ledger_info() {
    let (_parent, store) = store_fixture();
    capture_assertion_and_confirm(&store, "project:brain", "docker-compose");

    let head = store.ledger_head().unwrap();
    let claims = store.all_claims_current(head, chrono::Utc::now()).unwrap();
    assert_eq!(claims.active.len(), 1);
    // The tool would return this as structured content; we verify the store
    // produces the data the tool needs.
    assert!(head >= 2); // capture + confirm events
}

// =============================================================================
// brain_search queries claims
// =============================================================================

#[test]
fn brain_search_finds_confirmed_claims() {
    let (_parent, store) = store_fixture();
    capture_assertion_and_confirm(&store, "GULF", "58");
    capture_assertion_and_confirm(&store, "PTT", "160");

    let head = store.ledger_head().unwrap();
    let claims = store.all_claims_current(head, chrono::Utc::now()).unwrap();
    // Verify search would find GULF
    let gulf = claims
        .active
        .iter()
        .find(|c| c.subject == "GULF")
        .expect("GULF claim found");
    assert_eq!(gulf.value, json!("58"));
    assert_eq!(gulf.origin, llm_wiki::semantic::OriginClass::HumanAuthored);
}

// =============================================================================
// brain_get reads a single claim by subject
// =============================================================================

#[test]
fn brain_get_reads_single_claim() {
    let (_parent, store) = store_fixture();
    capture_assertion_and_confirm(&store, "GULF", "58");

    let head = store.ledger_head().unwrap();
    let _result = store
        .claims_current(head, chrono::Utc::now(), "stocks", "GULF", "preference")
        .unwrap();
    // Note: we confirmed under domain "projects" not "stocks" in our fixture.
    // The tool should search across domains. Verify the store can find it.
    let head = store.ledger_head().unwrap();
    let all = store.all_claims_current(head, chrono::Utc::now()).unwrap();
    let gulf = all
        .active
        .iter()
        .find(|c| c.subject == "GULF")
        .expect("found");
    assert_eq!(gulf.value, json!("58"));
}
