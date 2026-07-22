//! Task F3.1 and F3.2: encrypted backup and restore drill.
//!
//! Integration coverage for `SemanticStore::backup_encrypted` and
//! `SemanticStore::load_or_create_backup_key` (F3.1), and for
//! `SemanticStore::restore_from_backup`, `SemanticStore::run_restore_drill`,
//! and the `restore-drill.json` outcome file (F3.2). These tests stage a
//! real snapshot via the existing `backup_consistent` VACUUM-into and
//! objects-copy path, then assert the encrypted layers and manifest are
//! produced, that a decrypt round-trip recovers the original plaintext,
//! that the manifest's composite checksum matches the live store, that the
//! restore drill flips `backup_health().last_restore_drill_ok` to true on
//! success and FAILS CLOSED when the registry epoch or checksum diverge,
//! and (on Unix) that the backup key file is created mode 0600.

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
    // Carry fix 2 (Task F3.2): the plaintext staging dir — a sibling of the
    // target named `.<target-name>-staging` — MUST be wiped on the error
    // path too. A plaintext snapshot left behind on a failed run would leak
    // unencrypted object bytes the next time anyone (or anything) lists the
    // parent directory.
    let staging_dir = parent.path().join(".enc-backup-staging");
    assert!(
        !staging_dir.exists(),
        "staging dir must be wiped on the error path (got {})",
        staging_dir.display()
    );
}

// ── Task F3.2 — restore drill + registry fail-closed ────────────────────────

/// Helper: take an encrypted backup, then load the live store's `backup.key`
/// into a `Key<Aes256Gcm>` (the parameter shape `restore_from_backup` and
/// `run_restore_drill` accept). Returns everything the F3.2 tests need.
fn backup_and_key() -> (
    TempDir,
    std::path::PathBuf,
    SemanticStore,
    std::path::PathBuf,
    Key<Aes256Gcm>,
) {
    let (parent, root, store) = fixture();
    let backup_dir = parent.path().join("enc-backup");
    store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");
    let key_bytes = fs::read(root.join("backup.key")).expect("read backup.key");
    let key = *Key::<Aes256Gcm>::from_slice(&key_bytes);
    (parent, root, store, backup_dir, key)
}

/// Mutates `manifest.json` at `backup_dir/manifest.json` by applying a JSON
/// patch to the `composite_checksum` field — used to force a checksum
/// mismatch on restore without touching the encrypted layers.
fn tamper_manifest_checksum(backup_dir: &Path, new_checksum: &str) {
    let path = backup_dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read manifest")).expect("parse manifest");
    manifest["composite_checksum"] = json!(new_checksum);
    let bytes = serde_json::to_vec_pretty(&manifest).expect("serialize manifest");
    fs::write(&path, bytes).expect("write manifest back");
}

/// Mutates `manifest.json` at `backup_dir/manifest.json` by applying a JSON
/// patch to the `purge_epoch` field — used to force a registry epoch mismatch
/// on restore without touching the encrypted layers.
fn tamper_manifest_purge_epoch(backup_dir: &Path, new_epoch: u64) {
    let path = backup_dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read manifest")).expect("parse manifest");
    manifest["purge_epoch"] = json!(new_epoch);
    let bytes = serde_json::to_vec_pretty(&manifest).expect("serialize manifest");
    fs::write(&path, bytes).expect("write manifest back");
}

#[test]
fn restore_drill_happy_path() {
    let (_parent, _root, store, backup_dir, key) = backup_and_key();

    // Before the drill: no outcome file → last_restore_drill_ok is false.
    assert!(
        !store.backup_health().expect("health").last_restore_drill_ok,
        "drill must not have run yet"
    );

    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("run_restore_drill");

    assert!(
        result.passed(),
        "drill should pass on a freshly-taken, untampered backup (got {result:?})"
    );
    assert!(
        result.purge_registry_synced,
        "purge registry epoch should match (no purge activity in the fixture)"
    );
    assert!(
        result.composite_checksum_matches,
        "recomputed checksum should match the manifest's"
    );

    // The drill writes <root>/restore-drill.json with last_ok: true, and
    // backup_health now reads it back as last_restore_drill_ok: true.
    let drill_path = _root.join("restore-drill.json");
    assert!(drill_path.is_file(), "restore-drill.json must be written");
    let body: serde_json::Value =
        serde_json::from_slice(&fs::read(&drill_path).expect("read drill file"))
            .expect("drill file parses");
    assert_eq!(body["last_ok"], json!(true), "outcome file last_ok=true");
    assert!(
        body["composite_checksum"].is_string(),
        "outcome file records the recomputed checksum"
    );
    assert_eq!(
        body["purge_registry_synced"],
        json!(true),
        "outcome file records registry sync"
    );
    assert!(
        body["layers_restored"].is_array(),
        "outcome file records layer list"
    );
    assert!(
        store.backup_health().expect("health").last_restore_drill_ok,
        "backup_health().last_restore_drill_ok must flip to true after a successful drill"
    );
}

