//! Task 1.3. Sub-slice B: PurgeRegistry primitive and deny-first
//! fail-closed reads. `append_registry_denial`/`sync_purge_registry` are
//! raw primitives here (capability-free, not nonce-bound) -- sub-slice C's
//! hard-purge saga wires authorization, idempotency, and orchestration
//! around them, matching the Task 1.2b precedent for
//! `destroy_wrapped_key`/`rotate_epoch_and_rewrap`.
//!
//! Sub-slice C: the full hard-purge saga (`purge_preview`/`purge_execute`/
//! `purge_resume`) built on top of sub-slice B's registry primitive and
//! Task 1.2b's encryption primitives -- capability-gated, nonce-bound,
//! idempotent, and crash/retry-safe at every step boundary (ADR Decision 7).

use std::path::{Path, PathBuf};
#[cfg(feature = "semantic-test-failpoints")]
use std::sync::atomic::{AtomicI64, Ordering};
#[cfg(feature = "semantic-test-failpoints")]
use std::time::Duration;

use chrono::Utc;
#[cfg(feature = "semantic-test-failpoints")]
use chrono::{DateTime, TimeZone};
#[cfg(feature = "semantic-test-failpoints")]
use llm_wiki::semantic::SemanticClock;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticError, SemanticStore, TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

fn enabled_with_targets(parent: &Path, targets: Vec<PathBuf>) -> SemanticConfig {
    SemanticConfig::enabled_for(parent).with_purge_registry_targets(targets)
}

fn fixture() -> (TempDir, PathBuf, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, root, store, context)
}

fn fixture_with_targets(
    targets: Vec<PathBuf>,
) -> (TempDir, PathBuf, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, enabled_with_targets(parent.path(), targets)).expect("create");
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
        value: json!("kubernetes"),
        claim_kind: "decision".to_owned(),
        domain: Some("projects".to_owned()),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Confirms a claim end to end and returns the confirmed object's ID --
/// exactly the plaintext-returning identifier `append_registry_denial` and
/// `decrypt_object`'s deny gate operate on.
fn confirm_a_claim(store: &SemanticStore, context: &TrustedContext) -> String {
    store
        .capture(context, capture("cap", b"evidence bytes"))
        .unwrap();
    store
        .propose(
            context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    let confirmed = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
    confirmed.event.payload.object_id
}

#[test]
fn append_registry_denial_requires_at_least_two_targets() {
    let (_parent, _root, store, _context) = fixture();
    assert!(matches!(
        store.append_registry_denial(&["sha256:deadbeef".to_owned()]),
        Err(SemanticError::InvalidRoot(_))
    ));
    assert_eq!(store.registry_epoch().unwrap(), 0);

    let parent = tempfile::tempdir().unwrap();
    let one_target = parent.path().join("target-a");
    std::fs::create_dir_all(&one_target).unwrap();
    let (_parent2, _root2, store2, _context2) = fixture_with_targets(vec![one_target]);
    assert!(matches!(
        store2.append_registry_denial(&["sha256:deadbeef".to_owned()]),
        Err(SemanticError::InvalidRoot(_))
    ));
    assert_eq!(store2.registry_epoch().unwrap(), 0);
}

#[test]
fn denying_an_object_id_fails_closed_on_object_json_and_claims_current() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let target_b = targets_parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();
    let (_parent, _root, store, context) = fixture_with_targets(vec![target_a, target_b]);

    let object_id = confirm_a_claim(&store, &context);
    assert!(store.object_json(&object_id).is_ok());

    let epoch = store
        .append_registry_denial(std::slice::from_ref(&object_id))
        .unwrap();
    assert_eq!(epoch, 1);
    assert!(store.is_denied(&object_id).unwrap());

    assert!(matches!(
        store.object_json(&object_id),
        Err(SemanticError::Denied(denied)) if denied == object_id
    ));

    let ledger_head = store.ledger_head().unwrap();
    assert!(matches!(
        store.claims_current(
            ledger_head,
            Utc::now(),
            "projects",
            "project:brain",
            "deployment"
        ),
        Err(SemanticError::Denied(_))
    ));
}

