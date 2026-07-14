use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticError, SemanticStore, TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;
#[cfg(feature = "semantic-test-failpoints")]
use uuid::Uuid;
use uuid::Version;

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
        valid_from: None::<DateTime<Utc>>,
        valid_to: None::<DateTime<Utc>>,
    }
}

#[test]
fn registered_clients_have_stable_server_generated_identity() {
    let (parent, root, store, _bootstrap) = fixture();

    let codex = store.register_client("codex").unwrap();
    let console = store.register_client("console").unwrap();

    let codex_event = store
        .capture(&codex, capture("codex-capture", b"from codex"))
        .unwrap()
        .event;
    let console_event = store
        .capture(&console, capture("console-capture", b"from console"))
        .unwrap()
        .event;
    assert_ne!(codex_event.client_id, console_event.client_id);
    assert_eq!(codex_event.client_id.get_version(), Some(Version::SortRand));
    assert_eq!(
        console_event.client_id.get_version(),
        Some(Version::SortRand)
    );
    assert_eq!(codex_event.actor_id, console_event.actor_id);

    let codex_again = store.register_client("codex").unwrap();
    let replay = store
        .capture(&codex_again, capture("codex-capture", b"from codex"))
        .unwrap()
        .event;
    assert_eq!(replay.event_id, codex_event.event_id);
    assert_eq!(replay.client_id, codex_event.client_id);

    drop(store);
    let reopened = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let codex_reopened = reopened.register_client("codex").unwrap();
    let replay = reopened
        .capture(&codex_reopened, capture("codex-capture", b"from codex"))
        .unwrap()
        .event;
    assert_eq!(replay.event_id, codex_event.event_id);
    assert_eq!(replay.client_id, codex_event.client_id);
}

#[test]
fn register_client_rejects_invalid_and_reserved_labels() {
    let (_parent, _root, store, _bootstrap) = fixture();
    for label in [
        "",
        "__bootstrap__",
        "__anything",
        "UPPER",
        "has space",
        "ยาว",
    ] {
        assert!(
            matches!(
                store.register_client(label),
                Err(SemanticError::MissingDependency(_))
            ),
            "label {label:?} must be rejected"
        );
    }
    let long = "a".repeat(65);
    assert!(matches!(
        store.register_client(&long),
        Err(SemanticError::MissingDependency(_))
    ));
}

#[test]
fn same_operation_id_is_isolated_per_client_and_conflicts_within_client() {
    let (_parent, _root, store, bootstrap) = fixture();
    let first = store.register_client("client-one").unwrap();
    let second = store.register_client("client-two").unwrap();

    let first_outcome = store
        .capture(&first, capture("shared-operation", b"payload from one"))
        .unwrap();
    let second_outcome = store
        .capture(&second, capture("shared-operation", b"payload from two"))
        .unwrap();
    assert_ne!(first_outcome.event.event_id, second_outcome.event.event_id);
    assert_ne!(
        first_outcome.event.payload.object_id,
        second_outcome.event.payload.object_id
    );
    assert_ne!(
        first_outcome.event.client_id,
        second_outcome.event.client_id
    );

    let replay = store
        .capture(&first, capture("shared-operation", b"payload from one"))
        .unwrap();
    assert_eq!(replay.event.event_id, first_outcome.event.event_id);

    assert!(matches!(
        store.capture(&first, capture("shared-operation", b"different payload")),
        Err(SemanticError::IdempotencyConflict)
    ));

    let bootstrap_outcome = store
        .capture(
            &bootstrap,
            capture("shared-operation", b"payload from owner"),
        )
        .unwrap();
    assert_ne!(
        bootstrap_outcome.event.event_id,
        first_outcome.event.event_id
    );

    let diagnostics = store.diagnostics().unwrap();
    assert_eq!((diagnostics.events, diagnostics.operations), (3, 3));
    assert_eq!(diagnostics.outbox_pending, 0);
    assert_eq!(diagnostics.ledger_checksum, diagnostics.projection_checksum);
}

#[test]
fn contexts_do_not_cross_stores_in_either_direction() {
    let (_parent_a, _root_a, store_a, _bootstrap_a) = fixture();
    let (_parent_b, _root_b, store_b, _bootstrap_b) = fixture();
    let registered_a = store_a.register_client("codex").unwrap();
    let registered_b = store_b.register_client("codex").unwrap();

    assert!(matches!(
        store_b.capture(&registered_a, capture("cross", b"bytes")),
        Err(SemanticError::MissingDependency(_))
    ));
    assert!(matches!(
        store_a.capture(&registered_b, capture("cross", b"bytes")),
        Err(SemanticError::MissingDependency(_))
    ));
    assert_eq!(store_a.diagnostics().unwrap().events, 0);
    assert_eq!(store_b.diagnostics().unwrap().events, 0);
}

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn forged_unregistered_client_is_rejected_fail_closed() {
    let (_parent, _root, store, _bootstrap) = fixture();
    let forged = store.forge_context_for_test(Uuid::now_v7());
    assert!(matches!(
        store.capture(&forged, capture("forged", b"bytes")),
        Err(SemanticError::MissingDependency(_))
    ));
    let diagnostics = store.diagnostics().unwrap();
    assert_eq!((diagnostics.events, diagnostics.operations), (0, 0));
}