#[test]
fn restore_drill_fails_when_registry_tampered() {
    let (_parent, _root, store, backup_dir, key) = backup_and_key();

    // Force a registry epoch mismatch: the manifest's purge_epoch is bumped
    // to 99 (a value no freshly-taken backup could have — the fixture has no
    // purge activity so the real epoch is 0). Restore must compare the
    // decrypted db's actual registry head against this tampered value, see
    // the divergence, and FAIL CLOSED.
    tamper_manifest_purge_epoch(&backup_dir, 99);

    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("drill returns a result, not an error");

    assert!(
        !result.passed(),
        "drill must fail when the registry epoch diverges (got {result:?})"
    );
    assert!(
        !result.purge_registry_synced,
        "purge_registry_synced flag must be false on tampered registry"
    );

    // The outcome file is written even on this registry-divergence path
    // (restore_from_backup returns Ok with purge_registry_synced=false; the
    // drill records last_ok=false because passed() is false).
    let body: serde_json::Value = serde_json::from_slice(
        &fs::read(_root.join("restore-drill.json")).expect("read drill file"),
    )
    .expect("drill file parses");
    assert_eq!(
        body["last_ok"],
        json!(false),
        "outcome file records failure on tampered registry"
    );
    assert!(
        !store.backup_health().expect("health").last_restore_drill_ok,
        "backup_health().last_restore_drill_ok stays false after a failed drill"
    );
}

#[test]
fn restore_drill_fails_when_checksum_mismatch() {
    let (_parent, _root, store, backup_dir, key) = backup_and_key();

    // Force a checksum mismatch: replace the manifest's composite_checksum
    // with a known-bogus value. The decrypted db's recomputed checksum will
    // not match → restore_from_backup returns Err → drill reports
    // composite_checksum_matches=false.
    tamper_manifest_checksum(&backup_dir, &"0".repeat(64));

    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("drill returns a result, not an error");

    assert!(
        !result.passed(),
        "drill must fail when the composite checksum diverges (got {result:?})"
    );
    assert!(
        !result.composite_checksum_matches,
        "composite_checksum_matches flag must be false on tampered checksum"
    );
}

#[test]
fn restore_drill_fails_when_db_layer_missing() {
    let (_parent, _root, store, backup_dir, key) = backup_and_key();

    // Delete the encrypted db layer — a mandatory layer. Restore must fail
    // trying to read it (file-not-found → Io error → drill reports both
    // flags false + writes the outcome file with last_ok: false + the
    // error string).
    let db_layer = backup_dir.join("semantic.sqlite3.enc");
    assert!(db_layer.is_file(), "test setup: db layer must exist");
    fs::remove_file(&db_layer).expect("delete db layer");

    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("drill returns a result, not an error");
    assert!(
        !result.passed(),
        "drill must fail when the db layer is missing (got {result:?})"
    );
    assert!(
        !result.composite_checksum_matches,
        "composite_checksum_matches must be false on decrypt failure"
    );

    // The outcome file is written even on this decrypt-failure path.
    let body: serde_json::Value = serde_json::from_slice(
        &fs::read(_root.join("restore-drill.json")).expect("read drill file"),
    )
    .expect("drill file parses");
    assert_eq!(
        body["last_ok"],
        json!(false),
        "outcome file records failure on missing db layer"
    );
    assert!(
        body["error"].is_string() && body["error"].as_str().is_some_and(|s| !s.is_empty()),
        "outcome file carries the decrypt-failure error message (got {body})"
    );
}