#[test]
fn registry_epoch_increments_and_denied_ids_accumulate_across_calls() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let target_b = targets_parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();
    let (_parent, _root, store, _context) = fixture_with_targets(vec![target_a, target_b]);

    let first = store
        .append_registry_denial(&[
            "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
        ])
        .unwrap();
    let second = store
        .append_registry_denial(&[
            "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
        ])
        .unwrap();
    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(store.registry_epoch().unwrap(), 2);
    assert!(
        store
            .is_denied("sha256:1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap()
    );
    assert!(
        store
            .is_denied("sha256:2222222222222222222222222222222222222222222222222222222222222222")
            .unwrap()
    );
}

/// Deny-first: a replication shortfall must never roll back the local
/// denial. Three targets are configured (quorum = 2), but two of three
/// targets are unusable, so only 1 of 3 (below quorum 2) acknowledges. The
/// local commit -- and this store's own fail-closed read protection --
/// still takes effect immediately; only the replication guarantee is
/// reported as failed.
#[test]
fn quorum_shortfall_below_threshold_still_denies_locally_but_reports_failure() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let unusable_b = targets_parent.path().join("unusable-b");
    let unusable_c = targets_parent.path().join("unusable-c");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::write(&unusable_b, b"file, not a directory").unwrap();
    std::fs::write(&unusable_c, b"file, not a directory").unwrap();

    let (_parent, _root, store, context) =
        fixture_with_targets(vec![target_a, unusable_b, unusable_c]);
    let object_id = confirm_a_claim(&store, &context);

    assert!(matches!(
        store.append_registry_denial(std::slice::from_ref(&object_id)),
        Err(SemanticError::RegistryQuorumFailed(_))
    ));
    // Local denial still committed and still protects this store's reads.
    assert_eq!(store.registry_epoch().unwrap(), 1);
    assert!(store.is_denied(&object_id).unwrap());
    assert!(matches!(
        store.object_json(&object_id),
        Err(SemanticError::Denied(_))
    ));
}

/// A store opened with configured targets that report an epoch ahead of its
/// own local copy (e.g. a backup restore, or a sibling store that already
/// advanced the registry) must open sealed and refuse every plaintext read
/// until `sync_purge_registry` succeeds.
#[test]
fn store_opens_sealed_when_behind_a_reachable_quorum_and_unseals_after_sync() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let target_b = targets_parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();

    // Store A advances the registry using the two shared targets.
    let (parent_a, _root_a, store_a, context_a) =
        fixture_with_targets(vec![target_a.clone(), target_b.clone()]);
    let object_id = confirm_a_claim(&store_a, &context_a);
    store_a
        .append_registry_denial(std::slice::from_ref(&object_id))
        .unwrap();
    drop(store_a);
    drop(parent_a);

    // Store B is a fresh, unrelated store that happens to share the same
    // replication targets and has never applied this denial locally.
    let parent_b = tempfile::tempdir().unwrap();
    let root_b = parent_b.path().join("semantic-store");
    let (store_b, _admin_b) =
        SemanticStore::create(&root_b, enabled(parent_b.path())).expect("create store b");
    let context_b = store_b.trusted_context();
    let object_id_b = confirm_a_claim(&store_b, &context_b);
    drop(store_b);

    let reopened = SemanticStore::open(
        &root_b,
        enabled_with_targets(parent_b.path(), vec![target_a.clone(), target_b.clone()]),
    )
    .unwrap();
    assert!(reopened.is_registry_sealed().unwrap());
    assert!(matches!(
        reopened.object_json(&object_id_b),
        Err(SemanticError::RegistrySealed(_))
    ));

    reopened.sync_purge_registry().unwrap();
    assert!(!reopened.is_registry_sealed().unwrap());
    assert_eq!(reopened.registry_epoch().unwrap(), 1);
    assert!(reopened.is_denied(&object_id).unwrap());
    // The store's own, unrelated confirmed object is unaffected.
    assert!(reopened.object_json(&object_id_b).is_ok());
    assert!(matches!(
        reopened.object_json(&object_id),
        Err(SemanticError::Denied(_))
    ));
}

