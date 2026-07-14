use std::fs;
use std::path::{Path, PathBuf};

use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticError, SemanticStore, TrustedContext,
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

fn only_object_file(root: &Path) -> PathBuf {
    let mut files = Vec::new();
    for shard in fs::read_dir(root.join("objects")).unwrap() {
        for entry in fs::read_dir(shard.unwrap().path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                files.push(path);
            }
        }
    }
    assert_eq!(files.len(), 1, "expected exactly one object file");
    files.into_iter().next().unwrap()
}

#[test]
fn object_bytes_on_disk_are_encrypted_not_plaintext() {
    let (_parent, root, store, context) = fixture();
    let secret = b"the quick brown fox jumps over a lazy GULF target price";
    store.capture(&context, capture("cap", secret)).unwrap();

    let on_disk = fs::read(only_object_file(&root)).unwrap();
    assert_ne!(on_disk, secret);
    assert!(
        !on_disk
            .windows(secret.len())
            .any(|window| window == secret.as_slice()),
        "plaintext must not appear anywhere in the on-disk envelope"
    );

    // But the store's own decrypt path still returns the exact plaintext.
    let outcome = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    let proposal = store
        .object_json(&outcome.event.payload.object_id)
        .unwrap();
    assert_eq!(proposal["provenance"]["byte_end"], secret.len());
}

#[test]
fn identical_content_still_dedupes_to_one_object_and_one_wrapped_key() {
    let (_parent, root, store, context) = fixture();
    store
        .capture(&context, capture("cap-one", b"identical bytes"))
        .unwrap();
    store
        .capture(&context, capture("cap-two", b"identical bytes"))
        .unwrap();

    let mut object_files = 0;
    for shard in fs::read_dir(root.join("objects")).unwrap() {
        for entry in fs::read_dir(shard.unwrap().path()).unwrap() {
            assert!(entry.unwrap().path().is_file());
            object_files += 1;
        }
    }
    assert_eq!(object_files, 1);
    assert_eq!(store.wrapped_key_count().unwrap(), 1);
}

#[test]
fn full_capture_propose_confirm_round_trips_through_decryption() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"evidence for the claim"))
        .unwrap();
    let proposed = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    let confirmed = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();
    let claim = store
        .object_json(&confirmed.event.payload.object_id)
        .unwrap();
    assert_eq!(claim["claim"]["domain"], "projects");
    let proposal = store
        .object_json(&proposed.event.payload.object_id)
        .unwrap();
    assert_eq!(proposal["provenance"]["kind"], "evidence");
}

#[test]
fn tampered_ciphertext_is_rejected_fail_closed() {
    let (_parent, root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"tamper-detection payload"))
        .unwrap();
    let path = only_object_file(&root);
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(&path, bytes).unwrap();

    let result = store.propose(
        &context,
        ProposeCommand {
            operation_id: "prop".to_owned(),
            capture_operation_id: "cap".to_owned(),
            draft: draft(),
        },
    );
    assert!(matches!(
        result,
        Err(SemanticError::ObjectUnavailable(_)) | Err(SemanticError::CorruptLedger(_))
    ));
}

#[test]
fn destroying_a_wrapped_key_makes_only_that_object_unreadable() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-a", b"first secret payload"))
        .unwrap();
    store
        .capture(&context, capture("cap-b", b"second secret payload"))
        .unwrap();
    let a = store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
    let object_a = store
        .object_json(&a.event.payload.object_id)
        .unwrap()
        .clone();
    let source_object_a = object_a["source_object_id"].as_str().unwrap().to_owned();

    store.destroy_wrapped_key(&source_object_a).unwrap();

    assert!(matches!(
        store.propose(
            &context,
            ProposeCommand {
                operation_id: "prop-a-after-destroy".to_owned(),
                capture_operation_id: "cap-a".to_owned(),
                draft: draft(),
            },
        ),
        Err(SemanticError::ObjectUnavailable(_))
    ));

    // cap-b's object is on a different wrapped key and remains fully usable.
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-b".to_owned(),
                capture_operation_id: "cap-b".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
}

#[test]
fn rotate_epoch_and_rewrap_keeps_existing_objects_readable() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap-before", b"payload before rotation"))
        .unwrap();
    let before_epoch = store.current_epoch().unwrap();

    let new_epoch = store.rotate_epoch_and_rewrap().unwrap();
    assert!(new_epoch > before_epoch);
    assert_eq!(store.current_epoch().unwrap(), new_epoch);

    // Old object, wrapped under the now-destroyed old epoch, was rewrapped
    // and remains fully readable.
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-before".to_owned(),
                capture_operation_id: "cap-before".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();

    // New writes use the new epoch.
    store
        .capture(&context, capture("cap-after", b"payload after rotation"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop-after".to_owned(),
                capture_operation_id: "cap-after".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
}

#[test]
fn backup_consistent_carries_keys_and_stays_decryptable() {
    let (parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"payload backed up encrypted"))
        .unwrap();
    let backup_root = parent.path().join("semantic-backup");
    store.backup_consistent(&backup_root).unwrap();

    let backup = SemanticStore::open(&backup_root, enabled(parent.path())).unwrap();
    let backup_context = backup.trusted_context();
    backup
        .propose(
            &backup_context,
            ProposeCommand {
                operation_id: "prop-from-backup".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: draft(),
            },
        )
        .unwrap();
}