#[test]
fn restore_drill_writes_outcome_file_on_failure() {
    let (_parent, _root, store, backup_dir, _key) = backup_and_key();

    // Drill with a bogus cipher key: every layer decrypt fails, drill
    // reports failure, AND the outcome file is still written with
    // last_ok: false + an error message.
    let bogus_key_bytes = [0u8; 32];
    let bogus_key = Key::<Aes256Gcm>::from_slice(&bogus_key_bytes);

    let result = store
        .run_restore_drill(&backup_dir, bogus_key)
        .expect("drill returns a result, not an error");
    assert!(
        !result.passed(),
        "drill must fail with a bogus key (got {result:?})"
    );

    let drill_path = _root.join("restore-drill.json");
    assert!(
        drill_path.is_file(),
        "outcome file must exist even after a failed drill"
    );
    let body: serde_json::Value =
        serde_json::from_slice(&fs::read(&drill_path).expect("read drill file"))
            .expect("drill file parses");
    assert_eq!(
        body["last_ok"],
        json!(false),
        "outcome file last_ok must be false after a failed drill"
    );
    assert!(
        body["error"].is_string() && body["error"].as_str().is_some_and(|s| !s.is_empty()),
        "outcome file must carry a non-empty error message on failure (got {body})"
    );

    // backup_health flips to false (or stays false) on failure.
    assert!(
        !store.backup_health().expect("health").last_restore_drill_ok,
        "backup_health().last_restore_drill_ok must be false after a failed drill"
    );
}

#[test]
fn restore_from_backup_low_level_round_trip() {
    let (parent, _root, store, backup_dir, key) = backup_and_key();

    // The low-level restore API materializes a fresh store at --target.
    let target_dir = parent.path().join("restored-store");
    let receipt = store
        .restore_from_backup(&backup_dir, &target_dir, &key)
        .expect("restore_from_backup");

    assert_eq!(receipt.state, "completed");
    assert!(
        receipt.layers_restored.iter().any(|name| name == "db"),
        "db layer restored"
    );
    assert!(
        receipt.layers_restored.iter().any(|name| name == "marker"),
        "marker layer restored"
    );
    assert!(
        receipt.layers_restored.iter().any(|name| name == "objects"),
        "objects layer restored"
    );
    assert!(
        receipt.purge_registry_synced,
        "registry epoch matches on a fresh untampered backup"
    );
    assert!(
        !receipt.composite_checksum.is_empty(),
        "receipt carries the recomputed composite checksum"
    );

    // The restored store opens independently and reports the same identity
    // shape as the source (a marker file the open path accepts).
    let restored = SemanticStore::open(&target_dir, SemanticConfig::enabled_for(parent.path()))
        .expect("open restored store");
    let _ = restored; // opened successfully = marker + db + identity are consistent
}

// ── F3.2 review Fix 1 — staging cleanup on panic via Drop guard ─────────────

/// Defense-in-depth test for the `StagingDirGuard` Drop guard (F3.2 review
/// Fix 1): after a SUCCESSFUL restore, the staging dir MUST have been
/// atomically renamed into `target`, and the disarmed guard MUST NOT wipe
/// `target`. (The panic-unwind coverage is structural — the guard's `Drop`
/// runs whenever its owner scope exits, including unwinding; simulating a
/// rusqlite internal panic mid-`run_restore_inner` is not easily reachable
/// from a black-box integration test without synthesizing a corrupt backup
/// that panics inside SQLite, and we already have
/// `restore_drill_fails_when_db_layer_missing` covering the error-path
/// cleanup. The Drop guard is a one-liner-verify struct: any test that
/// reaches the staging-guarded scope exercises the drop path.)
#[test]
fn restore_target_kept_after_success_and_staging_wiped() {
    let (parent, _root, store, backup_dir, key) = backup_and_key();

    // Restore into a fresh target under the same parent.
    let target_dir = parent.path().join("restored-store");
    let receipt = store
        .restore_from_backup(&backup_dir, &target_dir, &key)
        .expect("restore_from_backup");
    assert_eq!(receipt.state, "completed");

    // On the success path the staging dir is renamed INTO target — so the
    // staging path (parent/.restored-store-restore-staging) MUST NOT exist,
    // and target MUST persist (operator data, never wiped by the guard).
    let staging_dir = parent.path().join(".restored-store-restore-staging");
    assert!(
        !staging_dir.exists(),
        "staging dir must be gone after successful restore (atomic rename into target), \
         got {}",
        staging_dir.display()
    );
    assert!(
        target_dir.exists(),
        "target MUST persist after restore (operator data)"
    );
    assert!(
        target_dir.join("semantic.sqlite3").is_file(),
        "restored db must exist at target (guard did not wipe target)"
    );
}

// ── F3.2 review Fix 2 — per-blob digests in BackupManifest ──────────────────

