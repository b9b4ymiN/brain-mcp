use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand,
    ProposeInferenceCommand, RejectCommand, RetractCommand, SemanticConfig, SemanticError,
    SemanticStore, SupersedeCommand, TrustedContext,
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

fn stock_draft(
    subject: &str,
    value: i64,
    valid_from: Option<DateTime<Utc>>,
    valid_to: Option<DateTime<Utc>>,
) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from,
        valid_to,
    }
}

fn decision_draft() -> ClaimDraft {
    ClaimDraft {
        subject: "project:brain".to_owned(),
        predicate: "deployment".to_owned(),
        value: json!("docker-compose"),
        claim_kind: "decision".to_owned(),
        domain: "projects".to_owned(),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn at(value: &str) -> DateTime<Utc> {
    value.parse().expect("RFC 3339 test timestamp")
}

#[test]
fn reject_transitions_a_proposal_without_creating_a_claim() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();

    store
        .reject(
            &context,
            RejectCommand {
                operation_id: "rej".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    let diagnostics = store.diagnostics().unwrap();
    assert_eq!(diagnostics.events, 3);

    assert!(matches!(
        store.confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-after-reject".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));
    assert_eq!(store.diagnostics().unwrap().events, 3);
}

#[test]
fn double_confirm_of_same_proposal_is_rejected_as_invalid_transition() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-one".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    assert!(matches!(
        store.confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-two".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));
    assert_eq!(store.diagnostics().unwrap().events, 3);

    assert!(matches!(
        store.reject(
            &context,
            RejectCommand {
                operation_id: "reject-confirmed".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));
}

#[test]
fn retract_marks_a_confirmed_claim_as_no_longer_current() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    let before = store
        .claims_current(store.ledger_head().unwrap(), Utc::now(), "stocks", "GULF", "target_price")
        .unwrap();
    assert_eq!(before.active.len(), 1);

    store
        .retract(
            &context,
            RetractCommand {
                operation_id: "retract".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        )
        .unwrap();

    let after = store
        .claims_current(store.ledger_head().unwrap(), Utc::now(), "stocks", "GULF", "target_price")
        .unwrap();
    assert_eq!(after.active.len(), 0);
    assert_eq!(after.past.len(), 1);
    assert_eq!(after.past[0].status, "confirmed");
}

#[test]
fn retract_of_unknown_or_already_retracted_claim_is_rejected() {
    let (_parent, _root, store, context) = fixture();
    assert!(matches!(
        store.retract(
            &context,
            RetractCommand {
                operation_id: "retract-unknown".to_owned(),
                claim_operation_id: "never-confirmed".to_owned(),
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));

    store
        .capture(&context, capture("cap", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
    store
        .retract(
            &context,
            RetractCommand {
                operation_id: "retract-once".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        )
        .unwrap();
    assert!(matches!(
        store.retract(
            &context,
            RetractCommand {
                operation_id: "retract-twice".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));
}

#[test]
fn supersede_replaces_the_prior_claim_and_as_of_ledger_head_ignores_later_supersession() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-a", b"fair value 58"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-a".to_owned(),
                proposal_operation_id: "prop-a".to_owned(),
            },
        )
        .unwrap();
    let head_after_a = store.ledger_head().unwrap();

    store
        .capture(&context, capture("cap-b", b"fair value 62"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: stock_draft("GULF", 62, None, None),
            },
        )
        .unwrap();
    store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-b".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-a".to_owned()],
            },
        )
        .unwrap();
    let head_after_b = store.ledger_head().unwrap();

    let now_view = store
        .claims_current(head_after_b, Utc::now(), "stocks", "GULF", "target_price")
        .unwrap();
    assert_eq!(now_view.active.len(), 1);
    assert_eq!(now_view.active[0].value, json!(62));
    assert_eq!(now_view.past.len(), 1);
    assert_eq!(now_view.past[0].value, json!(58));

    let as_of_a = store
        .claims_current(head_after_a, Utc::now(), "stocks", "GULF", "target_price")
        .unwrap();
    assert_eq!(as_of_a.active.len(), 1);
    assert_eq!(as_of_a.active[0].value, json!(58));
    assert_eq!(as_of_a.past.len(), 0);
}

#[test]
fn supersede_across_mismatched_scope_is_rejected() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-a", b"fair value 58"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-a".to_owned(),
                proposal_operation_id: "prop-a".to_owned(),
            },
        )
        .unwrap();

    store
        .capture(&context, capture("cap-b", b"different subject"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: stock_draft("PTT", 40, None, None),
            },
        )
        .unwrap();

    assert!(matches!(
        store.supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-mismatch".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-a".to_owned()],
            },
        ),
        Err(SemanticError::InvalidTransition(_))
    ));
    assert_eq!(store.diagnostics().unwrap().events, 4);
}

#[test]
fn supersede_requires_at_least_one_prior_claim() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"fair value 58"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58, None, None),
            },
        )
        .unwrap();
    assert!(matches!(
        store.supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-empty".to_owned(),
                proposal_operation_id: "prop".to_owned(),
                superseded_claim_operation_ids: Vec::new(),
            },
        ),
        Err(SemanticError::InvalidClaim(_))
    ));
}

