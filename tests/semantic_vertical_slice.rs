use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(feature = "semantic-test-failpoints")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{DateTime, TimeZone, Utc};
use jsonschema::validator_for;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, ManualRecovery, MutationOutcome, PrivacyLabel,
    ProposeCommand, RollbackStatus, SemanticClock, SemanticConfig, SemanticError, SemanticStore,
    TrustedContext, canonicalize_json, event_hash_from_value,
};
#[cfg(feature = "semantic-test-failpoints")]
use llm_wiki::semantic::{ProjectionState, StoreDiagnostics};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::{Uuid, Version};

const EVENT_SCHEMA: &str = include_str!("../evals/v1/contracts/event-schema-v1.json");
#[cfg(feature = "semantic-test-failpoints")]
static CRASH_PROCESS_LOCK: Mutex<()> = Mutex::new(());

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
        claim_kind: "project_decision".to_owned(),
        domain: "projects".to_owned(),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from,
        valid_to,
    }
}

fn run_git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn file_manifest(root: &Path) -> BTreeMap<String, String> {
    fn visit(root: &Path, current: &Path, output: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                visit(root, &path, output);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                output.insert(
                    relative,
                    hex::encode(Sha256::digest(fs::read(path).unwrap())),
                );
            }
        }
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn object_path(root: &Path, object_id: &str) -> PathBuf {
    let digest = object_id.strip_prefix("sha256:").unwrap();
    root.join("objects").join(&digest[..2]).join(digest)
}