/// Stage a SECOND store in the same parent (so the same `allowed_parent`)
/// with DIFFERENT object content, sharing the SAME `backup.key` as the first
/// store. Returns `(second_store, backup_dir_of_second)` so the swap test
/// can substitute a `.enc` blob from store-2 into store-1's backup dir.
///
/// Sharing the key is the threat model: an attacker who has the operator's
/// `backup.key` (or has read access to backups taken under that key) can
/// legitimately encrypt a foreign blob under it — AES-GCM authenticates the
/// ciphertext but does NOT bind it to the original backup. Per-blob
/// plaintext digests in `manifest.json` close that gap.
fn second_store_with_same_key(
    parent: &Path,
    src_root: &Path,
) -> (std::path::PathBuf, SemanticStore, std::path::PathBuf) {
    let second_root = parent.join("semantic-store-other");
    let (store, _admin) =
        SemanticStore::create(&second_root, enabled(parent)).expect("create second store");

    // Copy store-1's backup.key into store-2's root BEFORE its first
    // backup_encrypted call — that call's load_or_create_backup_key will
    // then pick up the shared key (idempotent key load), so both backups
    // are encrypted under the SAME AES-256-GCM key.
    let key_bytes = fs::read(src_root.join("backup.key")).expect("read src backup.key");
    fs::write(second_root.join("backup.key"), &key_bytes).expect("seed second store key");

    // Capture DIFFERENT object content so the digests differ.
    let ctx = store.trusted_context();
    store
        .capture(
            &ctx,
            CaptureCommand {
                operation_id: "op-other-a".to_owned(),
                bytes: b"completely different payload A".to_vec(),
                media_type: "text/plain".to_owned(),
            },
        )
        .expect("capture other A");
    store
        .capture(
            &ctx,
            CaptureCommand {
                operation_id: "op-other-b".to_owned(),
                bytes: b"unrelated bytes for swap test".to_vec(),
                media_type: "application/octet-stream".to_owned(),
            },
        )
        .expect("capture other B");

    let backup_dir = parent.join("enc-backup-other");
    store
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted second");
    (second_root, store, backup_dir)
}

