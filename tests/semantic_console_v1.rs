//! Phase E Task E0.1 — Console-facing read accessors + owner-scoped
//! confirm/reject/supersede on `SemanticStore` (GOAL-vNext §13 Task 5.1
//! inbox/timeline/evidence; §9 Console must never write storage directly).

use std::path::{Path, PathBuf};

use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmByProposalIdCommand, ConfirmCommand, PrivacyLabel,
    ProposeCommand, ProposeMechanicalCommand, RejectByProposalIdCommand, RejectCommand,
    SemanticConfig, SemanticStore, SupersedeByProposalIdCommand, SupersedeCommand, TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

fn fixture() -> (TempDir, PathBuf, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, root, store, context)
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
        domain: Some("stocks".to_owned()),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

#[test]
fn list_pending_proposals_excludes_confirmed_and_rejected() {
    let (_parent, _root, store, context) = fixture();
    store.capture(&context, capture("cap1", b"ev1")).unwrap();
    store.capture(&context, capture("cap2", b"ev2")).unwrap();
    store.capture(&context, capture("cap3", b"ev3")).unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p2".to_owned(),
                capture_operation_id: "cap2".to_owned(),
                draft: stock_draft("PTT", 70),
            },
        )
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p3".to_owned(),
                capture_operation_id: "cap3".to_owned(),
                draft: stock_draft("AOT", 90),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "c1".to_owned(),
                proposal_operation_id: "p1".to_owned(),
            },
        )
        .unwrap();
    store
        .reject(
            &context,
            RejectCommand {
                operation_id: "r1".to_owned(),
                proposal_operation_id: "p2".to_owned(),
            },
        )
        .unwrap();

    let pending = store.list_pending_proposals().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].subject, "AOT");
    assert_eq!(pending[0].predicate, "target_price");
    assert_eq!(pending[0].value, json!(90));
    assert_eq!(pending[0].provenance_kind, "evidence");
}

#[test]
fn claim_timeline_returns_chronological_history_for_scope() {
    let (_parent, _root, store, context) = fixture();
    store.capture(&context, capture("cap1", b"ev1")).unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "c1".to_owned(),
                proposal_operation_id: "p1".to_owned(),
            },
        )
        .unwrap();

    store.capture(&context, capture("cap2", b"ev2")).unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p2".to_owned(),
                capture_operation_id: "cap2".to_owned(),
                draft: stock_draft("GULF", 62),
            },
        )
        .unwrap();
    store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "c2".to_owned(),
                proposal_operation_id: "p2".to_owned(),
                superseded_claim_operation_ids: vec!["c1".to_owned()],
            },
        )
        .unwrap();

    let timeline = store
        .claim_timeline(Some("stocks"), "GULF", "target_price")
        .unwrap();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0].value, json!(58));
    assert_eq!(timeline[1].value, json!(62));
    assert!(timeline[0].confirmed_event_seq < timeline[1].confirmed_event_seq);
}

#[test]
fn evidence_for_returns_excerpt_for_evidence_provenance() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap1", b"GULF target price raised to 58"))
        .unwrap();
    let outcome = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    let proposal_id = outcome.generated.proposal_id.unwrap();

    let evidence = store.evidence_for(proposal_id).unwrap();
    assert_eq!(evidence.provenance_kind, "evidence");
    assert_eq!(
        evidence.excerpt.as_deref(),
        Some("GULF target price raised to 58")
    );
}

