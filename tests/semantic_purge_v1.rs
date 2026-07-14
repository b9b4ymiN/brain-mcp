//! Task 1.3, Sub-slice B: PurgeRegistry primitive and deny-first
//! fail-closed reads. `append_registry_denial`/`sync_purge_registry` are
//! raw primitives here (capability-free, not nonce-bound) -- Sub-slice C's
//! hard-purge saga wires authorization, idempotency, and orchestration
//! around them, matching the Task 1.2b precedent for
//! `destroy_wrapped_key`/`rotate_epoch_and_rewrap`.

use std::path::{Path, PathBuf};

use chrono::Utc;
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
        domain: "projects".to_owned(),
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