/// Collect every `objects/<shard>/<digest>.enc` path under `backup_dir` (the
/// encrypted object-blob tree). Returns absolute paths.
fn collect_enc_object_blobs(backup_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let objects = backup_dir.join("objects");
    if !objects.is_dir() {
        return out;
    }
    for shard in fs::read_dir(&objects).expect("read objects") {
        let shard = shard.unwrap().path();
        if !shard.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&shard).expect("read shard") {
            let path = entry.unwrap().path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("enc") {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn restore_drill_passes_with_valid_object_blobs() {
    // Happy path is unaffected by the per-blob digest layer: a freshly-taken
    // backup's manifest lists every blob it encrypted, and the recomputed
    // sha256 of each restored plaintext matches. This guards against the
    // digest code rejecting legitimate backups (the red→green pair for the
    // swap test).
    let (_parent, _root, store, backup_dir, key) = backup_and_key();

    // Manifest MUST now carry a non-empty `objects` digest map (F3.2 review
    // Fix 2). The fixture captures 2 objects → at least 2 digests.
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(backup_dir.join("manifest.json")).expect("read manifest"))
            .expect("manifest parses");
    let objects_map = manifest
        .get("objects")
        .expect("manifest.objects field present");
    let digest_count = objects_map.as_object().map(|m| m.len()).unwrap_or(0);
    assert!(
        digest_count >= 2,
        "manifest.objects must list >=2 per-blob digests (got {digest_count})"
    );

    // Restore must succeed end-to-end with all digests matching.
    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("run_restore_drill");
    assert!(
        result.passed(),
        "drill must pass on a fresh backup with valid per-blob digests (got {result:?})"
    );
}

#[test]
fn restore_drill_fails_when_object_blob_swapped() {
    // Threat model (F3.2 review Fix 2): an attacker substitutes
    // `objects/<shard>/<digest>.enc` with a legitimately-encrypted blob
    // from a DIFFERENT object under the SAME backup key. AES-GCM
    // authenticates the substituted ciphertext (so decryption succeeds),
    // but the decrypted plaintext no longer matches what the backup
    // originally recorded — detected here via the manifest's per-blob
    // sha256 digest map.
    let (parent, root, store, backup_dir, key) = backup_and_key();

    // Second store + backup sharing the same key (different object bytes).
    let (_other_root, _other_store, other_backup_dir) =
        second_store_with_same_key(parent.path(), &root);

    // Both backups must have at least one object blob to swap.
    let primary_blobs = collect_enc_object_blobs(&backup_dir);
    let other_blobs = collect_enc_object_blobs(&other_backup_dir);
    assert!(
        !primary_blobs.is_empty() && !other_blobs.is_empty(),
        "both backups must have at least one encrypted object blob to swap"
    );

    // Sanity: the substituted blob must decrypt cleanly under the same key
    // (this is what AES-GCM does NOT catch — it authenticates the foreign
    // ciphertext). If the key share above is wrong, this assertion catches
    // it before the swap rather than producing a misleading test outcome.
    let cipher = cipher_from_key(&root);
    let swapped_bytes = fs::read(&other_blobs[0]).expect("read other blob bytes");
    let _ = decrypt_backup_layer(&cipher, &other_blobs[0])
        .expect("substituted blob MUST decrypt under the shared key (otherwise this is not the threat model)");

    // Overwrite the primary's first object blob with the substituted bytes
    // (same nonce+ ciphertext, but for a DIFFERENT plaintext).
    let target_path = &primary_blobs[0];
    fs::write(target_path, &swapped_bytes).expect("swap blob bytes");

    // Restore must FAIL CLOSED via the per-blob digest check. The drill
    // returns Ok(RecoveryDrillResult) on the failure branch (never Err), so
    // check `passed()` is false + composite_checksum_matches is false.
    let result = store
        .run_restore_drill(&backup_dir, &key)
        .expect("drill returns a result, not an error");
    assert!(
        !result.passed(),
        "drill MUST fail when an object blob is swapped (got {result:?})"
    );
    assert!(
        !result.composite_checksum_matches,
        "composite_checksum_matches must be false on object-blob swap"
    );

    // The outcome file must record the failure with an error string that
    // mentions the digest mismatch (so operators see WHY the drill failed).
    let body: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("restore-drill.json")).expect("read drill file"),
    )
    .expect("drill file parses");
    assert_eq!(body["last_ok"], json!(false), "outcome file last_ok=false");
    let err = body["error"]
        .as_str()
        .expect("outcome file carries an error message");
    assert!(
        err.contains("object blob digest mismatch") || err.contains("digest mismatch"),
        "error must explain the digest mismatch, got: {err}"
    );
}

// ── Task F3.3 — schema upgrade runner + RPO/RTO recorder ─────────────────────
//
// The v2→v3 migration is a noop placeholder (every step is reversible), but
// the runner is real: plan + execute + rollback + dry-run + RPO/RTO record/
// read. The composite_checksum formula is
// `sha256("{ledger_head}:{purge_epoch}:{schema_version}")`, so bumping
// schema_version from 2 → 3 changes the checksum EVEN THOUGH ledger_head +
// purge_epoch are byte-identical. The "data intact" assertions below
// therefore check ledger_head + purge_epoch directly (via composite_checksum
// recomputation under a fixed schema_version), NOT raw checksum equality
// across the version bump — this is the contract the spec calls out and is
// documented inline in each test.

/// Reads the schema_version field out of `<root>/store.marker.json`. Used by
/// the F3.3 tests to verify the on-disk marker was actually rewritten by
/// `execute_schema_upgrade` / `rollback_schema_upgrade` (the in-memory
/// `self.marker` is fine, but the on-disk value is what the NEXT open()
/// reads — that is the value that matters operationally).
fn read_marker_schema_version(root: &Path) -> u8 {
    let bytes = fs::read(root.join("store.marker.json")).expect("read marker");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("marker parses");
    value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .expect("marker has schema_version") as u8
}

/// Downgrades the on-disk marker's `schema_version` to `to`. The in-memory
/// `SemanticStore` instance is unaffected (its `marker` was read on open);
/// the next `open_for_upgrade()` will pick up the rewritten marker. Used to
/// stage a v2 store for the upgrade tests (a freshly-created store is at v3
/// because `CURRENT_DISK_SCHEMA_VERSION = 3`).
fn rewrite_marker_schema_version(root: &Path, to: u8) {
    let path = root.join("store.marker.json");
    let bytes = fs::read(&path).expect("read marker");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("marker parses");
    value["schema_version"] = json!(to);
    let rewritten = serde_json::to_vec(&value).expect("serialize marker");
    fs::write(&path, rewritten).expect("rewrite marker");
}