#[test]
fn evidence_for_mechanical_provenance_has_hash_but_no_excerpt() {
    let (_parent, _root, store, context) = fixture();
    let outcome = store
        .propose_mechanical(
            &context,
            ProposeMechanicalCommand {
                operation_id: "p1".to_owned(),
                method: "hash-check".to_owned(),
                method_version: "1".to_owned(),
                input_hashes: vec!["sha256:input".to_owned()],
                output_hash: "sha256:output".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    let proposal_id = outcome.generated.proposal_id.unwrap();

    let evidence = store.evidence_for(proposal_id).unwrap();
    assert_eq!(evidence.provenance_kind, "mechanical");
    assert_eq!(evidence.excerpt, None);
    assert_eq!(evidence.quote_hash.as_deref(), Some("sha256:output"));
}

#[test]
fn confirm_by_proposal_id_allows_a_different_client_to_approve() {
    let (_parent, _root, store, proposer) = fixture();
    let reviewer = store.register_client("console-reviewer").unwrap();

    store.capture(&proposer, capture("cap1", b"ev1")).unwrap();
    let proposed = store
        .propose(
            &proposer,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    let proposal_id = proposed.generated.proposal_id.unwrap();

    let outcome = store
        .confirm_by_proposal_id(
            &reviewer,
            ConfirmByProposalIdCommand {
                operation_id: "reviewer-confirm".to_owned(),
                proposal_id,
            },
        )
        .unwrap();

    assert_eq!(outcome.event.actor_id, reviewer.actor_id());
    assert_eq!(outcome.event.client_id, reviewer.client_id());
    assert!(outcome.generated.claim_id.is_some());

    let pending = store.list_pending_proposals().unwrap();
    assert!(pending.is_empty());
}

#[test]
fn reject_by_proposal_id_allows_a_different_client_to_reject() {
    let (_parent, _root, store, proposer) = fixture();
    let reviewer = store.register_client("console-reviewer").unwrap();

    store.capture(&proposer, capture("cap1", b"ev1")).unwrap();
    let proposed = store
        .propose(
            &proposer,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    let proposal_id = proposed.generated.proposal_id.unwrap();

    let outcome = store
        .reject_by_proposal_id(
            &reviewer,
            RejectByProposalIdCommand {
                operation_id: "reviewer-reject".to_owned(),
                proposal_id,
            },
        )
        .unwrap();

    assert_eq!(outcome.event.client_id, reviewer.client_id());
    assert_ne!(outcome.event.client_id, proposer.client_id());

    let pending = store.list_pending_proposals().unwrap();
    assert!(pending.is_empty());
}

#[test]
fn supersede_by_proposal_id_allows_a_different_client_to_supersede() {
    let (_parent, _root, store, proposer) = fixture();
    let reviewer = store.register_client("console-reviewer").unwrap();

    store.capture(&proposer, capture("cap1", b"ev1")).unwrap();
    let first = store
        .propose(
            &proposer,
            ProposeCommand {
                operation_id: "p1".to_owned(),
                capture_operation_id: "cap1".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    let first_claim = store
        .confirm_by_proposal_id(
            &reviewer,
            ConfirmByProposalIdCommand {
                operation_id: "reviewer-confirm-1".to_owned(),
                proposal_id: first.generated.proposal_id.unwrap(),
            },
        )
        .unwrap();
    let first_claim_id = first_claim.generated.claim_id.unwrap();

    store.capture(&proposer, capture("cap2", b"ev2")).unwrap();
    let second = store
        .propose(
            &proposer,
            ProposeCommand {
                operation_id: "p2".to_owned(),
                capture_operation_id: "cap2".to_owned(),
                draft: stock_draft("GULF", 62),
            },
        )
        .unwrap();

    let outcome = store
        .supersede_by_proposal_id(
            &reviewer,
            SupersedeByProposalIdCommand {
                operation_id: "reviewer-supersede".to_owned(),
                proposal_id: second.generated.proposal_id.unwrap(),
                superseded_claim_ids: vec![first_claim_id],
            },
        )
        .unwrap();

    assert!(outcome.generated.claim_id.is_some());
    assert_eq!(outcome.event.client_id, reviewer.client_id());
    assert_ne!(outcome.event.client_id, proposer.client_id());
    let timeline = store
        .claim_timeline(Some("stocks"), "GULF", "target_price")
        .unwrap();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0].claim_id, first_claim_id);
}