/// A store opened with configured targets that are all unreachable (quorum
/// cannot be established) must also seal, even though it has never denied
/// anything locally -- an unreachable quorum can hide a denial made
/// elsewhere.
#[test]
fn store_opens_sealed_when_no_target_quorum_is_reachable() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let target_b = targets_parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();

    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(
        &root,
        enabled_with_targets(parent.path(), vec![target_a.clone(), target_b.clone()]),
    )
    .expect("create");
    drop(store);

    // Both targets vanish before the next open (simulating both replicas
    // being unreachable).
    std::fs::remove_dir_all(&target_a).unwrap();
    std::fs::remove_dir_all(&target_b).unwrap();

    let reopened = SemanticStore::open(
        &root,
        enabled_with_targets(parent.path(), vec![target_a, target_b]),
    )
    .unwrap();
    assert!(reopened.is_registry_sealed().unwrap());
    assert!(matches!(
        reopened.sync_purge_registry(),
        Err(SemanticError::RegistryQuorumFailed(_))
    ));
    assert!(reopened.is_registry_sealed().unwrap());
}

/// A tampered target entry (denied_ids/entry_hash internally inconsistent)
/// is rejected during sync as a broken hash chain, and the store stays
/// sealed rather than silently applying corrupted denial data.
#[test]
fn sync_purge_registry_rejects_a_tampered_entry_and_stays_sealed() {
    let targets_parent = tempfile::tempdir().unwrap();
    let target_a = targets_parent.path().join("target-a");
    let target_b = targets_parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();

    let (parent_a, _root_a, store_a, context_a) =
        fixture_with_targets(vec![target_a.clone(), target_b.clone()]);
    let object_id = confirm_a_claim(&store_a, &context_a);
    store_a.append_registry_denial(&[object_id]).unwrap();
    drop(store_a);
    drop(parent_a);

    // Tamper target_a's epoch-1 entry: keep valid JSON shape, but corrupt
    // the hash so it no longer matches its own declared fields.
    let mut entries: Vec<_> = std::fs::read_dir(&target_a)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    let epoch_file = entries.pop().unwrap();
    let raw = std::fs::read_to_string(&epoch_file).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["entry_hash"] = json!("sha256:tampered-hash-value-does-not-match-fields");
    std::fs::write(&epoch_file, serde_json::to_vec(&value).unwrap()).unwrap();

    let parent_b = tempfile::tempdir().unwrap();
    let root_b = parent_b.path().join("semantic-store");
    let (store_b, _admin_b) =
        SemanticStore::create(&root_b, enabled(parent_b.path())).expect("create store b");
    drop(store_b);

    let reopened = SemanticStore::open(
        &root_b,
        enabled_with_targets(parent_b.path(), vec![target_a, target_b]),
    )
    .unwrap();
    assert!(reopened.is_registry_sealed().unwrap());
    assert!(matches!(
        reopened.sync_purge_registry(),
        Err(SemanticError::CorruptLedger(_))
    ));
    assert!(reopened.is_registry_sealed().unwrap());
}

// ---------------------------------------------------------------------
// Sub-slice C: the hard-purge saga.
// ---------------------------------------------------------------------

#[cfg(feature = "semantic-test-failpoints")]
#[derive(Debug)]
struct TestClock(AtomicI64);

#[cfg(feature = "semantic-test-failpoints")]
impl TestClock {
    fn at(value: DateTime<Utc>) -> Self {
        Self(AtomicI64::new(value.timestamp_millis()))
    }

    fn set(&self, value: DateTime<Utc>) {
        self.0.store(value.timestamp_millis(), Ordering::SeqCst);
    }
}

#[cfg(feature = "semantic-test-failpoints")]
impl SemanticClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.0.load(Ordering::SeqCst))
            .single()
            .expect("valid test timestamp")
    }
}

fn two_targets(base: &Path) -> Vec<PathBuf> {
    let target_a = base.join("target-a");
    let target_b = base.join("target-b");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();
    vec![target_a, target_b]
}