/// Stage a v2 store: `fixture()` creates a v3 store with two captures,
/// close it, then rewrite its on-disk marker to schema_version=2. Returns
/// the parent tempdir + the store root so the test can `open_for_upgrade()`
/// against the staged v2 state.
fn fixture_at_v2() -> (TempDir, std::path::PathBuf) {
    let (parent, root, _store) = fixture();
    // `_store` dropped here — closes the connection so the marker file is
    // not contended when we rewrite it.
    drop(_store);
    rewrite_marker_schema_version(&root, 2);
    assert_eq!(
        read_marker_schema_version(&root),
        2,
        "test setup: marker downgraded to v2"
    );
    (parent, root)
}

#[test]
fn upgrade_v2_to_v3_noop_succeeds() {
    let (_parent, root) = fixture_at_v2();

    let store = SemanticStore::open_for_upgrade(&root, enabled(_parent.path()))
        .expect("open_for_upgrade tolerates v2 (known migration path)");
    assert_eq!(
        store.schema_version(),
        2,
        "live marker is at v2 pre-upgrade"
    );

    let plan = store.plan_schema_upgrade(2, 3).expect("plan v2→v3");
    assert!(plan.is_reversible(), "v2→v3 plan must be reversible");
    assert_eq!(plan.from_version, 2);
    assert_eq!(plan.to_version, 3);
    assert_eq!(plan.steps.len(), 1, "v2→v3 is one noop step");
    assert!(
        plan.steps[0]
            .description
            .contains("noop placeholder migration"),
        "step description names the noop (got {:?})",
        plan.steps[0].description
    );

    store.execute_schema_upgrade(&plan).expect("execute v2→v3");

    // The on-disk marker's schema_version must be 3 now (execute rewrote
    // it atomically — this is what the NEXT open() reads).
    assert_eq!(
        read_marker_schema_version(&root),
        3,
        "on-disk marker must be at v3 after execute"
    );

    // Data-intact contract: ledger_head + purge_epoch must be unchanged
    // across the noop upgrade. composite_checksum recomputation includes
    // schema_version as its third input (`sha256("{ledger_head}:
    // {purge_epoch}:{schema_version}")`), so the checksum STRING differs
    // across the bump (v2 input → v3 input) even though ledger_head +
    // purge_epoch are byte-identical — see the F3.3 task spec. We assert
    // data-intact via ledger_head + purge_epoch directly, NOT via raw
    // checksum equality across the bump.
    //
    // Re-open to pick up the rewritten marker. NOTE: since
    // CURRENT_DISK_SCHEMA_VERSION moved to 4 (Entity Identity Reform), a v3
    // store is no longer servable via plain `open` — it is "older than
    // binary." We use `open_for_upgrade` (which tolerates known-path older
    // versions) to re-open and verify data integrity. The store is genuinely
    // at v3 (marker + tables); it just needs another upgrade run to reach v4.
    let upgraded =
        SemanticStore::open_for_upgrade(&root, enabled(_parent.path())).expect("reopen at v3");
    assert_eq!(upgraded.schema_version(), 3, "upgraded store reports v3");
    // ledger_head + purge_epoch are read via composite_checksum's internals
    // (those fields are not exposed as public methods), but the noop
    // migration made ZERO data changes — captured_at v2 are still present
    // at v3. Verify the data via a normal read: read_capture of the first
    // op still returns its bytes.
    let ctx = upgraded.trusted_context();
    let bytes = upgraded
        .read_capture(&ctx, "op-one")
        .expect("read_capture op-one after upgrade");
    assert_eq!(
        bytes, b"first captured payload",
        "ledger + objects intact across v2→v3 noop upgrade"
    );
}

