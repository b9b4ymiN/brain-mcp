use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{DateTime, TimeZone, Utc};
use jsonschema::validator_for;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, ManualRecovery, MutationOutcome, ProjectionState,
    ProposeCommand, RollbackStatus, SemanticClock, SemanticConfig, SemanticError, SemanticStore,
    StoreDiagnostics, TrustedContext, canonicalize_json,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::{Uuid, Version};

const EVENT_SCHEMA: &str = include_str!("../evals/v1/contracts/event-schema-v1.json");

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

fn at(value: &str) -> DateTime<Utc> {
    value.parse().expect("RFC 3339 test timestamp")
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

fn capture(operation_id: &str, bytes: &[u8]) -> CaptureCommand {
    CaptureCommand {
        operation_id: operation_id.to_owned(),
        bytes: bytes.to_vec(),
        media_type: "text/plain; charset=utf-8".to_owned(),
    }
}

fn draft(valid_from: Option<DateTime<Utc>>, valid_to: Option<DateTime<Utc>>) -> ClaimDraft {
    ClaimDraft {
        subject: "project:brain".to_owned(),
        predicate: "deployment".to_owned(),
        value: json!("sqlite-event-ledger"),
        valid_from,
        valid_to,
    }
}

fn assert_uuid_v7(value: Uuid) {
    assert_eq!(value.get_version(), Some(Version::SortRand));
}

fn assert_schema_valid(outcome: &MutationOutcome) {
    let schema: Value = serde_json::from_str(EVENT_SCHEMA).expect("schema JSON");
    let validator = validator_for(&schema).expect("compile event schema");
    let event = serde_json::to_value(&outcome.event).expect("event JSON");
    if let Err(error) = validator.validate(&event) {
        panic!("event schema violation: {error}; event={event}");
    }
    assert_eq!(
        event["payload"].as_object().unwrap().keys().count(),
        if event["payload"].get("media_type").is_some() {
            3
        } else {
            2
        }
    );
}

fn happy_path(
    store: &SemanticStore,
    context: &TrustedContext,
    prefix: &str,
    valid_from: Option<DateTime<Utc>>,
    valid_to: Option<DateTime<Utc>>,
) -> (MutationOutcome, MutationOutcome, MutationOutcome) {
    let captured = store
        .capture(
            context,
            capture(&format!("{prefix}-capture"), b"source evidence"),
        )
        .expect("capture");
    let proposed = store
        .propose(
            context,
            ProposeCommand {
                operation_id: format!("{prefix}-propose"),
                capture_operation_id: format!("{prefix}-capture"),
                draft: draft(valid_from, valid_to),
            },
        )
        .expect("propose");
    let confirmed = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: format!("{prefix}-confirm"),
                proposal_operation_id: format!("{prefix}-propose"),
            },
        )
        .expect("confirm");
    (captured, proposed, confirmed)
}

#[test]
fn disabled_by_default_fails_before_any_filesystem_access() {
    let parent = tempfile::tempdir().unwrap();
    let missing_parent = parent.path().join("must-not-be-touched");
    let root = missing_parent.join("store");

    let error = SemanticStore::create(&root, SemanticConfig::default()).unwrap_err();

    assert!(matches!(error, SemanticError::Disabled));
    assert!(!missing_parent.exists());
}