#[test]
fn happy_path_purge_preview_and_execute_reaches_completed() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);

    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();
    assert_eq!(preview.targets, vec![object_id.clone()]);

    let receipt = store
        .purge_execute(&context, "purge-op", &preview.preview_hash, &preview.nonce)
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.registry_epoch, Some(1));
    assert!(receipt.composite_checksum.is_some());
    let backup_path = PathBuf::from(receipt.new_backup_path.unwrap());
    assert!(backup_path.exists());

    assert!(store.is_denied(&object_id).unwrap());
    assert!(!store.object_exists(&object_id));
    assert!(matches!(
        store.object_json(&object_id),
        Err(SemanticError::Denied(_))
    ));
}

#[test]
fn purge_execute_same_operation_id_and_preview_hash_replays_the_stored_receipt() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();

    let first = store
        .purge_execute(&context, "purge-op", &preview.preview_hash, &preview.nonce)
        .unwrap();
    let second = store
        .purge_execute(&context, "purge-op", &preview.preview_hash, &preview.nonce)
        .unwrap();
    assert_eq!(first, second);
}

/// The explicit purge idempotency-conflict case: same `operation_id` with a
/// *different* `preview_hash`/target set must reject as `IDEMPOTENCY_CONFLICT`
/// with zero side effects, not silently start a second purge. This is not
/// inherited "for free" from `mutate_once` -- the saga is its own bespoke
/// state machine -- so it needs its own explicit check and test.
#[test]
fn purge_execute_same_operation_id_different_preview_hash_is_idempotency_conflict() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);
    let preview_one = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();

    store
        .capture(&context, capture("cap-2", b"second evidence"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-2".to_owned(),
                capture_operation_id: "cap-2".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    let confirmed_two = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-2".to_owned(),
                proposal_operation_id: "prop-2".to_owned(),
            },
        )
        .unwrap();
    let object_id_two = confirmed_two.event.payload.object_id;
    let preview_two = store
        .purge_preview(std::slice::from_ref(&object_id_two))
        .unwrap();

    store
        .purge_execute(
            &context,
            "purge-op",
            &preview_one.preview_hash,
            &preview_one.nonce,
        )
        .unwrap();
    assert!(matches!(
        store.purge_execute(
            &context,
            "purge-op",
            &preview_two.preview_hash,
            &preview_two.nonce,
        ),
        Err(SemanticError::IdempotencyConflict)
    ));
    // The conflicting call must not have denied, revoked, or deleted
    // anything for the second target.
    assert!(!store.is_denied(&object_id_two).unwrap());
    assert!(store.object_exists(&object_id_two));
}

#[test]
fn purge_execute_requires_purge_capability() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();

    let propose_only = store
        .register_client_scoped("worker", &["confirm"])
        .unwrap();
    // Sanity: this client can still confirm/reject/retract, just not purge.
    assert!(matches!(
        store.purge_execute(
            &propose_only,
            "purge-op",
            &preview.preview_hash,
            &preview.nonce,
        ),
        Err(SemanticError::CapabilityDenied(_))
    ));
    assert!(store.object_exists(&object_id));
    assert!(!store.is_denied(&object_id).unwrap());
}

#[test]
fn purge_nonce_is_single_use() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();

    store
        .purge_execute(
            &context,
            "purge-op-a",
            &preview.preview_hash,
            &preview.nonce,
        )
        .unwrap();

    // A second, distinct operation_id trying to reuse the same nonce (even
    // with the matching preview_hash) must be rejected -- the nonce was
    // already consumed by the first purge.
    assert!(matches!(
        store.purge_execute(
            &context,
            "purge-op-b",
            &preview.preview_hash,
            &preview.nonce
        ),
        Err(SemanticError::InvalidClaim(_))
    ));
}

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn purge_nonce_expires_after_sixty_seconds() {
    let clock = std::sync::Arc::new(TestClock::at("2026-07-15T12:00:00Z".parse().unwrap()));
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let config = SemanticConfig::enabled_for(parent.path())
        .with_clock(clock.clone())
        .with_purge_registry_targets(targets);
    let (store, _admin) = SemanticStore::create(&root, config).unwrap();
    let context = store.trusted_context();
    let object_id = confirm_a_claim(&store, &context);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();

    clock.set("2026-07-15T12:01:01Z".parse().unwrap());
    assert!(matches!(
        store.purge_execute(&context, "purge-op", &preview.preview_hash, &preview.nonce),
        Err(SemanticError::InvalidClaim(_))
    ));
    assert!(!store.is_denied(&object_id).unwrap());
}