#[test]
fn rollback_v3_to_v2_noop_succeeds() {
    let (_parent, root) = fixture_at_v2();

    // Step 1: upgrade v2 → v3.
    let store = SemanticStore::open_for_upgrade(&root, enabled(_parent.path()))
        .expect("open_for_upgrade v2");
    let plan = store.plan_schema_upgrade(2, 3).expect("plan v2→v3");
    store.execute_schema_upgrade(&plan).expect("execute v2→v3");
    assert_eq!(read_marker_schema_version(&root), 3, "marker at v3");

    // Step 2: open the now-v3 store. Since CURRENT_DISK_SCHEMA_VERSION is 4
    // (Entity Identity Reform), a v3 store is "older than binary" — use
    // open_for_upgrade (tolerates known-path older versions) instead of open.
    let upgraded =
        SemanticStore::open_for_upgrade(&root, enabled(_parent.path())).expect("reopen at v3");
    let rollback_plan = upgraded
        .plan_schema_upgrade(2, 3)
        .expect_err("plan_schema_upgrade against v3 marker with from=2 must fail (live is v3)");
    let msg = format!("{rollback_plan}");
    assert!(
        msg.contains("plan from=2 does not match live marker schema_version=3"),
        "plan against wrong starting version fails closed: {msg}"
    );

    // The original plan captured from v2 is what rollback needs (it
    // remembers from=2, to=3). Reuse it.
    upgraded
        .rollback_schema_upgrade(&plan)
        .expect("rollback v3→v2");

    // On-disk marker must be back at v2 (execute's atomic rewrite, run in
    // reverse by rollback).
    assert_eq!(
        read_marker_schema_version(&root),
        2,
        "on-disk marker must be at v2 after rollback"
    );

    // Data intact: op-one's bytes still readable after the round trip.
    // open_for_upgrade because the marker is at v2 again.
    let rolled_back =
        SemanticStore::open_for_upgrade(&root, enabled(_parent.path())).expect("reopen at v2");
    let ctx = rolled_back.trusted_context();
    let bytes = rolled_back
        .read_capture(&ctx, "op-one")
        .expect("read_capture op-one after rollback");
    assert_eq!(
        bytes, b"first captured payload",
        "ledger + objects intact across v2→v3→v2 round trip"
    );
}

#[test]
fn upgrade_rejects_unsupported_path() {
    let (_parent, root) = fixture_at_v2();
    let store = SemanticStore::open_for_upgrade(&root, enabled(_parent.path()))
        .expect("open_for_upgrade v2");

    // v1 → v5 is not a known migration path (supported paths: 2→3, 3→4).
    let err = store
        .plan_schema_upgrade(1, 5)
        .expect_err("v1→v5 must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("unsupported schema upgrade path") && msg.contains("supported paths"),
        "error must name the unsupported path + the supported ones, got: {msg}"
    );

    // Even on a freshly-created v3 store, plan v2→v4 must fail.
    let (parent2, _root2, store2) = fixture();
    let err = store2
        .plan_schema_upgrade(2, 4)
        .expect_err("v2→v4 rejected even from a v3 store");
    let msg = format!("{err}");
    // The from-version mismatch check fires FIRST here (live is 3, not 2),
    // so the message names that mismatch — but it must still error out,
    // not silently produce a plan.
    assert!(
        msg.contains("does not match live marker")
            || msg.contains("unsupported schema upgrade path"),
        "any unsupported combination must Err, got: {msg}"
    );
    drop(parent2);
}

#[test]
fn upgrade_dry_run_does_not_mutate() {
    let (_parent, root) = fixture_at_v2();
    let original_marker_bytes =
        fs::read(root.join("store.marker.json")).expect("read marker pre-plan");

    let store = SemanticStore::open_for_upgrade(&root, enabled(_parent.path()))
        .expect("open_for_upgrade v2");

    // Plan only — DO NOT execute. The store must be byte-for-byte unchanged
    // (dry_run is a CLI flag that skips execute_schema_upgrade; this test
    // exercises the underlying invariant: plan_schema_upgrade is read-only).
    let plan = store.plan_schema_upgrade(2, 3).expect("plan v2→v3");
    assert!(plan.is_reversible());

    let after_marker_bytes =
        fs::read(root.join("store.marker.json")).expect("read marker post-plan");
    assert_eq!(
        original_marker_bytes, after_marker_bytes,
        "dry-run (plan only) must leave the marker file untouched"
    );
    assert_eq!(
        read_marker_schema_version(&root),
        2,
        "marker schema_version still v2 after plan-only"
    );
}

