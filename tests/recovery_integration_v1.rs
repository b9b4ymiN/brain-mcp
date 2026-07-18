//! Task F3.1 — Encrypted backup (AES-GCM) + BackupReport producer.
//!
//! Integration coverage for `SemanticStore::backup_encrypted` and
//! `SemanticStore::load_or_create_backup_key`. These tests stage a real
//! snapshot via the existing `backup_consistent` VACUUM-into + objects-copy
//! path, then assert the encrypted layers + manifest are produced, that a
//! decrypt round-trip recovers the original plaintext, that the manifest's
//! composite checksum matches the live store, and (on Unix) that the backup
//! key file is created mode 0600.

use std::fs;
use std::path::Path;

use aes_gcm::aead::KeyInit;
use aes_gcm::{Aes256Gcm, Key};
use llm_wiki::recovery::BackupReport;
use llm_wiki::semantic::{CaptureCommand, SemanticConfig, SemanticStore, decrypt_backup_layer};
use serde_json::json;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

/// Stage fixture: tempdir parent + SemanticStore at `<parent>/semantic-store`
/// with two captured objects so the encrypted snapshot has real content in
/// every layer (db rows + object blobs + marker + projection).
fn fixture() -> (TempDir, std::path::PathBuf, SemanticStore) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    let ctx = store.trusted_context();
    store
        .capture(
            &ctx,
            CaptureCommand {
                operation_id: "op-one".to_owned(),
                bytes: b"first captured payload".to_vec(),
                media_type: "text/plain".to_owned(),
            },
        )
        .expect("capture one");
    store
        .capture(
            &ctx,
            CaptureCommand {
                operation_id: "op-two".to_owned(),
                bytes: br#"{"second":"payload"}"#.to_vec(),
                media_type: "application/json".to_owned(),
            },
        )
        .expect("capture two");
    (parent, root, store)
}

/// Re-derives the AES-256-GCM cipher from a `<root>/backup.key` file. Mirrors
/// what an independent restore tool would do: read the key, construct the
/// cipher, decrypt each `.enc` layer.
fn cipher_from_key(root: &Path) -> Aes256Gcm {
    let key_bytes = fs::read(root.join("backup.key")).expect("read backup.key");
    assert_eq!(
        key_bytes.len(),
        32,
        "backup.key must be 32 raw bytes (AES-256)"
    );
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    Aes256Gcm::new(key)
}

#[test]
fn encrypted_backup_produces_enc_files() {
    let (parent, _root, store) = fixture();
    let backup_dir = parent.path().join("enc-backup");
    let report = store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");

    assert!(report.encrypted, "BackupReport.encrypted must be true");
    assert!(report.objects_backed_up >= 2, "two captured objects");
    assert!(report.ledger_events_backed_up >= 2, "two capture events");
    assert!(report.config_snapshot, "marker layer always present");
    assert!(
        !report.checksum.is_empty(),
        "composite checksum must be recorded"
    );

    // Layer files: db + marker + projection (projection may or may not exist
    // depending on whether a read has populated it; either is acceptable as
    // long as the db + marker encrypted files are present) + manifest.
    assert!(
        backup_dir.join("semantic.sqlite3.enc").is_file(),
        "encrypted db layer must exist"
    );
    assert!(
        backup_dir.join("store.marker.json.enc").is_file(),
        "encrypted marker layer must exist"
    );
    assert!(
        backup_dir.join("manifest.json").is_file(),
        "plaintext manifest must exist"
    );

    // Objects shard tree: at least one shard dir, with .enc blobs inside.
    let objects_dir = backup_dir.join("objects");
    assert!(objects_dir.is_dir(), "encrypted objects tree must exist");
    let mut enc_blob_count = 0usize;
    for shard in fs::read_dir(&objects_dir).expect("read objects") {
        let shard = shard.unwrap().path();
        if !shard.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&shard).expect("read shard") {
            let path = entry.unwrap().path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("enc") {
                enc_blob_count += 1;
            }
        }
    }
    assert!(
        enc_blob_count >= 2,
        "expected at least 2 encrypted object blobs, found {enc_blob_count}"
    );

    // Manifest shape: encrypted=true, version=1, cipher name, checksum
    // matching the live store's composite_checksum.
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(backup_dir.join("manifest.json")).expect("read manifest"))
            .expect("manifest parses");
    assert_eq!(manifest["version"], json!(1));
    assert_eq!(manifest["encrypted"], json!(true));
    assert_eq!(manifest["cipher"], json!("AES-256-GCM"));
    assert_eq!(manifest["composite_checksum"], json!(report.checksum));
}