#[cfg(feature = "semantic-test-failpoints")]
fn run_purge_crash_child(
    parent: &Path,
    root: &Path,
    targets: &[PathBuf],
    failpoint: &str,
    operation_id: &str,
    preview_hash: &str,
    nonce: &str,
) {
    let targets_env = targets
        .iter()
        .map(|target| target.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(";");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("purge_crash_worker")
        .arg("--nocapture")
        .env("SEMANTIC_PURGE_CRASH_WORKER", "1")
        .env("SEMANTIC_ALLOWED_PARENT", parent)
        .env("SEMANTIC_ROOT", root)
        .env("SEMANTIC_TEST_FAILPOINT", failpoint)
        .env("SEMANTIC_PURGE_REGISTRY_TARGETS", targets_env)
        .env("SEMANTIC_PURGE_OPERATION_ID", operation_id)
        .env("SEMANTIC_PURGE_PREVIEW_HASH", preview_hash)
        .env("SEMANTIC_PURGE_NONCE", nonce)
        .status()
        .expect("spawn purge crash child");
    assert!(
        !status.success(),
        "failpoint {failpoint} did not terminate abruptly"
    );
}

#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn purge_crash_worker() {
    if std::env::var_os("SEMANTIC_PURGE_CRASH_WORKER").is_none() {
        return;
    }
    let parent = PathBuf::from(std::env::var_os("SEMANTIC_ALLOWED_PARENT").unwrap());
    let root = PathBuf::from(std::env::var_os("SEMANTIC_ROOT").unwrap());
    let targets: Vec<PathBuf> = std::env::var("SEMANTIC_PURGE_REGISTRY_TARGETS")
        .unwrap()
        .split(';')
        .map(PathBuf::from)
        .collect();
    let store = SemanticStore::open(&root, enabled_with_targets(&parent, targets)).unwrap();
    let context = store.register_client("purge-worker").unwrap();
    let operation_id = std::env::var("SEMANTIC_PURGE_OPERATION_ID").unwrap();
    let preview_hash = std::env::var("SEMANTIC_PURGE_PREVIEW_HASH").unwrap();
    let nonce = std::env::var("SEMANTIC_PURGE_NONCE").unwrap();
    let _ = store.purge_execute(&context, &operation_id, &preview_hash, &nonce);
    panic!("configured failpoint did not abort the process");
}

/// Crash/retry matrix: abort the process right after each saga step
/// boundary commits, reopen, and resume by replaying the exact same
/// `purge_execute` call (idempotent resume path -- no nonce needed since
/// the saga row already exists). Proves every step is individually
/// crash-recoverable and the saga always reaches `completed`. Also proves
/// deny-first: once past `registry_denied`, the target is already
/// unreadable even though the saga has not finished.
#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn hard_purge_saga_crash_retry_matrix_completes_and_stays_fail_closed_at_every_boundary() {
    let steps = [
        "purge_after_requested",
        "purge_after_registry_denied",
        "purge_after_key_revoked",
        "purge_after_live_deleted",
        "purge_after_projections_cleaned",
        "purge_after_retention_pending",
    ];
    for failpoint in steps {
        let targets_parent = tempfile::tempdir().unwrap();
        let targets = two_targets(targets_parent.path());

        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("semantic-store");
        let (store, _admin) =
            SemanticStore::create(&root, enabled_with_targets(parent.path(), targets.clone()))
                .unwrap();
        let worker = store.register_client("purge-worker").unwrap();
        let object_id = confirm_a_claim(&store, &worker);
        let preview = store
            .purge_preview(std::slice::from_ref(&object_id))
            .unwrap();
        let operation_id = format!("purge-{failpoint}");
        drop(store);

        run_purge_crash_child(
            parent.path(),
            &root,
            &targets,
            failpoint,
            &operation_id,
            &preview.preview_hash,
            &preview.nonce,
        );

        let reopened =
            SemanticStore::open(&root, enabled_with_targets(parent.path(), targets.clone()))
                .unwrap();
        if failpoint != "purge_after_requested" {
            assert!(
                reopened.is_denied(&object_id).unwrap(),
                "failpoint {failpoint}: target must already be denied"
            );
            assert!(matches!(
                reopened.object_json(&object_id),
                Err(SemanticError::Denied(_))
            ));
        }

        let worker_again = reopened.register_client("purge-worker").unwrap();
        let receipt = reopened
            .purge_execute(
                &worker_again,
                &operation_id,
                &preview.preview_hash,
                &preview.nonce,
            )
            .unwrap();
        assert_eq!(receipt.state, "completed", "failpoint {failpoint}");
        assert!(receipt.composite_checksum.is_some());
        assert!(reopened.is_denied(&object_id).unwrap());
        assert!(!reopened.object_exists(&object_id));
        let backup_path = PathBuf::from(receipt.new_backup_path.unwrap());
        assert!(backup_path.exists());
    }
}