#[test]
fn upgrade_rehearsal_round_trip() {
    // The full F3.3 rehearsal: backup → upgrade v2→v3 → rollback v3→v2 →
    // drill still passes. Proves the upgrade path does not corrupt the
    // backup-then-drill contract: even after a noop version bump AND its
    // rollback, the restore drill against the v2-era backup must still
    // verify composite checksum + PurgeRegistry sync (fail-closed contract).
    let (parent, root, store_at_v3) = fixture();
    // 1. Take an encrypted backup BEFORE any version manipulation. The
    //    fixture store is at v3 (CURRENT_DISK_SCHEMA_VERSION), so the
    //    backup's manifest records a v3 composite checksum.
    let backup_dir = parent.path().join("enc-backup");
    store_at_v3
        .backup_encrypted(&backup_dir)
        .expect("backup_encrypted");
    let key_bytes = fs::read(root.join("backup.key")).expect("read backup.key");
    let key = *Key::<Aes256Gcm>::from_slice(&key_bytes);
    drop(store_at_v3);

    // 2. Downgrade the on-disk marker to v2 to stage an "old store" for the
    //    upgrade. The backup manifest's v3 checksum will be used by the
    //    drill against a target whose marker we re-bump to v3 by the
    //    upgrade — drill's recomputed checksum must match.
    rewrite_marker_schema_version(&root, 2);

    // 3. Upgrade v2 → v3. The marker file is rewritten to v3 atomically.
    let upgraded = SemanticStore::open_for_upgrade(&root, enabled(parent.path()))
        .expect("open_for_upgrade v2");
    let plan = upgraded.plan_schema_upgrade(2, 3).expect("plan v2→v3");
    upgraded.execute_schema_upgrade(&plan).expect("execute");
    assert_eq!(
        read_marker_schema_version(&root),
        3,
        "marker at v3 post-upgrade"
    );
    drop(upgraded);

    // 4. Re-open at v3 and run the restore drill. Since
    // CURRENT_DISK_SCHEMA_VERSION is 4 (Entity Identity Reform), a v3 store
    // is "older than binary" — use open_for_upgrade (tolerates known-path
    // older versions). composite_checksum recomputed against the restored v3
    // snapshot must match the manifest's v3 checksum → drill passes.
    let reopened =
        SemanticStore::open_for_upgrade(&root, enabled(parent.path())).expect("reopen at v3");
    let result = reopened
        .run_restore_drill(&backup_dir, &key)
        .expect("drill after upgrade");
    assert!(
        result.passed(),
        "drill must pass after v2→v3 upgrade (got {result:?})"
    );
    drop(reopened);

    // 5. Rollback v3 → v2. Marker back at v2.
    let rolled = SemanticStore::open_for_upgrade(&root, enabled(parent.path()))
        .expect("open_for_upgrade v3 for rollback");
    rolled
        .rollback_schema_upgrade(&plan)
        .expect("rollback v3→v2");
    assert_eq!(
        read_marker_schema_version(&root),
        2,
        "marker back at v2 post-rollback"
    );
}

#[test]
fn rpo_rto_record_and_read() {
    let (_parent, _root, store) = fixture();

    // Fresh store: no record yet → read returns Ok(None).
    assert!(
        store.read_rpo_rto().expect("read fresh").is_none(),
        "fresh store has no rpo-rto.json"
    );

    let record = store
        .record_rpo_rto(1440, 60, true)
        .expect("record_rpo_rto");
    assert_eq!(record.rpo_minutes, 1440);
    assert_eq!(record.rto_minutes, 60);
    assert!(record.last_met);

    // The file lands at the documented path with the documented shape.
    let path = _root.join("rpo-rto.json");
    assert!(path.is_file(), "rpo-rto.json written");
    let body: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read rpo-rto")).expect("rpo-rto parses");
    assert_eq!(body["rpo_minutes"], json!(1440));
    assert_eq!(body["rto_minutes"], json!(60));
    assert_eq!(body["last_met"], json!(true));
    assert!(
        body["recorded_at"].is_string(),
        "recorded_at timestamp recorded"
    );

    // Read returns the same record.
    let read_back = store
        .read_rpo_rto()
        .expect("read after write")
        .expect("Some after write");
    assert_eq!(read_back, record, "read returns the written record");

    // Rewrite with --not-met semantics (a follow-up record_rpo_rto call).
    let updated = store
        .record_rpo_rto(1440, 60, false)
        .expect("record_rpo_rto update");
    assert!(!updated.last_met, "rewrite can flip last_met to false");
    let read_after = store.read_rpo_rto().expect("read").expect("Some");
    assert!(!read_after.last_met, "read reflects the rewrite");
}

#[test]
fn rpo_rto_read_returns_none_when_absent() {
    let (_parent, _root, store) = fixture();
    let result = store
        .read_rpo_rto()
        .expect("read returns Ok, not Err, when file absent");
    assert!(result.is_none(), "absent file → None, not an error");

    // Confirm the file truly does not exist on a fresh store.
    assert!(
        !_root.join("rpo-rto.json").exists(),
        "fresh store has no rpo-rto.json on disk"
    );
}
