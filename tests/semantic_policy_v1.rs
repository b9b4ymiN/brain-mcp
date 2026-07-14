//! Task 1.3, Sub-slice A: policy tests for explicit user save, latest
//! scoped correction ordering, scope-key validation, and trusted-actor
//! enforcement across the confirm/reject/retract/supersede mutations.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticClock,
    SemanticConfig, SemanticError, SemanticStore, SupersedeCommand, TrustedContext,
};
#[cfg(feature = "semantic-test-failpoints")]
use llm_wiki::semantic::{RejectCommand, RetractCommand};
use serde_json::json;
use tempfile::TempDir;
#[cfg(feature = "semantic-test-failpoints")]
use uuid::Uuid;

#[derive(Debug)]
struct TestClock(AtomicI64);

impl TestClock {
    fn at(value: DateTime<Utc>) -> Self {
        Self(AtomicI64::new(value.timestamp_millis()))
    }

    fn set(&self, value: DateTime<Utc>) {
        self.0.store(value.timestamp_millis(), Ordering::SeqCst);
    }
}

impl SemanticClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.0.load(Ordering::SeqCst))
            .single()
            .expect("valid test timestamp")
    }
}

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

fn fixture_with_clock(clock: Arc<TestClock>) -> (TempDir, PathBuf, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let config = enabled(parent.path()).with_clock(clock);
    let (store, _admin) = SemanticStore::create(&root, config).expect("create");
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

fn decision_draft(value: &str) -> ClaimDraft {
    ClaimDraft {
        subject: "project:brain".to_owned(),
        predicate: "deployment".to_owned(),
        value: json!(value),
        claim_kind: "decision".to_owned(),
        domain: "projects".to_owned(),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Explicit user save: capture -> propose -> confirm produces a claim whose
/// ledger events are all attributed to the one authenticated actor, and a
/// second registered client (different technical identity, same owner) is
/// attributed to the same actor_id but a distinct client_id -- exactly the
/// ADR Decision 6 "owner and partner share actor_id=owner; only client is
/// auditable" model.
#[test]
fn explicit_user_save_is_attributed_to_one_authenticated_actor_across_clients() {
    let (_parent, _root, store, bootstrap) = fixture();
    let second_client = store.register_client("second-device").unwrap();
    assert_ne!(bootstrap, second_client);

    let bootstrap_capture = store
        .capture(
            &bootstrap,
            capture("cap-bootstrap", b"user said docker compose"),
        )
        .unwrap();
    let bootstrap_propose = store
        .propose(
            &bootstrap,
            ProposeCommand {
                operation_id: "prop-bootstrap".to_owned(),
                capture_operation_id: "cap-bootstrap".to_owned(),
                draft: decision_draft("docker-compose"),
            },
        )
        .unwrap();
    let bootstrap_confirm = store
        .confirm(
            &bootstrap,
            ConfirmCommand {
                operation_id: "confirm-bootstrap".to_owned(),
                proposal_operation_id: "prop-bootstrap".to_owned(),
            },
        )
        .unwrap();

    let second_capture = store
        .capture(&second_client, capture("cap-second", b"second device save"))
        .unwrap();

    // Same authenticated actor (single-owner model) ...
    assert_eq!(
        bootstrap_capture.event.actor_id,
        bootstrap_propose.event.actor_id
    );
    assert_eq!(
        bootstrap_capture.event.actor_id,
        bootstrap_confirm.event.actor_id
    );
    assert_eq!(
        bootstrap_capture.event.actor_id,
        second_capture.event.actor_id
    );
    // ... but distinct, auditable client identities.
    assert_ne!(
        bootstrap_capture.event.client_id,
        second_capture.event.client_id
    );

    let object = store
        .object_json(&bootstrap_confirm.event.payload.object_id)
        .unwrap();
    assert_eq!(object["claim"]["value"], "docker-compose");
    assert_eq!(object["claim"]["claim_kind"], "decision");
}

/// Latest scoped correction must be ordered by the server-assigned monotonic
/// `event_seq`, never by wall-clock `recorded_at`. This drives the clock
/// backward between the original confirm and the correcting supersede so a
/// timestamp-based (or `recorded_at`-based) implementation would pick the
/// wrong claim as active, while an `event_seq`-based one still returns the
/// later (superseding) claim.
#[test]
fn latest_scoped_correction_orders_by_event_seq_not_wall_clock() {
    let clock = Arc::new(TestClock::at("2026-07-15T12:00:00Z".parse().unwrap()));
    let (_parent, _root, store, context) = fixture_with_clock(clock.clone());

    store
        .capture(&context, capture("cap-a", b"fair value 58"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: decision_draft("kubernetes"),
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

    // Wall clock moves BACKWARD before the correction is recorded.
    clock.set("2026-07-15T09:00:00Z".parse().unwrap());

    store
        .capture(&context, capture("cap-b", b"fair value 62"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: decision_draft("docker-compose"),
            },
        )
        .unwrap();
    let correction = store
        .supersede(
            &context,
            SupersedeCommand {
                operation_id: "supersede-b".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec!["confirm-a".to_owned()],
            },
        )
        .unwrap();

    // The superseding event's recorded_at is earlier than the superseded
    // event's, yet its event_seq is greater.
    let original = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "projects",
            "project:brain",
            "deployment",
        )
        .unwrap();
    assert_eq!(original.active.len(), 1);
    assert_eq!(original.active[0].value, "docker-compose");
    assert_eq!(original.past.len(), 1);
    assert_eq!(original.past[0].value, "kubernetes");
    assert!(correction.event.event_seq > 1);
}

/// Scope key fields must be validated before any event is appended --
/// whitespace-only domain/subject/predicate are rejected exactly like empty
/// ones, and no partial state (proposal/event) survives the rejection.
#[test]
fn scope_key_with_whitespace_only_fields_is_rejected_before_any_event() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence bytes"))
        .unwrap();

    let mut blank_domain = decision_draft("x");
    blank_domain.domain = "   ".to_owned();
    assert!(matches!(
        store.propose(
            &context,
            ProposeCommand {
                operation_id: "prop-blank-domain".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: blank_domain,
            },
        ),
        Err(SemanticError::InvalidClaim(_))
    ));

    let mut blank_subject = decision_draft("x");
    blank_subject.subject = "\t\n".to_owned();
    assert!(matches!(
        store.propose(
            &context,
            ProposeCommand {
                operation_id: "prop-blank-subject".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: blank_subject,
            },
        ),
        Err(SemanticError::InvalidClaim(_))
    ));

    let mut blank_predicate = decision_draft("x");
    blank_predicate.predicate = String::new();
    assert!(matches!(
        store.propose(
            &context,
            ProposeCommand {
                operation_id: "prop-blank-predicate".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: blank_predicate,
            },
        ),
        Err(SemanticError::InvalidClaim(_))
    ));

    // Only the one legitimate capture event exists; all three rejected
    // proposals left no trace.
    assert_eq!(store.diagnostics().unwrap().events, 1);
}

/// Trusted actor must be derived from an authenticated, registered client --
/// a forged (unregistered) context is rejected fail-closed by every
/// confirm-family mutation, not just capture (Task 1.1's existing coverage).
/// Confirms the shared `mutate_once` choke point (already proven for
/// capture) uniformly protects confirm/reject/retract/supersede too.
#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn forged_unregistered_client_cannot_confirm_reject_retract_or_supersede() {
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
                draft: decision_draft("kubernetes"),
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
        .capture(&context, capture("cap-b", b"correction evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: decision_draft("docker-compose"),
            },
        )
        .unwrap();

    let before = store.diagnostics().unwrap();
    let forged = store.forge_context_for_test(Uuid::now_v7());

    assert!(matches!(
        store.confirm(
            &forged,
            ConfirmCommand {
                operation_id: "forged-confirm".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.reject(
            &forged,
            RejectCommand {
                operation_id: "forged-reject".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.retract(
            &forged,
            RetractCommand {
                operation_id: "forged-retract".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store.supersede(
            &forged,
            SupersedeCommand {
                operation_id: "forged-supersede".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
                superseded_claim_operation_ids: vec!["confirm".to_owned()],
            },
        ),
        Err(SemanticError::MissingDependency(_))
    ));

    let after = store.diagnostics().unwrap();
    assert_eq!(before, after);
}