/// The `projections_cleaned` step performs a real absence proof, not a
/// no-op: if a stray copy of the (already key-revoked) ciphertext reappears
/// on disk between `live_deleted` and this step, the step must fail closed
/// instead of silently advancing -- and once the stray copy is removed, the
/// same saga can still complete normally.
#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn projections_cleaned_step_fails_closed_on_a_stray_leftover_object_copy() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());

    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, enabled_with_targets(parent.path(), targets.clone())).unwrap();
    let worker = store.register_client("purge-worker").unwrap();
    let object_id = confirm_a_claim(&store, &worker);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();
    let operation_id = "purge-absence-proof";

    let digest = object_id.strip_prefix("sha256:").unwrap();
    let object_file = root.join("objects").join(&digest[..2]).join(digest);
    let saved_ciphertext = std::fs::read(&object_file).unwrap();
    drop(store);

    run_purge_crash_child(
        parent.path(),
        &root,
        &targets,
        "purge_after_live_deleted",
        operation_id,
        &preview.preview_hash,
        &preview.nonce,
    );

    // Simulate a stray leftover: the object file reappears (its key was
    // already destroyed in key_revoked, so this is inert ciphertext, but
    // its mere presence must still block the absence proof).
    std::fs::create_dir_all(object_file.parent().unwrap()).unwrap();
    std::fs::write(&object_file, &saved_ciphertext).unwrap();

    let reopened =
        SemanticStore::open(&root, enabled_with_targets(parent.path(), targets.clone())).unwrap();
    let worker_again = reopened.register_client("purge-worker").unwrap();
    assert!(matches!(
        reopened.purge_execute(
            &worker_again,
            operation_id,
            &preview.preview_hash,
            &preview.nonce,
        ),
        Err(SemanticError::CorruptLedger(_))
    ));

    std::fs::remove_file(&object_file).unwrap();
    let receipt = reopened
        .purge_execute(
            &worker_again,
            operation_id,
            &preview.preview_hash,
            &preview.nonce,
        )
        .unwrap();
    assert_eq!(receipt.state, "completed");
}

/// Retract remains a fully separate, reversible mechanism: it must never
/// touch the purge registry or destroy any encryption key.
#[test]
fn retract_does_not_touch_the_purge_registry_or_destroy_keys() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);
    let object_id = confirm_a_claim(&store, &context);

    store
        .retract(
            &context,
            llm_wiki::semantic::RetractCommand {
                operation_id: "retract-op".to_owned(),
                claim_operation_id: "confirm".to_owned(),
            },
        )
        .unwrap();

    assert_eq!(store.registry_epoch().unwrap(), 0);
    assert!(!store.is_denied(&object_id).unwrap());
    assert!(store.object_exists(&object_id));
    assert!(store.object_json(&object_id).is_ok());
}