#[test]
fn capture_rejects_oversized_bytes_fail_closed() {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let config = enabled(parent.path()).with_max_object_bytes(8);
    let (store, _admin) = SemanticStore::create(&root, config).expect("create");
    let context = store.trusted_context();

    assert!(matches!(
        store.capture(&context, capture("too-big", b"nine bytes")),
        Err(SemanticError::InvalidCapture(_))
    ));
    let diagnostics = store.diagnostics().unwrap();
    assert_eq!((diagnostics.events, diagnostics.operations), (0, 0));

    store
        .capture(&context, capture("at-limit", b"8 bytes."))
        .unwrap();
    assert_eq!(store.diagnostics().unwrap().events, 1);
}

#[test]
fn capture_default_limit_is_32_mib() {
    let (_parent, _root, store, context) = fixture();
    let oversized = vec![0_u8; 32 * 1024 * 1024 + 1];
    let mut command = capture("default-limit", &oversized);
    command.media_type = "application/octet-stream".to_owned();
    assert!(matches!(
        store.capture(&context, command),
        Err(SemanticError::InvalidCapture(_))
    ));
    assert_eq!(store.diagnostics().unwrap().events, 0);
}

#[test]
fn capture_rejects_malformed_media_types() {
    let (_parent, _root, store, context) = fixture();
    for media_type in [
        "",
        "text",
        "/plain",
        "text/",
        "text plain",
        "text/plain/extra",
        "text/pl ain",
    ] {
        let mut command = capture("bad-media", b"bytes");
        command.media_type = media_type.to_owned();
        assert!(
            matches!(
                store.capture(&context, command),
                Err(SemanticError::InvalidCapture(_))
            ),
            "media type {media_type:?} must be rejected"
        );
    }
    assert_eq!(store.diagnostics().unwrap().events, 0);

    for (operation, media_type) in [
        ("ok-plain", "text/plain"),
        ("ok-vendor", "application/vnd.brain.semantic+json"),
        ("ok-params", "text/plain; charset=utf-8"),
    ] {
        let mut command = capture(operation, b"bytes");
        command.media_type = media_type.to_owned();
        store.capture(&context, command).unwrap();
    }
    assert_eq!(store.diagnostics().unwrap().events, 3);
}

#[test]
fn identical_bytes_deduplicate_to_one_object() {
    let (_parent, root, store, context) = fixture();
    let first = store
        .capture(&context, capture("dedup-one", b"identical bytes"))
        .unwrap();
    let second = store
        .capture(&context, capture("dedup-two", b"identical bytes"))
        .unwrap();
    assert_ne!(first.event.event_id, second.event.event_id);
    assert_eq!(
        first.event.payload.object_id,
        second.event.payload.object_id
    );

    let mut object_files = 0;
    for shard in fs::read_dir(root.join("objects")).unwrap() {
        for entry in fs::read_dir(shard.unwrap().path()).unwrap() {
            assert!(entry.unwrap().path().is_file());
            object_files += 1;
        }
    }
    assert_eq!(object_files, 1);
    assert_eq!(store.diagnostics().unwrap().events, 2);
}

#[test]
fn consistent_backup_restores_identical_state_and_stays_isolated() {
    let (parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("backup-capture", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "backup-proposal".to_owned(),
                capture_operation_id: "backup-capture".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "backup-confirm".to_owned(),
                proposal_operation_id: "backup-proposal".to_owned(),
            },
        )
        .unwrap();
    let source_diagnostics = store.diagnostics().unwrap();
    assert_eq!(source_diagnostics.events, 3);

    let backup_root = parent.path().join("semantic-backup");
    store.backup_consistent(&backup_root).unwrap();

    let backup = SemanticStore::open(&backup_root, enabled(parent.path())).unwrap();
    let backup_diagnostics = backup.diagnostics().unwrap();
    assert_eq!(backup_diagnostics.events, source_diagnostics.events);
    assert_eq!(backup_diagnostics.operations, source_diagnostics.operations);
    assert_eq!(backup_diagnostics.outbox_pending, 0);
    assert_eq!(
        backup_diagnostics.ledger_checksum,
        source_diagnostics.ledger_checksum
    );
    assert_eq!(
        backup_diagnostics.ledger_checksum,
        backup_diagnostics.projection_checksum
    );

    store
        .capture(&context, capture("after-backup", b"newer bytes"))
        .unwrap();
    assert_eq!(store.diagnostics().unwrap().events, 4);
    assert_eq!(backup.diagnostics().unwrap().events, 3);
}

#[test]
fn backup_rejects_existing_or_outside_targets() {
    let (parent, root, store, _context) = fixture();

    assert!(matches!(
        store.backup_consistent(&root),
        Err(SemanticError::InvalidRoot(_))
    ));

    let outside = tempfile::tempdir().expect("outside parent");
    assert!(matches!(
        store.backup_consistent(outside.path().join("escape")),
        Err(SemanticError::InvalidRoot(_))
    ));

    let nested = parent.path().join("missing-middle").join("backup");
    assert!(matches!(
        store.backup_consistent(nested),
        Err(SemanticError::InvalidRoot(_))
    ));
}