#[test]
fn capture_propose_confirm_is_schema_valid_trusted_and_bitemporal() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let clock = Arc::new(TestClock::at(at("2026-07-13T02:00:00Z")));
    let config = enabled(parent.path()).with_clock(clock.clone());
    let (store, _admin) = SemanticStore::create(&root, config).unwrap();
    let context = store.trusted_context();

    let (captured, proposed, confirmed) = happy_path(
        &store,
        &context,
        "journey",
        Some(at("2026-07-13T01:00:00Z")),
        Some(at("2026-07-14T01:00:00Z")),
    );

    assert_eq!(
        (
            captured.event.event_seq,
            proposed.event.event_seq,
            confirmed.event.event_seq
        ),
        (1, 2, 3)
    );
    for outcome in [&captured, &proposed, &confirmed] {
        assert_schema_valid(outcome);
        assert_uuid_v7(outcome.event.owner_id);
        assert_uuid_v7(outcome.event.event_id);
        assert_uuid_v7(outcome.event.actor_id);
        assert_uuid_v7(outcome.event.client_id);
    }
    for id in [
        captured.generated.source_id,
        captured.generated.rendition_id,
        captured.generated.evidence_id,
        proposed.generated.proposal_id,
        confirmed.generated.claim_id,
    ] {
        assert_uuid_v7(id.expect("event-specific generated ID"));
    }
    assert!(captured.event.prior_event_hash.is_none());
    assert_eq!(
        proposed.event.prior_event_hash.as_deref(),
        Some(captured.event.event_hash.as_str())
    );
    assert_eq!(
        confirmed.event.prior_event_hash.as_deref(),
        Some(proposed.event.event_hash.as_str())
    );

    assert!(
        store
            .claim_at(2, at("2026-07-13T02:00:00Z"))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .claim_at(3, at("2026-07-13T00:59:59Z"))
            .unwrap()
            .is_none()
    );
    let current = store
        .claim_at(3, at("2026-07-13T01:00:00Z"))
        .unwrap()
        .unwrap();
    assert_eq!(current.claim_id, confirmed.generated.claim_id.unwrap());
    assert!(
        store
            .claim_at(3, at("2026-07-14T01:00:00Z"))
            .unwrap()
            .is_none()
    );

    // recorded_at can move backwards, but ledger sequence remains the authority.
    clock.set(at("2026-07-12T23:00:00Z"));
    let rollback_capture = store
        .capture(&context, capture("clock-rollback", b"later event"))
        .unwrap();
    assert_eq!(rollback_capture.event.event_seq, 4);
    assert!(rollback_capture.event.recorded_at < confirmed.event.recorded_at);
    assert_eq!(store.ledger_head().unwrap(), 4);
}

#[test]
fn rfc8785_vector_hash_chain_and_byte_exact_idempotency() {
    let vector = json!({
        "numbers": [333333333.33333329_f64, 1E30_f64, 4.50_f64, 2e-3_f64, 1e-27_f64],
        "string": "€$\u{000f}\nA'B\"\\\\\"/",
        "literals": [null, true, false]
    });
    let expected = "{\"literals\":[null,true,false],\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27],\"string\":\"€$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}";
    assert_eq!(canonicalize_json(&vector).unwrap(), expected.as_bytes());

    let (_parent, _root, store, context) = fixture();
    let command = capture("same-operation", b"same bytes");
    let first = store.capture(&context, command.clone()).unwrap();
    let replay = store.capture(&context, command).unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        replay.canonical_bytes().unwrap()
    );
    assert_eq!(store.diagnostics().unwrap().events, 1);

    let changed = store.capture(&context, capture("same-operation", b"changed bytes"));
    assert!(matches!(changed, Err(SemanticError::IdempotencyConflict)));
    let altered_type = store.propose(
        &context,
        ProposeCommand {
            operation_id: "same-operation".to_owned(),
            capture_operation_id: "same-operation".to_owned(),
            draft: draft(None, None),
        },
    );
    assert!(matches!(
        altered_type,
        Err(SemanticError::IdempotencyConflict)
    ));
    assert_eq!(store.diagnostics().unwrap().events, 1);
}

#[test]
fn independent_connections_serialize_unique_and_conflicting_operations() {
    let (parent, root, store, context) = fixture();
    let second = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let barrier = Arc::new(Barrier::new(21));
    let mut joins = Vec::new();
    for index in 0..20 {
        let root = root.clone();
        let parent = parent.path().to_path_buf();
        let context = context.clone();
        let barrier = barrier.clone();
        joins.push(thread::spawn(move || {
            let independent = SemanticStore::open(&root, enabled(&parent)).unwrap();
            barrier.wait();
            independent.capture(
                &context,
                capture(
                    &format!("unique-{index}"),
                    format!("bytes-{index}").as_bytes(),
                ),
            )
        }));
    }
    barrier.wait();
    let mut sequences = joins
        .into_iter()
        .map(|join| join.join().unwrap().unwrap().event.event_seq)
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=20).collect::<Vec<_>>());

    let a_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let b_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let a_context = context.clone();
    let b_context = context.clone();
    let same_a =
        thread::spawn(move || a_store.capture(&a_context, capture("race-same", b"identical")));
    let same_b =
        thread::spawn(move || b_store.capture(&b_context, capture("race-same", b"identical")));
    let a = same_a.join().unwrap().unwrap();
    let b = same_b.join().unwrap().unwrap();
    assert_eq!(a.canonical_bytes().unwrap(), b.canonical_bytes().unwrap());

    let a_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let b_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let a_context = context.clone();
    let b_context = context.clone();
    let conflict_a =
        thread::spawn(move || a_store.capture(&a_context, capture("race-conflict", b"a")));
    let conflict_b =
        thread::spawn(move || b_store.capture(&b_context, capture("race-conflict", b"b")));
    let results = [conflict_a.join().unwrap(), conflict_b.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(SemanticError::IdempotencyConflict)))
            .count(),
        1
    );

    let diagnostics = store.diagnostics().unwrap();
    assert_eq!(diagnostics.events, 22);
    assert_eq!(diagnostics.operations, 22);
    assert_eq!(diagnostics.event_sequences, (1..=22).collect::<Vec<_>>());
    drop(second);
}

