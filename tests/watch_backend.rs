//! Unit tests for `watch::fs_magic_kind` + `watch::resolve_backend` —
//! the statfs-detection logic for the watcher bind-mount fix (2026-07-25).

use llm_wiki::config::{WatchBackendConfig, WatchConfig};
use llm_wiki::watch::{Backend, FilesystemKind, fs_magic_kind, resolve_backend};

#[test]
fn fs_magic_v9fs_is_broken() {
    // V9FS_MAGIC = 0x01021997 — WSL2 9P interop, the Docker Desktop Windows
    // bind mount. Verified against include/uapi/linux/magic.h in the kernel.
    assert_eq!(fs_magic_kind(0x01021997), FilesystemKind::Broken);
}

#[test]
fn fs_magic_nfs_is_broken() {
    // NFS_SUPER_MAGIC = 0x6969.
    assert_eq!(fs_magic_kind(0x6969), FilesystemKind::Broken);
}

#[test]
fn fs_magic_cifs_is_broken() {
    // CIFS_MAGIC_NUMBER = 0xff534d42.
    assert_eq!(fs_magic_kind(0xff534d42), FilesystemKind::Broken);
}

#[test]
fn fs_magic_fuse_is_broken() {
    // FUSE_SUPER_MAGIC = 0x65735546.
    assert_eq!(fs_magic_kind(0x65735546), FilesystemKind::Broken);
}

#[test]
fn fs_magic_ext4_is_native() {
    // ext4 SUPER_MAGIC = 0xef53.
    assert_eq!(fs_magic_kind(0xef53), FilesystemKind::Native);
}

#[test]
fn fs_magic_overlay_is_native() {
    // OVERLAYFS_SUPER_MAGIC = 0x794c7630 — Docker's default storage driver;
    // inotify on the merged view works (the host-side bind-mount case is
    // V9FS/FUSE, already classified as Broken).
    assert_eq!(fs_magic_kind(0x794c7630), FilesystemKind::Native);
}

#[test]
fn fs_magic_unknown_is_native() {
    // Unknown magic numbers fail-open to Native (do not block startup).
    assert_eq!(fs_magic_kind(0xdeadbeef), FilesystemKind::Native);
}

#[test]
fn resolve_backend_forced_native() {
    let cfg = WatchConfig {
        backend: WatchBackendConfig::Native,
        ..WatchConfig::default()
    };
    let kinds = vec![FilesystemKind::Broken]; // would normally pick Poll
    assert_eq!(resolve_backend(&cfg, &kinds), Backend::Native);
}

#[test]
fn resolve_backend_forced_poll() {
    let cfg = WatchConfig {
        backend: WatchBackendConfig::Poll,
        ..WatchConfig::default()
    };
    let kinds = vec![FilesystemKind::Native]; // would normally pick Native
    assert_eq!(resolve_backend(&cfg, &kinds), Backend::Poll);
}

#[test]
fn resolve_backend_auto_all_native() {
    let cfg = WatchConfig::default(); // backend = Auto
    let kinds = vec![FilesystemKind::Native, FilesystemKind::Native];
    assert_eq!(resolve_backend(&cfg, &kinds), Backend::Native);
}

#[test]
fn resolve_backend_auto_one_broken_picks_poll() {
    let cfg = WatchConfig::default();
    let kinds = vec![FilesystemKind::Native, FilesystemKind::Broken];
    assert_eq!(resolve_backend(&cfg, &kinds), Backend::Poll);
}

#[test]
fn resolve_backend_auto_empty_watches_defaults_to_native() {
    // Edge case: no wikis mounted. Default to Native (no work to do anyway).
    let cfg = WatchConfig::default();
    let kinds: Vec<FilesystemKind> = vec![];
    assert_eq!(resolve_backend(&cfg, &kinds), Backend::Native);
}
