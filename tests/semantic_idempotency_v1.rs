use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

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

fn draft(subject: &str) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(58),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

#[test]
fn reject_replays_idempotently_and_conflicts_on_different_target() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-a", b"evidence a"))
        .unwrap();
    store
        .capture(&context, capture("cap-b", b"evidence b"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft("PTT"),
            },
        )
        .unwrap();

    let first = store
        .reject(
            &context,
            RejectCommand {
                operation_id: "reject-op".to_owned(),
                proposal_operation_id: "prop-a".to_owned(),
            },
        )
        .unwrap();
    let replay = store
        .reject(
            &context,
            RejectCommand {
                operation_id: "reject-op".to_owned(),
                proposal_operation_id: "prop-a".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        replay.canonical_bytes().unwrap()
    );
    assert_eq!(store.diagnostics().unwrap().events, 5);

    assert!(matches!(
        store.reject(
            &context,
            RejectCommand {
                operation_id: "reject-op".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
            },
        ),
        Err(SemanticError::IdempotencyConflict)
    ));
    assert_eq!(store.diagnostics().unwrap().events, 5);
}

#[test]
fn retract_replays_idempotently_and_conflicts_on_different_target() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-a", b"evidence a"))
        .unwrap();
    store
        .capture(&context, capture("cap-b", b"evidence b"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft("PTT"),
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
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-b".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
            },
        )
        .unwrap();

    let first = store
        .retract(
            &context,
            RetractCommand {
                operation_id: "retract-op".to_owned(),
                claim_operation_id: "confirm-a".to_owned(),
            },
        )
        .unwrap();
    let replay = store
        .retract(
            &context,
            RetractCommand {
                operation_id: "retract-op".to_owned(),
                claim_operation_id: "confirm-a".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        replay.canonical_bytes().unwrap()
    );
    assert_eq!(store.diagnostics().unwrap().events, 7);

    assert!(matches!(
        store.retract(
            &context,
            RetractCommand {
                operation_id: "retract-op".to_owned(),
                claim_operation_id: "confirm-b".to_owned(),
            },
        ),
        Err(SemanticError::IdempotencyConflict)
    ));
    assert_eq!(store.diagnostics().unwrap().events, 7);
}

#[test]
fn supersede_replays_idempotently_and_conflicts_on_different_target() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-old", b"old evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-old".to_owned(),
                capture_operation_id: "cap-old".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-old".to_owned(),
                proposal_operation_id: "prop-old".to_owned(),
            },
        )
        .unwrap();
    store
        .capture(&context, capture("cap-new", b"new evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-new".to_owned(),
                capture_operation_id: "cap-new".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    store
        .capture(&context, capture("cap-other", b"other evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-other".to_owned(),
                capture_operation_id: "cap-other".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();

    let first = store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-op".to_owned(),
                proposal_operation_id: "prop-new".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-old".to_owned()],
            },
        )
        .unwrap();
    let replay = store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-op".to_owned(),
                proposal_operation_id: "prop-new".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-old".to_owned()],
            },
        )
        .unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        replay.canonical_bytes().unwrap()
    );
    let events_after_replay = store.diagnostics().unwrap().events;

    assert!(matches!(
        store.supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-op".to_owned(),
                proposal_operation_id: "prop-other".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-old".to_owned()],
            },
        ),
        Err(SemanticError::IdempotencyConflict)
    ));
    assert_eq!(store.diagnostics().unwrap().events, events_after_replay);
}

#[test]
fn propose_inference_replays_idempotently_and_conflicts_on_different_payload() {
    let (_parent, _root, store, context) = fixture();
    let first = store
        .propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "prop-inf".to_owned(),
                evidence_capture_operation_ids: Vec::new(),
                method: "llm-synthesis".to_owned(),
                model: Some("glm-test".to_owned()),
                prompt_version: Some("v1".to_owned()),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    let replay = store
        .propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "prop-inf".to_owned(),
                evidence_capture_operation_ids: Vec::new(),
                method: "llm-synthesis".to_owned(),
                model: Some("glm-test".to_owned()),
                prompt_version: Some("v1".to_owned()),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        replay.canonical_bytes().unwrap()
    );
    assert_eq!(store.diagnostics().unwrap().events, 1);

    assert!(matches!(
        store.propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "prop-inf".to_owned(),
                evidence_capture_operation_ids: Vec::new(),
                method: "llm-synthesis".to_owned(),
                model: Some("glm-test".to_owned()),
                prompt_version: Some("v2".to_owned()),
                draft: draft("GULF"),
            },
        ),
        Err(SemanticError::IdempotencyConflict)
    ));
    assert_eq!(store.diagnostics().unwrap().events, 1);
}

#[test]
fn cross_client_reference_to_anothers_operation_id_is_missing_dependency_not_a_leak() {
    let (_parent, _root, store, _bootstrap) = fixture();
    let client_a = store.register_client("client-a").unwrap();
    let client_b = store.register_client("client-b").unwrap();

    store
        .capture(&client_a, capture("cap", b"evidence"))
        .unwrap();
    store
        .propose(
            &client_a,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft("GULF"),
            },
        )
        .unwrap();
    store
        .confirm(
            &client_a,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    // client_b never made these operation_ids; referencing client_a's is a
    // missing dependency, not a cross-client leak into client_a's data.
    assert!(matches!(
        store.reject(
            &client_b,
            RejectCommand {
                operation_id: "reject-b".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.confirm(
            &client_b,
            ConfirmCommand {
                operation_id: "confirm-b".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.retract(
            &client_b,
            RetractCommand {
                operation_id: "retract-b".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.supersede(
            &client_b,
            SupersedeCommand {
                operation_id: "supersede-b".to_owned(),
                proposal_operation_id: "prop".to_owned(),
                superseded_claim_operation_ids: vec!["confirm".to_owned()],
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));

    // client_a's claim is untouched by client_b's failed attempts.
    let current = store
        .claims_current(
            store.ledger_head().unwrap(),
            chrono::Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(current.active.len(), 1);
}

#[test]
fn concurrent_retract_race_on_same_claim_has_exactly_one_winner() {
    let (parent, root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft("GULF"),
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
    drop(store);

    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for index in 0..2 {
        let parent_path = parent.path().to_path_buf();
        let root_path = root.clone();
        let sender = tx.clone();
        handles.push(thread::spawn(move || {
            let store = SemanticStore::open(&root_path, enabled(&parent_path)).unwrap();
            let context = store.trusted_context();
            let result = store.retract(
                &context,
                RetractCommand {
                    operation_id: format!("retract-{index}"),
                    claim_operation_id: "confirm".to_owned(),
                },
            );
            sender.send(result).unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let results: Vec<_> = rx.try_iter().collect();
    assert_eq!(results.len(), 2);
    let successes = results.iter().filter(|result| result.is_ok()).count();
    let conflicts = results
        .iter()
        .filter(|result| matches!(result, Err(SemanticError::InvalidTransition(_))))
        .count();
    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);

    let reopened = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    assert_eq!(reopened.diagnostics().unwrap().events, 4);
}