fn run_crash_child(parent: &Path, root: &Path, failpoint: &str, operation: &str, bytes: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("crash_worker")
        .arg("--nocapture")
        .env("SEMANTIC_CRASH_WORKER", "1")
        .env("SEMANTIC_ALLOWED_PARENT", parent)
        .env("SEMANTIC_ROOT", root)
        .env("SEMANTIC_TEST_FAILPOINT", failpoint)
        .env("SEMANTIC_OPERATION", operation)
        .env("SEMANTIC_BYTES", bytes)
        .status()
        .expect("spawn crash child");
    assert!(
        !status.success(),
        "failpoint {failpoint} did not terminate abruptly"
    );
}

#[test]
fn crash_worker() {
    if std::env::var_os("SEMANTIC_CRASH_WORKER").is_none() {
        return;
    }
    let parent = PathBuf::from(std::env::var_os("SEMANTIC_ALLOWED_PARENT").unwrap());
    let root = PathBuf::from(std::env::var_os("SEMANTIC_ROOT").unwrap());
    let store = if root.exists() {
        SemanticStore::open(&root, enabled(&parent)).unwrap()
    } else {
        SemanticStore::create(&root, enabled(&parent)).unwrap().0
    };
    let context = store.trusted_context();
    let operation = std::env::var("SEMANTIC_OPERATION").unwrap();
    let bytes = std::env::var("SEMANTIC_BYTES").unwrap();
    let _ = store.capture(&context, capture(&operation, bytes.as_bytes()));
    panic!("configured failpoint did not abort the process");
}

#[test]
fn abrupt_failpoint_matrix_has_atomic_recovery_and_effectively_once_projection() {
    let pre_commit = [
        "after_idempotency_reservation",
        "after_object_temp_flush",
        "after_object_rename",
        "after_event_insert",
        "after_outbox_insert",
        "after_stored_response",
    ];
    for failpoint in pre_commit {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("store");
        run_crash_child(parent.path(), &root, failpoint, "crash-op", "crash bytes");
        let store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
        store.recover(ManualRecovery).unwrap();
        assert_eq!(store.diagnostics().unwrap(), StoreDiagnostics::empty());
    }

    let post_commit = [
        "after_db_commit_before_projection",
        "after_projection_temp_flush",
        "after_snapshot_rename_before_outbox_ack",
    ];
    for failpoint in post_commit {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("store");
        run_crash_child(parent.path(), &root, failpoint, "crash-op", "crash bytes");
        let store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
        let context = store.trusted_context();
        let before = store.diagnostics().unwrap();
        assert_eq!((before.events, before.operations), (1, 1));
        store.recover(ManualRecovery).unwrap();
        let first = store
            .capture(&context, capture("crash-op", b"crash bytes"))
            .unwrap();
        let second = store
            .capture(&context, capture("crash-op", b"crash bytes"))
            .unwrap();
        assert_eq!(
            first.canonical_bytes().unwrap(),
            second.canonical_bytes().unwrap()
        );
        let after = store.diagnostics().unwrap();
        assert_eq!(
            (after.events, after.operations, after.outbox_pending),
            (1, 1, 0)
        );
        assert_eq!(after.ledger_checksum, after.projection_checksum);
        assert_eq!(store.projection_state().unwrap().event_count, 1);
    }
}