#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) {
    let output = Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("create Windows junction");
    assert!(
        output.status.success(),
        "mklink failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
        Some(at("2026-07-13T03:00:00Z")),
        Some(at("2026-07-14T03:00:00Z")),
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

    let proposal_object = store
        .object_json(&proposed.event.payload.object_id)
        .unwrap();
    assert_eq!(proposal_object["provenance"]["kind"], "evidence");
    assert_eq!(proposal_object["provenance"]["byte_start"], 0);
    assert_eq!(proposal_object["provenance"]["byte_end"], 15);
    assert_eq!(
        proposal_object["provenance"]["object_id"],
        captured.event.payload.object_id
    );
    assert_eq!(
        proposal_object["provenance"]["quote_hash"],
        captured
            .event
            .payload
            .object_id
            .trim_start_matches("sha256:")
    );
    let claim_object = store
        .object_json(&confirmed.event.payload.object_id)
        .unwrap();
    assert_eq!(claim_object["claim"]["status"], "confirmed");
    assert_eq!(claim_object["claim"]["claim_kind"], "project_decision");
    assert_eq!(claim_object["claim"]["domain"], "projects");
    assert_eq!(claim_object["claim"]["confidence_basis_points"], 9_000);
    assert_eq!(claim_object["claim"]["privacy_label"], "local_only");
    assert_eq!(claim_object["claim"]["supersedes"], json!([]));
    assert_eq!(claim_object["claim"]["retracts"], json!([]));
    assert_eq!(
        claim_object["claim"]["recorded_event_id"],
        confirmed.event.event_id.to_string()
    );
    assert_eq!(claim_object["claim"]["recorded_event_seq"], 3);

    assert!(
        store
            .claim_at(2, at("2026-07-13T02:00:00Z"))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .claim_at(3, at("2026-07-13T02:59:59Z"))
            .unwrap()
            .is_none()
    );
    let current = store
        .claim_at(3, at("2026-07-13T03:00:00Z"))
        .unwrap()
        .unwrap();
    assert_eq!(current.claim_id, confirmed.generated.claim_id.unwrap());
    assert_eq!(current.status, "confirmed");
    assert_eq!(current.privacy_label, PrivacyLabel::LocalOnly);
    assert!(
        store
            .claim_at(3, at("2026-07-14T03:00:00Z"))
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
        "numbers": [333_333_333.333_333_3_f64, 1E30_f64, 4.50_f64, 2e-3_f64, 1e-27_f64],
        "string": "€$\u{000f}\nA'B\"\\\\\"/",
        "literals": [null, true, false]
    });
    let expected = "{\"literals\":[null,true,false],\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27],\"string\":\"€$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}";
    assert_eq!(canonicalize_json(&vector).unwrap(), expected.as_bytes());

    let fixed_event = json!({
        "schema_version": 1,
        "event_id": "01890f7e-3c00-7000-8000-000000000001",
        "owner_id": "01890f7e-3c00-7000-8000-000000000002",
        "event_seq": 2,
        "event_type": "claim_confirmed",
        "recorded_at": "2026-07-13T02:00:00Z",
        "actor_id": "01890f7e-3c00-7000-8000-000000000003",
        "client_id": "01890f7e-3c00-7000-8000-000000000004",
        "operation_id": "fixed-chain-vector",
        "request_hash": "1111111111111111111111111111111111111111111111111111111111111111",
        "payload": {"kind":"object_ref","object_id":format!("sha256:{}", "2".repeat(64)),"media_type":"application/vnd.brain.semantic+json"},
        "prior_event_hash": "3333333333333333333333333333333333333333333333333333333333333333",
        "event_hash": "this field is deliberately omitted by the hash contract",
        "purge_epoch": 0
    });
    assert_eq!(
        event_hash_from_value(&fixed_event).unwrap(),
        "47750a496d582f4c10c374ff0e35552250bc63b9b81ab2623e30ab96ea4ae584"
    );

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
fn deserialized_commands_reject_all_identity_and_domain_id_injection() {
    for injected in [
        "owner_id",
        "event_id",
        "actor_id",
        "client_id",
        "source_id",
        "rendition_id",
        "evidence_id",
        "proposal_id",
        "claim_id",
        "status",
        "provenance",
        "supersedes",
        "retracts",
    ] {
        let injected_value = json!("01890f7e-3c00-7000-8000-000000000099");
        let mut capture_command = json!({
            "operation_id": "injection-test",
            "bytes": [115, 97, 102, 101],
            "media_type": "text/plain"
        });
        capture_command[injected] = injected_value.clone();
        assert!(
            serde_json::from_value::<CaptureCommand>(capture_command).is_err(),
            "capture command accepted injected {injected}"
        );

        let mut propose_command = json!({
            "operation_id": "injection-propose",
            "capture_operation_id": "capture",
            "draft": {
                "subject": "project:brain",
                "predicate": "deployment",
                "value": "sqlite",
                "claim_kind": "project_decision",
                "domain": "projects",
                "confidence_basis_points": 9000,
                "privacy_label": "local_only",
                "valid_from": null,
                "valid_to": null
            }
        });
        propose_command[injected] = injected_value.clone();
        assert!(
            serde_json::from_value::<ProposeCommand>(propose_command).is_err(),
            "propose command accepted top-level injected {injected}"
        );
        let mut nested_propose = json!({
            "operation_id": "injection-propose",
            "capture_operation_id": "capture",
            "draft": {
                "subject": "project:brain",
                "predicate": "deployment",
                "value": "sqlite",
                "claim_kind": "project_decision",
                "domain": "projects",
                "confidence_basis_points": 9000,
                "privacy_label": "local_only",
                "valid_from": null,
                "valid_to": null
            }
        });
        nested_propose["draft"][injected] = injected_value.clone();
        assert!(
            serde_json::from_value::<ProposeCommand>(nested_propose).is_err(),
            "claim draft accepted injected {injected}"
        );

        let mut confirm_command = json!({
            "operation_id": "injection-confirm",
            "proposal_operation_id": "proposal"
        });
        confirm_command[injected] = injected_value;
        assert!(
            serde_json::from_value::<ConfirmCommand>(confirm_command).is_err(),
            "confirm command accepted injected {injected}"
        );
    }
}

#[test]
fn invalid_claim_metadata_cannot_create_evidence_less_proposal() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("invalid-capture", b"evidence"))
        .unwrap();
    let mut invalid = draft(None, None);
    invalid.confidence_basis_points = 10_001;
    let result = store.propose(
        &context,
        ProposeCommand {
            operation_id: "invalid-proposal".to_owned(),
            capture_operation_id: "invalid-capture".to_owned(),
            draft: invalid,
        },
    );
    assert!(matches!(result, Err(SemanticError::InvalidClaim(_))));
    assert_eq!(store.diagnostics().unwrap().events, 1);

    let mut publishable = draft(None, None);
    publishable.privacy_label = PrivacyLabel::Publishable;
    let result = store.propose(
        &context,
        ProposeCommand {
            operation_id: "invalid-privacy-proposal".to_owned(),
            capture_operation_id: "invalid-capture".to_owned(),
            draft: publishable,
        },
    );
    assert!(matches!(result, Err(SemanticError::InvalidClaim(_))));
    assert_eq!(store.diagnostics().unwrap().events, 1);
}