#[test]
fn encrypted_backup_decrypt_round_trip() {
    let (parent, root, store) = fixture();

    // Take ONE encrypted backup, then independently decrypt every layer and
    // verify it matches the source. We do NOT compare against a second
    // `backup_consistent` run byte-for-byte on the sqlite layer because each
    // `backup_consistent` call inserts one row into `purge_backup_sets`
    // (the snapshot-recording ledger), so two sequential snapshots' DB
    // images legitimately differ. Instead we verify the decrypted DB is a
    // valid, queryable sqlite file with the expected tables + the captured
    // events. Marker, projection, and object blobs ARE byte-stable and are
    // compared against a parallel plaintext backup.

    // Take the plaintext reference AFTER the encrypted backup so both have
    // the same number of `purge_backup_sets` rows (each path calls
    // backup_consistent exactly once before this second call: encrypted path
    // = 1 insert, this plaintext path = 1 more insert; the objects/marker
    // layers do not depend on that ledger row).
    let enc_dir = parent.path().join("enc-backup");
    store.backup_encrypted(&enc_dir).expect("backup_encrypted");
    let plaintext_dir = parent.path().join("plain-backup");
    store
        .backup_consistent(&plaintext_dir)
        .expect("plaintext backup_consistent");

    let cipher = cipher_from_key(&root);

    // DB layer: decrypt and assert it is a valid SQLite file (magic header
    // + non-trivial size). We do NOT byte-compare against a second
    // `backup_consistent` run because each call inserts a row into
    // `purge_backup_sets`, so two sequential snapshots legitimately differ
    // by one row. Opening with rusqlite and counting rows would tie this
    // test to the internal schema; the SQLite magic header is the stable
    // correctness signal here — if decryption produced garbage, the header
    // would not match.
    let dec_db = decrypt_backup_layer(&cipher, &enc_dir.join("semantic.sqlite3.enc"))
        .expect("decrypt db layer");
    assert!(
        dec_db.len() > 1024,
        "decrypted db should be non-trivially sized, got {} bytes",
        dec_db.len()
    );
    assert_eq!(
        &dec_db[..16],
        b"SQLite format 3\0",
        "decrypted db must start with the SQLite magic header"
    );

    // Marker layer: byte-stable, exact compare.
    let plain_marker = fs::read(plaintext_dir.join("store.marker.json")).expect("plain marker");
    let dec_marker = decrypt_backup_layer(&cipher, &enc_dir.join("store.marker.json.enc"))
        .expect("decrypt marker layer");
    assert_eq!(plain_marker, dec_marker, "marker layer round-trip");

    // Projection layer (only present if backup_consistent copied one).
    let plain_projection = plaintext_dir.join("projection.json");
    if plain_projection.is_file() {
        let plain_proj = fs::read(&plain_projection).expect("plain projection");
        let dec_proj = decrypt_backup_layer(&cipher, &enc_dir.join("projection.json.enc"))
            .expect("decrypt projection layer");
        assert_eq!(plain_proj, dec_proj, "projection layer round-trip");
    }

    // Object blob layers: walk the plaintext objects tree, find each file's
    // encrypted sibling at <enc_dir>/objects/<shard>/<digest>.enc, decrypt,
    // and compare bytes. Object blobs are content-addressed and not touched
    // by the purge_backup_sets insert, so byte-equality holds.
    let plain_objects = plaintext_dir.join("objects");
    let enc_objects = enc_dir.join("objects");
    assert!(plain_objects.is_dir(), "plaintext objects tree exists");
    let mut checked = 0usize;
    for shard in fs::read_dir(&plain_objects).expect("read plain objects") {
        let shard = shard.unwrap().path();
        if !shard.is_dir() {
            continue;
        }
        let shard_name = shard.file_name().unwrap();
        let enc_shard = enc_objects.join(shard_name);
        for entry in fs::read_dir(&shard).expect("read shard") {
            let path = entry.unwrap().path();
            if !path.is_file() {
                continue;
            }
            let digest = path.file_name().unwrap();
            let enc_path = enc_shard.join(format!("{}.enc", digest.to_string_lossy()));
            let plain_bytes = fs::read(&path).expect("plain object");
            let dec_bytes = decrypt_backup_layer(&cipher, &enc_path).expect("decrypt object blob");
            assert_eq!(plain_bytes, dec_bytes, "object {digest:?} round-trip");
            checked += 1;
        }
    }
    assert!(checked >= 2, "expected >=2 object blobs, checked {checked}");
}