#[test]
fn recovery_preserves_shared_objects_and_is_checksum_idempotent() {
    let (parent, root, store, context) = fixture();
    let first = store
        .capture(&context, capture("shared-first", b"deduplicated"))
        .unwrap();
    let object_id = first.event.payload.object_id.clone();
    let stable = store.diagnostics().unwrap();
    drop(store);

    run_crash_child(
        parent.path(),
        &root,
        "after_object_rename",
        "shared-crash",
        "deduplicated",
    );
    let reopened = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    reopened.recover(ManualRecovery).unwrap();
    let once = reopened.diagnostics().unwrap();
    reopened.recover(ManualRecovery).unwrap();
    let twice = reopened.diagnostics().unwrap();
    assert_eq!(once, twice);
    assert_eq!(once.events, stable.events);
    assert!(reopened.object_exists(&object_id));
    assert_eq!(
        reopened.projection_state().unwrap(),
        ProjectionState::from_diagnostics(&once)
    );
}

#[test]
fn rollback_capability_is_bound_rejects_live_handles_and_is_repeatable() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let legacy = parent.path().join("legacy-wiki");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("README.md"), "legacy byte identity").unwrap();
    let legacy_before = fs::read(legacy.join("README.md")).unwrap();

    let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
    let other = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    assert!(matches!(
        admin.rollback(),
        Err(SemanticError::ActiveHandles)
    ));
    drop(other);
    assert!(matches!(
        admin.rollback(),
        Err(SemanticError::ActiveHandles)
    ));
    drop(store);
    assert_eq!(admin.rollback().unwrap(), RollbackStatus::Removed);
    assert_eq!(admin.rollback().unwrap(), RollbackStatus::AlreadyRemoved);
    assert!(!root.exists());
    assert_eq!(fs::read(legacy.join("README.md")).unwrap(), legacy_before);

    let outside = tempfile::tempdir().unwrap();
    let error =
        SemanticStore::create(&outside.path().join("store"), enabled(parent.path())).unwrap_err();
    assert!(matches!(error, SemanticError::InvalidRoot(_)));
    let repo_error = SemanticStore::create(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        enabled(parent.path()),
    )
    .unwrap_err();
    assert!(matches!(repo_error, SemanticError::InvalidRoot(_)));
}

#[test]
fn rollback_rejects_missing_or_tampered_marker_without_deleting() {
    for mode in ["missing", "tampered"] {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("store");
        let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
        drop(store);
        let marker = root.join("store.marker.json");
        let original = fs::read(&marker).unwrap();
        if mode == "missing" {
            fs::remove_file(&marker).unwrap();
        } else {
            fs::write(&marker, b"{\"store_uuid\":\"copied-or-wrong\"}").unwrap();
        }
        assert!(matches!(
            admin.rollback(),
            Err(SemanticError::MarkerMismatch)
        ));
        assert!(root.exists());
        fs::write(&marker, original).unwrap();
        assert_eq!(admin.rollback().unwrap(), RollbackStatus::Removed);
    }
}

#[cfg(unix)]
#[test]
fn symlink_root_is_rejected() {
    use std::os::unix::fs::symlink;
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = parent.path().join("store-link");
    symlink(&target, &link).unwrap();
    assert!(matches!(
        SemanticStore::create(&link, enabled(parent.path())),
        Err(SemanticError::InvalidRoot(_))
    ));
}

#[test]
fn semantic_module_is_isolated_and_legacy_runtime_does_not_call_writer() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let semantic = fs::read_to_string(repo.join("src/semantic.rs")).unwrap();
    for forbidden in [
        "crate::ops",
        "crate::markdown",
        "crate::git",
        "crate::index_manager",
        "crate::mcp",
        "crate::server",
        "tantivy",
        "petgraph",
    ] {
        assert!(
            !semantic.contains(forbidden),
            "semantic core couples to {forbidden}"
        );
    }
    for entry in fs::read_dir(repo.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().and_then(|name| name.to_str()) == Some("semantic.rs")
            || path.file_name().and_then(|name| name.to_str()) == Some("lib.rs")
            || !path.is_file()
        {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        assert!(
            !source.contains("semantic::"),
            "legacy runtime calls semantic writer: {}",
            path.display()
        );
    }
}