#[test]
fn empty_or_invalid_utf8_capture_cannot_become_a_confirmed_evidence_claim() {
    for (suffix, bytes) in [("empty", Vec::new()), ("invalid-utf8", vec![0xff, 0xfe])] {
        let (_parent, _root, store, context) = fixture();
        let capture_operation = format!("{suffix}-capture");
        let proposal_operation = format!("{suffix}-proposal");
        store
            .capture(&context, capture(&capture_operation, &bytes))
            .unwrap();

        let proposal = store.propose(
            &context,
            ProposeCommand {
                operation_id: proposal_operation.clone(),
                capture_operation_id: capture_operation,
                draft: draft(None, None),
            },
        );
        assert!(matches!(proposal, Err(SemanticError::InvalidClaim(_))));
        assert!(matches!(
            store.confirm(
                &context,
                ConfirmCommand {
                    operation_id: format!("{suffix}-confirm"),
                    proposal_operation_id: proposal_operation,
                }
            ),
            Err(SemanticError::MissingDependency(_))
        ));
        assert_eq!(store.diagnostics().unwrap().events, 1);
    }

    let (_parent, _root, store, context) = fixture();
    let rendition = "หลักฐาน".as_bytes();
    let captured = store
        .capture(&context, capture("utf8-capture", rendition))
        .unwrap();
    let proposed = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "utf8-proposal".to_owned(),
                capture_operation_id: "utf8-capture".to_owned(),
                draft: draft(None, None),
            },
        )
        .unwrap();
    let provenance = store
        .object_json(&proposed.event.payload.object_id)
        .unwrap()["provenance"]
        .clone();
    assert_eq!(provenance["byte_start"], 0);
    assert_eq!(provenance["byte_end"], rendition.len());
    assert_eq!(
        provenance["quote_hash"],
        hex::encode(Sha256::digest(rendition))
    );
    assert_eq!(provenance["object_id"], captured.event.payload.object_id);
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "utf8-confirm".to_owned(),
                proposal_operation_id: "utf8-proposal".to_owned(),
            },
        )
        .unwrap();
}

#[test]
fn confirmation_rejects_a_tampered_evidence_less_proposal_object() {
    let (_parent, root, store, context) = fixture();
    store
        .capture(
            &context,
            capture("tampered-capture", b"evidence is required"),
        )
        .unwrap();
    let proposal = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "tampered-proposal".to_owned(),
                capture_operation_id: "tampered-capture".to_owned(),
                draft: draft(None, None),
            },
        )
        .unwrap();
    fs::write(
        object_path(&root, &proposal.event.payload.object_id),
        br#"{"kind":"claim_proposal"}"#,
    )
    .unwrap();

    let result = store.confirm(
        &context,
        ConfirmCommand {
            operation_id: "tampered-confirm".to_owned(),
            proposal_operation_id: "tampered-proposal".to_owned(),
        },
    );
    // Tampering now hits AEAD authentication during decryption (Task 1.2b)
    // rather than a plaintext-checksum mismatch after a successful decrypt,
    // so the object is reported unavailable rather than corrupt.
    assert!(matches!(result, Err(SemanticError::ObjectUnavailable(_))));
    assert_eq!(store.diagnostics().unwrap().events, 2);
}