#[test]
fn disputed_external_facts_coexist_without_superseding() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-broker-a", b"broker a: 48"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-broker-a".to_owned(),
                capture_operation_id: "cap-broker-a".to_owned(),
                draft: stock_draft("GULF", 48, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-broker-a".to_owned(),
                proposal_operation_id: "prop-broker-a".to_owned(),
            },
        )
        .unwrap();

    store
        .capture(&context, capture("cap-broker-b", b"broker b: 55"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-broker-b".to_owned(),
                capture_operation_id: "cap-broker-b".to_owned(),
                draft: stock_draft("GULF", 55, None, None),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-broker-b".to_owned(),
                proposal_operation_id: "prop-broker-b".to_owned(),
            },
        )
        .unwrap();

    let current = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(current.active.len(), 2);
    let values: Vec<_> = current.active.iter().map(|claim| claim.value.clone()).collect();
    assert!(values.contains(&json!(48)));
    assert!(values.contains(&json!(55)));
}

#[test]
fn future_valid_claim_is_known_but_not_current_until_valid_from() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"future guidance"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft(
                    "GULF",
                    70,
                    Some(at("2027-01-01T00:00:00Z")),
                    None,
                ),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    let before_valid_from = store
        .claims_current(
            store.ledger_head().unwrap(),
            at("2026-07-14T00:00:00Z"),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(before_valid_from.active.len(), 0);
    assert_eq!(before_valid_from.future.len(), 1);
    assert_eq!(before_valid_from.future[0].value, json!(70));
    assert_eq!(before_valid_from.past.len(), 0);

    let after_valid_from = store
        .claims_current(
            store.ledger_head().unwrap(),
            at("2027-02-01T00:00:00Z"),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(after_valid_from.active.len(), 1);
    assert_eq!(after_valid_from.future.len(), 0);
}

#[test]
fn propose_inference_without_evidence_is_unsupported_and_cannot_be_confirmed() {
    let (_parent, _root, store, context) = fixture();
    store
        .propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "prop-inference".to_owned(),
                evidence_capture_operation_ids: Vec::new(),
                method: "llm-synthesis".to_owned(),
                model: Some("glm-test".to_owned()),
                prompt_version: Some("v1".to_owned()),
                draft: stock_draft("GULF", 90, None, None),
            },
        )
        .unwrap();

    assert!(matches!(
        store.confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-unsupported".to_owned(),
                proposal_operation_id: "prop-inference".to_owned(),
            },
        ),
        Err(SemanticError::UnsupportedInference)
    ));
    assert_eq!(store.diagnostics().unwrap().events, 1);

    // User acceptance is a brand-new event, not a mutation of the unsupported one.
    store
        .capture(&context, capture("cap-accept", b"user confirmed deployment"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-accept".to_owned(),
                capture_operation_id: "cap-accept".to_owned(),
                draft: decision_draft(),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-accept".to_owned(),
                proposal_operation_id: "prop-accept".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(store.diagnostics().unwrap().events, 4);
}

#[test]
fn propose_inference_with_evidence_is_supported_and_confirmable() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"underlying evidence text"))
        .unwrap();
    store
        .propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "prop-inference".to_owned(),
                evidence_capture_operation_ids: vec!["cap".to_owned()],
                method: "llm-synthesis".to_owned(),
                model: Some("glm-test".to_owned()),
                prompt_version: Some("v1".to_owned()),
                draft: stock_draft("GULF", 60, None, None),
            },
        )
        .unwrap();

    let outcome = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-inference".to_owned(),
                proposal_operation_id: "prop-inference".to_owned(),
            },
        )
        .unwrap();
    let claim_object = store
        .object_json(&outcome.event.payload.object_id)
        .unwrap();
    assert_eq!(claim_object["claim"]["provenance"]["kind"], "inference");
    assert_eq!(claim_object["claim"]["provenance"]["unsupported"], false);
    assert_eq!(claim_object["claim"]["provenance"]["method"], "llm-synthesis");
}
