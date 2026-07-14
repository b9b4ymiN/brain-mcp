use std::path::{Path, PathBuf};

use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, RejectCommand,
    RetractCommand, SemanticConfig, SemanticError, SemanticStore, SupersedeCommand, TrustedContext,
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

fn draft() -> ClaimDraft {
    ClaimDraft {
        subject: "project:brain".to_owned(),
        predicate: "deployment".to_owned(),
        value: json!("sqlite-event-ledger"),
        claim_kind: "project_decision".to_owned(),
        domain: "projects".to_owned(),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

#[test]
fn default_registered_client_retains_confirm_capability() {
    // Regression guard: register_client's existing 1-argument signature must
    // keep granting full capability by default, exactly as every Task
    // 1.1/1.2 test already assumes.
    let (_parent, _root, store, _bootstrap) = fixture();
    let codex = store.register_client("codex").unwrap();
    store.capture(&codex, capture("cap", b"evidence bytes")).unwrap();
    store
        .propose(
            &codex,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    store
        .confirm(
            &codex,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
}

#[test]
fn worker_client_can_propose_but_cannot_confirm_reject_retract_or_supersede() {
    let (_parent, _root, store, bootstrap) = fixture();
    let worker = store.register_client_scoped("extraction-worker", &[]).unwrap();

    store.capture(&worker, capture("cap", b"evidence bytes")).unwrap();
    store
        .propose(
            &worker,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();

    assert!(matches!(
        store.confirm(
            &worker,
            ConfirmCommand {
                operation_id: "confirm-denied".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));
    assert!(matches!(
        store.reject(
            &worker,
            RejectCommand {
                operation_id: "reject-denied".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));

    // A trusted actor confirms it instead, then the worker still cannot
    // retract or supersede its own confirmed claim.
    let confirmed = store
        .confirm(
            &bootstrap,
            ConfirmCommand {
                operation_id: "confirm-trusted".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
    assert!(matches!(
        store.retract(
            &worker,
            RetractCommand {
                operation_id: "retract-denied".to_owned(),
                claim_operation_id: "confirm-trusted".to_owned(),
            },
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));

    store
        .capture(&worker, capture("cap-b", b"superseding evidence"))
        .unwrap();
    store
        .propose(
            &worker,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    assert!(matches!(
        store.supersede(
            &worker,
            SupersedeCommand {
                operation_id: "supersede-denied".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-trusted".to_owned()],
            },
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));

    // Denial happens before any event is written.
    let diagnostics = store.diagnostics().unwrap();
    assert_eq!(diagnostics.events, 5); // cap, prop, cap-b, prop-b, confirm-trusted
    let _ = confirmed;
}

#[test]
fn worker_can_be_granted_confirm_without_purge() {
    let (_parent, _root, store, _bootstrap) = fixture();
    let promoted = store
        .register_client_scoped("promoted-worker", &["confirm"])
        .unwrap();
    store.capture(&promoted, capture("cap", b"evidence bytes")).unwrap();
    store
        .propose(
            &promoted,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    store
        .confirm(
            &promoted,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
}

#[test]
fn scoped_registration_of_an_existing_label_does_not_change_prior_capabilities() {
    let (_parent, _root, store, _bootstrap) = fixture();
    let worker = store.register_client_scoped("worker", &[]).unwrap();
    // Re-registering the same label with a broader capability list must not
    // silently escalate an already-registered client's grants.
    let same = store.register_client_scoped("worker", &["confirm", "purge"]).unwrap();
    assert_eq!(worker, same);

    store.capture(&worker, capture("cap", b"evidence bytes")).unwrap();
    store
        .propose(
            &worker,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    assert!(matches!(
        store.confirm(
            &worker,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));
}

#[test]
fn invalid_capability_name_is_rejected() {
    let (_parent, _root, store, _bootstrap) = fixture();
    assert!(matches!(
        store.register_client_scoped("worker", &["not-a-real-capability"]),
        Err(SemanticError::InvalidClaim(_))
    ));
}