#[test]
fn sqlite_authority_pragmas_schema_and_ledger_tamper_guard_are_executable() {
    let (_parent, root, store, context) = fixture();
    let pragmas = store.storage_pragmas().unwrap();
    assert_eq!(pragmas.journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(pragmas.synchronous, 2);
    assert_eq!(pragmas.foreign_keys, 1);
    assert_eq!(pragmas.busy_timeout_ms, 5_000);

    store
        .capture(&context, capture("tamper-guard", b"ledger integrity"))
        .unwrap();
    let connection = rusqlite::Connection::open(root.join("semantic.sqlite3")).unwrap();
    for table in ["events", "operations", "outbox"] {
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "missing authority table {table}");
    }
    connection
        .execute(
            "UPDATE events SET event_hash=?1 WHERE event_seq=1",
            ["ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"],
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        store.recover(ManualRecovery),
        Err(SemanticError::CorruptLedger(_))
    ));
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
            barrier.wait();
            let independent = SemanticStore::open(&root, enabled(&parent))
                .map_err(|error| format!("open failed: {error:?}"))?;
            independent
                .capture(
                    &context,
                    capture(
                        &format!("unique-{index}"),
                        format!("bytes-{index}").as_bytes(),
                    ),
                )
                .map_err(|error| format!("capture failed: {error:?}"))
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

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn recovery_queued_during_committed_mutation_does_not_deadlock() {
    let (parent, root, store, context) = fixture();
    let mutation_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let recovery_store = SemanticStore::open(&root, enabled(parent.path())).unwrap();
    let pause = mutation_store.pause_after_commit_for_test();

    let (mutation_tx, mutation_rx) = std::sync::mpsc::sync_channel(1);
    let mutation_context = context.clone();
    let mutation_thread = thread::spawn(move || {
        let result = mutation_store.capture(
            &mutation_context,
            capture("recovery-overlap", b"committed evidence"),
        );
        mutation_tx.send(result).unwrap();
    });
    assert!(pause.wait_until_entered(std::time::Duration::from_secs(5)));

    let (recovery_tx, recovery_rx) = std::sync::mpsc::sync_channel(1);
    let recovery_thread = thread::spawn(move || {
        let result = recovery_store.recover(ManualRecovery);
        recovery_tx.send(result).unwrap();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !store.recovery_blocked_for_test() && std::time::Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(
        store.recovery_blocked_for_test(),
        "recovery did not queue behind the mutation maintenance guard"
    );

    pause.release();
    mutation_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("mutation deadlocked with queued recovery")
        .unwrap();
    recovery_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("recovery did not converge after mutation")
        .unwrap();
    mutation_thread.join().unwrap();
    recovery_thread.join().unwrap();

    let diagnostics = store.diagnostics().unwrap();
    assert_eq!((diagnostics.events, diagnostics.operations), (1, 1));
    assert_eq!(diagnostics.outbox_pending, 0);
    assert_eq!(diagnostics.ledger_checksum, diagnostics.projection_checksum);
}

#[cfg(feature = "semantic-test-failpoints")]
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

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn abrupt_failpoint_matrix_has_atomic_recovery_and_effectively_once_projection() {
    let _crash_process_guard = CRASH_PROCESS_LOCK.lock().unwrap();
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

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn recovery_preserves_shared_objects_and_is_checksum_idempotent() {
    let _crash_process_guard = CRASH_PROCESS_LOCK.lock().unwrap();
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
    fs::create_dir(legacy.join("notes")).unwrap();
    fs::write(legacy.join("notes/reference.md"), "existing knowledge").unwrap();
    run_git(&legacy, &["init", "--quiet"]);
    run_git(&legacy, &["config", "user.name", "Semantic Test"]);
    run_git(
        &legacy,
        &["config", "user.email", "semantic@example.invalid"],
    );
    run_git(&legacy, &["add", "."]);
    run_git(&legacy, &["commit", "--quiet", "-m", "fixture"]);
    let legacy_head_before = run_git(&legacy, &["rev-parse", "HEAD"]);
    let legacy_before = file_manifest(&legacy);

    let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
    let store_debug = format!("{store:?}");
    let admin_debug = format!("{admin:?}");
    assert_eq!(store_debug, "SemanticStore { capability: \"<redacted>\" }");
    assert_eq!(admin_debug, "StoreAdmin { capability: \"<redacted>\" }");
    assert!(!store_debug.contains(&root.to_string_lossy().to_string()));
    assert!(!admin_debug.contains(&root.to_string_lossy().to_string()));
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
    assert_eq!(run_git(&legacy, &["rev-parse", "HEAD"]), legacy_head_before);
    assert_eq!(file_manifest(&legacy), legacy_before);

    let outside = tempfile::tempdir().unwrap();
    let error =
        SemanticStore::create(outside.path().join("store"), enabled(parent.path())).unwrap_err();
    assert!(matches!(error, SemanticError::InvalidRoot(_)));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_error = SemanticStore::create(repo, enabled(repo.parent().unwrap())).unwrap_err();
    assert!(matches!(repo_error, SemanticError::InvalidRoot(_)));
    let git_error = SemanticStore::create(repo.join(".git"), enabled(repo)).unwrap_err();
    assert!(matches!(git_error, SemanticError::InvalidRoot(_)));

    let filesystem_root = parent.path().ancestors().last().unwrap();
    let filesystem_error = SemanticStore::create(
        filesystem_root.join(format!("semantic-test-{}", Uuid::now_v7())),
        enabled(filesystem_root),
    )
    .unwrap_err();
    assert!(matches!(filesystem_error, SemanticError::InvalidRoot(_)));
}

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn rollback_rejects_an_active_transaction_after_the_store_handle_is_dropped() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
    let transaction = store.begin_test_transaction().unwrap();
    drop(store);

    assert!(matches!(
        admin.rollback(),
        Err(SemanticError::ActiveHandles)
    ));
    assert!(root.exists());
    drop(transaction);
    assert_eq!(admin.rollback().unwrap(), RollbackStatus::Removed);
}

#[test]
fn rollback_rejects_missing_or_tampered_marker_without_deleting() {
    for mode in ["missing", "wrong_store_uuid", "wrong_deletion_nonce"] {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("store");
        let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
        drop(store);
        let marker = root.join("store.marker.json");
        let original = fs::read(&marker).unwrap();
        match mode {
            "missing" => fs::remove_file(&marker).unwrap(),
            "wrong_store_uuid" | "wrong_deletion_nonce" => {
                let mut copied: Value = serde_json::from_slice(&original).unwrap();
                copied[if mode == "wrong_store_uuid" {
                    "store_uuid"
                } else {
                    "deletion_nonce"
                }] = json!(Uuid::now_v7());
                fs::write(&marker, serde_json::to_vec(&copied).unwrap()).unwrap();
            }
            _ => unreachable!(),
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

    fs::remove_file(&link).unwrap();
    let root = parent.path().join("semantic-store");
    let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
    drop(store);
    let outside = parent.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), "do not delete").unwrap();
    let interior = root.join("escape");
    symlink(&outside, &interior).unwrap();
    assert!(matches!(
        admin.rollback(),
        Err(SemanticError::InvalidRoot(_))
    ));
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "do not delete"
    );
    fs::remove_file(interior).unwrap();
    assert_eq!(admin.rollback().unwrap(), RollbackStatus::Removed);
}

#[cfg(windows)]
#[test]
fn windows_root_and_interior_reparse_points_are_rejected() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("junction-target");
    fs::create_dir(&target).unwrap();
    let link = parent.path().join("store-link");
    make_junction(&link, &target);
    assert!(matches!(
        SemanticStore::create(&link, enabled(parent.path())),
        Err(SemanticError::InvalidRoot(_))
    ));
    fs::remove_dir(&link).unwrap();

    let root = parent.path().join("semantic-store");
    let (store, admin) = SemanticStore::create(&root, enabled(parent.path())).unwrap();
    drop(store);
    let outside = parent.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), "do not delete").unwrap();
    let interior = root.join("escape");
    make_junction(&interior, &outside);
    assert!(matches!(
        admin.rollback(),
        Err(SemanticError::InvalidRoot(_))
    ));
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "do not delete"
    );
    fs::remove_dir(interior).unwrap();
    assert_eq!(admin.rollback().unwrap(), RollbackStatus::Removed);
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
        let file_name = path.file_name().and_then(|name| name.to_str());
        // src/projection.rs (Task 2.1) is the one authorized bridge between
        // the canonical semantic layers and the legacy Tantivy/Petgraph/
        // Markdown runtime: it reads claims via SemanticStore to build
        // projections. Every other legacy file remains forbidden from
        // referencing `semantic::` — this allowlist is intentionally one
        // entry wide, not a relaxation of the isolation guarantee itself.
        if matches!(file_name, Some("semantic.rs") | Some("lib.rs") | Some("projection.rs"))
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