/// TOCTOU proof for the task's own named "one dominant risk": a
/// `registry_denied` commit that lands *while* a `claims_current` call is
/// mid-flight (already past its own deny-check for one row) must not tear
/// that call's view -- every row it touches must reflect one consistent
/// snapshot fixed before the denial committed, not a mix where an
/// earlier-processed row succeeds and a later one suddenly reports Denied.
/// This is the scenario the approved Task Brief committed to testing via
/// the Task 0.3 pause-hook pattern (`pause_read_after_deny_check_for_test`,
/// armed inside `decrypt_object` itself, immediately after its deny/seal
/// check and before it touches the ciphertext file).
#[cfg(feature = "semantic-test-failpoints")]
#[test]
fn claims_current_uses_one_snapshot_across_all_rows_despite_a_mid_query_denial() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (_parent, _root, store, context) = fixture_with_targets(targets);

    let mut draft_a = draft();
    draft_a.subject = "GULF".to_owned();
    draft_a.predicate = "target_price".to_owned();
    draft_a.claim_kind = "external_fact".to_owned();
    draft_a.domain = Some("stocks".to_owned());
    draft_a.value = json!(58);
    let mut draft_b = draft_a.clone();
    draft_b.value = json!(62);

    store
        .capture(&context, capture("cap-a", b"broker a says 58"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: draft_a,
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
        .capture(&context, capture("cap-b", b"broker b says 62"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft_b,
            },
        )
        .unwrap();
    let confirm_b = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-b".to_owned(),
                proposal_operation_id: "prop-b".to_owned(),
            },
        )
        .unwrap();
    let object_b = confirm_b.event.payload.object_id;

    let pause = store.pause_read_after_deny_check_for_test();
    let store_ref = &store;
    let ledger_head = store.ledger_head().unwrap();
    std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            store_ref.claims_current(ledger_head, Utc::now(), "stocks", "GULF", "target_price")
        });

        assert!(
            pause.wait_until_entered(Duration::from_secs(5)),
            "reader did not reach the pause point before the deadline"
        );
        // Deny claim B's object while the reader's transaction is paused
        // between the deny-check and the plaintext read for whichever row
        // it is currently on -- SQLite's WAL snapshot for that already-open
        // deferred transaction was fixed at its first read, before this
        // commit, so the rest of the same call must still see the
        // pre-denial state consistently.
        store_ref
            .append_registry_denial(std::slice::from_ref(&object_b))
            .unwrap();
        assert!(store_ref.is_denied(&object_b).unwrap());
        pause.release();

        let result = reader
            .join()
            .unwrap()
            .expect("in-flight claims_current must not observe a torn mid-query denial");
        let total = result.active.len() + result.past.len() + result.future.len();
        assert_eq!(
            total, 2,
            "claims_current must see one consistent pre-denial snapshot across every row"
        );
    });

    // The denial is fully durable for this store's *next* (new) call.
    assert!(matches!(
        store.claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price"
        ),
        Err(SemanticError::Denied(_))
    ));
}

/// `retention_pending` must invalidate only backups actually capable of
/// decrypting a purge target -- an earlier backup made before the target
/// object even existed carries no wrapped key for it and must survive.
#[test]
fn retention_pending_spares_a_backup_that_cannot_decrypt_the_target() {
    let targets_parent = tempfile::tempdir().unwrap();
    let targets = two_targets(targets_parent.path());
    let (parent, _root, store, context) = fixture_with_targets(targets);

    // An unrelated backup made before the purge target is even captured.
    let unrelated_backup = parent.path().join("early-backup");
    store.backup_consistent(&unrelated_backup).unwrap();
    assert!(unrelated_backup.exists());

    let object_id = confirm_a_claim(&store, &context);
    let preview = store
        .purge_preview(std::slice::from_ref(&object_id))
        .unwrap();
    let receipt = store
        .purge_execute(&context, "purge-op", &preview.preview_hash, &preview.nonce)
        .unwrap();
    assert_eq!(receipt.state, "completed");

    // The unrelated, pre-dating backup was never capable of decrypting the
    // target and must survive; the fresh post-purge backup exists too.
    assert!(
        unrelated_backup.exists(),
        "a backup with no wrapped key for the target must not be destroyed by an unrelated purge"
    );
    let fresh_backup = PathBuf::from(receipt.new_backup_path.unwrap());
    assert!(fresh_backup.exists());
}