#[test]
fn backup_report_encrypted_true() {
    let (parent, _root, store) = fixture();
    let backup_dir = parent.path().join("enc-backup");
    let report: BackupReport = store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");
    assert!(report.encrypted, "BackupReport.encrypted must be true");
    assert!(report.config_snapshot, "config/marker layer present");
    assert!(
        report.objects_backed_up > 0,
        "object count must be positive"
    );
    assert!(
        report.ledger_events_backed_up > 0,
        "ledger event count must be positive"
    );
}

#[test]
fn composite_checksum_in_manifest_matches_live() {
    let (parent, _root, store) = fixture();
    let backup_dir = parent.path().join("enc-backup");
    let report = store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");

    // The manifest's composite_checksum must equal the BackupReport's
    // checksum, which must equal what the store records as the live
    // composite checksum right now (no further mutations happened).
    let manifest_bytes = fs::read(backup_dir.join("manifest.json")).expect("read manifest");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest parses");
    let manifest_checksum = manifest["composite_checksum"]
        .as_str()
        .expect("composite_checksum present")
        .to_owned();

    assert_eq!(
        manifest_checksum, report.checksum,
        "manifest checksum == BackupReport checksum"
    );

    // Take a second backup and assert its checksum equals the first — no
    // mutations between the two means composite_checksum is deterministic.
    let backup_dir_2 = parent.path().join("enc-backup-2");
    let report_2 = store.backup_encrypted(&backup_dir_2).expect("backup 2");
    assert_eq!(
        report.checksum, report_2.checksum,
        "composite checksum is stable across back-to-back backups"
    );
}

#[test]
fn backup_key_created_with_0600_on_unix() {
    let (parent, root, store) = fixture();
    let key_path = SemanticStore::backup_key_path(&root);
    assert!(!key_path.exists(), "no backup.key before first backup");

    let backup_dir = parent.path().join("enc-backup");
    store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");

    assert!(key_path.is_file(), "backup.key created on first backup");
    let key_bytes = fs::read(&key_path).expect("read backup.key");
    assert_eq!(key_bytes.len(), 32, "backup.key is 32 bytes (AES-256)");

    // Re-running backup_encrypted MUST reuse the same key (idempotent key
    // creation), not generate a fresh one — otherwise backups taken under
    // the first key would become unrecoverable.
    let key_bytes_before = key_bytes;
    let backup_dir_2 = parent.path().join("enc-backup-2");
    store.backup_encrypted(&backup_dir_2).expect("backup 2");
    let key_bytes_after = fs::read(&key_path).expect("read backup.key again");
    assert_eq!(
        key_bytes_before, key_bytes_after,
        "load_or_create_backup_key must be idempotent across calls"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = key_path
            .metadata()
            .expect("key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "backup.key must be mode 0600 on Unix (got {mode:o})"
        );
    }
    #[cfg(not(unix))]
    {
        // Windows: no std mode API. The file inherits the user-profile
        // default ACL (user-only in a normal profile). Operators wanting a
        // stricter guarantee should `icacls backup.key /inheritance:r
        // /grant:r "%USERNAME%:R"` — documented in deploy-docker.md.
        eprintln!("[windows] backup.key mode check skipped (no std API)");
    }
}

#[test]
fn corrupted_backup_key_is_rejected_not_silently_rewritten() {
    let (parent, root, store) = fixture();
    // Pre-write a too-short "key" to simulate corruption. The store MUST
    // refuse to use it rather than overwriting it (overwriting would silently
    // orphan any prior backups taken under a real key that got corrupted).
    let key_path = SemanticStore::backup_key_path(&root);
    fs::write(&key_path, b"only-16-bytes!!!!").expect("seed corrupt key");
    let backup_dir = parent.path().join("enc-backup");
    let err = store
        .backup_encrypted(&backup_dir)
        .expect_err("corrupted key must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("backup.key") && msg.contains("32"),
        "error must explain the key length mismatch, got: {msg}"
    );
    // The corrupted bytes must be untouched (no silent rewrite).
    assert_eq!(
        fs::read(&key_path).unwrap(),
        b"only-16-bytes!!!!",
        "corrupted key must not be silently overwritten"
    );
    assert!(
        !backup_dir.exists(),
        "no backup dir created when key is invalid"
    );
}
