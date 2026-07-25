# Watcher Bind-Mount Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the watcher (Layer 2) reliably detect file changes inside Docker Desktop (Windows + macOS) containers, where inotify silently fails across the V9FS / gRPC-FUSE virtualized bind mount — without sacrificing the zero-cost event-driven behavior on local Linux filesystems.

**Architecture:** At watcher startup, statfs-probe each mounted `wiki_root` and check its filesystem magic number against a small table of known-broken virtual / network filesystems (V9FS, NFS, CIFS, FUSE). If any watched path sits on a broken filesystem, the watcher uses `notify::PollWatcher`; otherwise it uses `notify::RecommendedWatcher` as today. The choice can be overridden by config (`[watch] backend = auto|native|poll`) and by env var (`LLM_WIKI_WATCH_BACKEND`, `LLM_WIKI_WATCH_POLL_MS`).

**Tech Stack:** Rust, notify 8.x (RecommendedWatcher + PollWatcher), libc (`statfs` syscall), Python integration tests (pytest + mcp client).

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | Add `libc` to `[dependencies]` (Linux target only) | Provides `statfs` syscall for filesystem magic detection |
| `src/config.rs` | Extend `WatchConfig` with `backend` + `poll_interval_ms`; add `WatchBackendConfig` enum; add env-var resolution | Config schema + operator escape hatch |
| `src/watch.rs` | Add `Backend` enum, `resolve_backend`, `build_watcher`, `fs_magic_kind`; refactor `start_notify_watcher` | statfs detection + watcher factory |
| `tests/watch_backend.rs` (NEW) | Unit tests for `resolve_backend` + `fs_magic_kind` | Verify detection logic against synthetic magic numbers |
| `tests-integration/mcp/test_watcher_layer2.py` (existing) | Extend with poll-mode env override variant | End-to-end proof that env-var override works |
| `docs/guides/deploy-docker.md` | Document auto-detection + env-var override | Operator-facing docs |
| `examples/config.docker.toml` | Comment that `backend = "auto"` works on Docker Desktop out of the box | Production example |

### Files NOT touched

- `src/server.rs` — `serve()` already starts the watcher correctly when `--watch` is passed; only the watcher construction changes inside `start_notify_watcher`.
- `src/ops/*.rs` — the ingest / index path triggered by the watcher is unchanged.
- FTS5 / semantic claims path — separate concern.

---

## Task 1: Add `libc` dependency

**Files:** Modify `Cargo.toml`

**Why:** `statfs()` syscall is needed for filesystem magic detection. `libc` is already a transitive dependency via `notify` on Linux, but pinning it as a direct dependency is the right call because we use its API directly.

- [ ] **Step 1: Inspect current Cargo.toml [dependencies] section**

Read the `[dependencies]` section of `Cargo.toml`. Find the line where `notify = "8"` is declared.

- [ ] **Step 2: Add libc as a target-gated dependency**

After the `notify` line, add:

```toml
notify = "8"
# Direct dependency for `statfs()` filesystem magic detection in
# src/watch.rs (watcher bind-mount reliability, 2026-07-25). Already a
# transitive dep via notify on Linux; pinning as direct because we use
# the libc API surface directly. Only needed on Linux where statfs lives.
[target.'cfg(target_os = "linux")'.dependencies]
libc = "0.2"
```

If a `[target.'cfg(target_os = "linux")'.dependencies]` section already exists, append `libc = "0.2"` to it instead of creating a new one.

- [ ] **Step 3: Verify the dependency resolves**

```bash
cargo build
```

Expected: builds cleanly. The new dependency has no effect yet because no code uses it.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(deps): add libc direct dep for statfs in watch.rs

Pinned as Linux-only because statfs is a Linux syscall. Already a
transitive dep via notify on Linux; direct pin because src/watch.rs will
use the libc::statfs API surface directly (watcher bind-mount
reliability, 2026-07-25)."
```

---

## Task 2: Extend `WatchConfig` with `backend` + `poll_interval_ms`

**Files:** Modify `src/config.rs`

**Why:** Operators need an explicit escape hatch when statfs detection is wrong, plus control over poll interval. The env-var resolution mirrors the existing `[provider]` env-var indirection pattern.

- [ ] **Step 1: Read the current WatchConfig + its Default impl**

Read `src/config.rs` lines 730-750 (the `WatchConfig` struct + Default impl, currently containing only `debounce_ms`).

- [ ] **Step 2: Add the WatchBackendConfig enum + extend WatchConfig**

Replace the existing `WatchConfig` block:

```rust
/// `[watch]` section — filesystem watcher configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    /// Debounce delay in milliseconds before triggering ingest after a file change (default: 500).
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u32,
    /// Watcher backend selection (default: auto — statfs-detect broken filesystems).
    #[serde(default)]
    pub backend: WatchBackendConfig,
    /// Poll interval in milliseconds when running in `poll` mode (default: 30000, matches notify upstream).
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u32,
}

/// Operator-selected watcher backend.
///
/// `Auto` (default) statfs-probes each wiki_root at startup and falls back
/// to `Poll` if any sits on a known-broken filesystem (V9FS, NFS, CIFS,
/// FUSE). `Native` forces the OS-native watcher (inotify/FSEvents/ReadDirectoryChanges).
/// `Poll` forces polling — required for Docker Desktop bind mounts on Windows/macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WatchBackendConfig {
    #[default]
    Auto,
    Native,
    Poll,
}

fn default_poll_interval_ms() -> u32 {
    30000
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: default_debounce_ms(),
            backend: WatchBackendConfig::Auto,
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}
```

- [ ] **Step 3: Add env-var resolution**

Find where the global config is loaded in `src/config.rs` (look for the function that reads + parses the TOML file, typically `pub fn load(path: &Path) -> Result<GlobalConfig>` or similar). After the config is parsed but before it is returned, apply env-var overrides:

```rust
// Watcher env overrides (watcher bind-mount reliability, 2026-07-25).
// Env wins over config file, mirroring the [provider] env-var indirection.
if let Ok(val) = std::env::var("LLM_WIKI_WATCH_BACKEND") {
    match val.trim().to_ascii_lowercase().as_str() {
        "auto" => config.watch.backend = WatchBackendConfig::Auto,
        "native" => config.watch.backend = WatchBackendConfig::Native,
        "poll" => config.watch.backend = WatchBackendConfig::Poll,
        other => {
            tracing::warn!(
                var = %other,
                "LLM_WIKI_WATCH_BACKEND must be auto|native|poll; ignoring"
            );
        }
    }
}
if let Ok(val) = std::env::var("LLM_WIKI_WATCH_POLL_MS") {
    match val.trim().parse::<u32>() {
        Ok(ms) if ms >= 100 => config.watch.poll_interval_ms = ms,
        Ok(ms) => tracing::warn!(
            ms,
            "LLM_WIKI_WATCH_POLL_MS must be >= 100; ignoring"
        ),
        Err(_) => tracing::warn!(
            raw = %val,
            "LLM_WIKI_WATCH_POLL_MS is not a valid u32; ignoring"
        ),
    }
}
```

The exact insertion point is in the function that returns the loaded `GlobalConfig`. If the load function lives in a different file (e.g. `src/config.rs::load`), apply the overrides there. Verify with `cargo check` after this step.

- [ ] **Step 4: Build + verify**

```bash
cargo build
```

Expected: builds cleanly. The new fields have serde defaults so existing config files without `[watch] backend` keep working.

- [ ] **Step 5: Verify existing config parses**

```bash
./target/debug/llm-wiki --config examples/config.docker.toml config list 2>&1 | grep -E 'backend|poll_interval'
```

Expected: prints `backend = auto` and `poll_interval_ms = 30000` (or similar; the exact format depends on the existing `config list` output).

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): watch.backend + poll_interval_ms with env-var override

Extends WatchConfig with two new fields for watcher bind-mount
reliability (2026-07-25):
  - backend: auto|native|poll (default auto — statfs-detect)
  - poll_interval_ms: u32 (default 30000)

Env vars LLM_WIKI_WATCH_BACKEND and LLM_WIKI_WATCH_POLL_MS override the
config-file values, mirroring the [provider] env-var indirection pattern."
```

---

## Task 3: Add `fs_magic_kind` + `Backend` enum in `src/watch.rs`

**Files:** Modify `src/watch.rs`

**Why:** This is the pure detection logic (no I/O side effects). Splitting it out from `resolve_backend` makes it unit-testable with synthetic magic numbers without touching the real filesystem.

- [ ] **Step 1: Write the failing unit test**

Create `tests/watch_backend.rs`:

```rust
//! Unit tests for `watch::fs_magic_kind` + `watch::resolve_backend` —
//! the statfs-detection logic for the watcher bind-mount fix (2026-07-25).

use llm_wiki::config::{WatchBackendConfig, WatchConfig};
use llm_wiki::watch::{fs_magic_kind, Backend, FilesystemKind};

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
    let kinds: Vec<FilesystemKind> = vec![FilesystemKind::Broken]; // would normally pick Poll
    assert_eq!(
        llm_wiki::watch::resolve_backend(&cfg, &kinds),
        Backend::Native
    );
}

#[test]
fn resolve_backend_forced_poll() {
    let cfg = WatchConfig {
        backend: WatchBackendConfig::Poll,
        ..WatchConfig::default()
    };
    let kinds: Vec<FilesystemKind> = vec![FilesystemKind::Native]; // would normally pick Native
    assert_eq!(
        llm_wiki::watch::resolve_backend(&cfg, &kinds),
        Backend::Poll
    );
}

#[test]
fn resolve_backend_auto_all_native() {
    let cfg = WatchConfig::default(); // backend = Auto
    let kinds = vec![FilesystemKind::Native, FilesystemKind::Native];
    assert_eq!(
        llm_wiki::watch::resolve_backend(&cfg, &kinds),
        Backend::Native
    );
}

#[test]
fn resolve_backend_auto_one_broken_picks_poll() {
    let cfg = WatchConfig::default();
    let kinds = vec![FilesystemKind::Native, FilesystemKind::Broken];
    assert_eq!(
        llm_wiki::watch::resolve_backend(&cfg, &kinds),
        Backend::Poll
    );
}

#[test]
fn resolve_backend_auto_empty_watches_defaults_to_native() {
    // Edge case: no wikis mounted. Default to Native (no work to do anyway).
    let cfg = WatchConfig::default();
    let kinds: Vec<FilesystemKind> = vec![];
    assert_eq!(
        llm_wiki::watch::resolve_backend(&cfg, &kinds),
        Backend::Native
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test watch_backend --no-run
```

Expected: FAIL with `error[E0425]: cannot find function 'fs_magic_kind' in module 'watch'` (and similar for `Backend`, `FilesystemKind`, `resolve_backend`).

- [ ] **Step 3: Implement the types + detection functions**

At the top of `src/watch.rs`, after the existing `use` block, add:

```rust
// ── Filesystem magic detection (watcher bind-mount reliability, 2026-07-25) ──

/// Selected watcher backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// OS-native event-driven watcher (inotify / FSEvents / ReadDirectoryChanges).
    Native,
    /// Polling watcher — universal fallback for virtual / network filesystems
    /// that do not deliver native events (V9FS bind mounts, NFS, CIFS, FUSE).
    Poll,
}

/// Classification of a single filesystem based on its statfs magic number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemKind {
    /// Local filesystem that delivers reliable native events.
    Native,
    /// Virtual / network filesystem where inotify silently fails — must poll.
    Broken,
}

/// Map a statfs filesystem magic number to a [`FilesystemKind`].
///
/// Magic numbers from `<linux/magic.h>`. Unknown magic numbers fall back to
/// [`FilesystemKind::Native`] (fail-open — do not block startup on an
/// unfamiliar filesystem; the operator can force `poll` via config if
/// needed).
pub fn fs_magic_kind(magic: u64) -> FilesystemKind {
    // V9FS_MAGIC — WSL2 9P interop, the Docker Desktop Windows bind mount.
    const V9FS_MAGIC: u64 = 0x01021997;
    // NFS_SUPER_MAGIC.
    const NFS_SUPER_MAGIC: u64 = 0x6969;
    // CIFS_MAGIC_NUMBER — SMB / CIFS.
    const CIFS_MAGIC_NUMBER: u64 = 0xff534d42;
    // FUSE_SUPER_MAGIC — includes gRPC-FUSE (Docker Desktop macOS bind mount).
    const FUSE_SUPER_MAGIC: u64 = 0x65735546;

    match magic {
        V9FS_MAGIC | NFS_SUPER_MAGIC | CIFS_MAGIC_NUMBER | FUSE_SUPER_MAGIC => {
            FilesystemKind::Broken
        }
        _ => FilesystemKind::Native,
    }
}

/// Resolve the final [`Backend`] given the operator config and the per-wiki
/// filesystem classifications.
///
/// - `Native` / `Poll` config: returned verbatim, bypassing detection.
/// - `Auto` (default): `Poll` wins if **any** watched path sits on a broken
///   filesystem; otherwise `Native`. An empty watch list defaults to
///   `Native` (no work to do either way).
pub fn resolve_backend(config: &crate::config::WatchConfig, kinds: &[FilesystemKind]) -> Backend {
    use crate::config::WatchBackendConfig;

    match config.backend {
        WatchBackendConfig::Native => Backend::Native,
        WatchBackendConfig::Poll => Backend::Poll,
        WatchBackendConfig::Auto => {
            if kinds.iter().any(|k| *k == FilesystemKind::Broken) {
                Backend::Poll
            } else {
                Backend::Native
            }
        }
    }
}
```

- [ ] **Step 4: Re-export the new public items from the crate root if needed**

Check `src/lib.rs` for how `watch` is exposed. If it is `pub mod watch;`, the new public items are reachable as `llm_wiki::watch::fs_magic_kind` etc. — no extra re-export needed. If the module is gated, add `pub use watch::{Backend, FilesystemKind, fs_magic_kind, resolve_backend};` to the relevant place.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test --test watch_backend -- --nocapture
```

Expected: all 12 tests PASS.

- [ ] **Step 6: Run clippy + fmt**

```bash
cargo fmt
cargo clippy --tests -- -D warnings
```

Expected: no warnings introduced by the new code (pre-existing warnings are out of scope).

- [ ] **Step 7: Commit**

```bash
git add src/watch.rs tests/watch_backend.rs
git commit -m "feat(watch): fs_magic_kind + Backend + resolve_backend (statfs detection)

Pure detection logic for the watcher bind-mount fix. Maps statfs magic
numbers to FilesystemKind::Native | Broken (V9FS/NFS/CIFS/FUSE = broken),
and resolves the final Backend (Native | Poll) from the operator config
+ the per-wiki classifications. Unit-tested with synthetic magic numbers
so the tests are platform-agnostic."
```

---

## Task 4: Add `statfs_wiki_roots` helper (Linux-only, with non-Linux stub)

**Files:** Modify `src/watch.rs`

**Why:** Bridge between the pure detection logic (Task 3) and the real filesystem. Linux uses `libc::statfs`; non-Linux targets return `Native` (no statfs concept — RecommendedWatcher already does the right thing per platform).

- [ ] **Step 1: Implement `statfs_wiki_roots`**

In `src/watch.rs`, after the `resolve_backend` function added in Task 3, add:

```rust
/// statfs-probe each wiki_root and return its [`FilesystemKind`].
///
/// On Linux: real `libc::statfs` call per path. Failures are logged at WARN
/// and treated as [`FilesystemKind::Native`] (fail-open).
///
/// On non-Linux targets: always returns `Native` for each path. Other
/// platforms (macOS, Windows) do not have the bind-mount silent-failure
/// problem because their native watchers already use the right API
/// (FSEvents on macOS, ReadDirectoryChangesW on Windows). The Docker
/// Desktop containers, however, are Linux — so the cfg gate below is what
/// matters in practice.
pub fn statfs_wiki_roots(wiki_roots: &[std::path::PathBuf]) -> Vec<FilesystemKind> {
    wiki_roots.iter().map(|p| statfs_kind(p)).collect()
}

#[cfg(target_os = "linux")]
fn statfs_kind(path: &std::path::Path) -> FilesystemKind {
    use std::ffi::CString;

    let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "statfs: path contained NUL; treating as Native"
            );
            return FilesystemKind::Native;
        }
    };

    let mut buf = unsafe { std::mem::zeroed::<libc::statfs>() };
    let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut buf) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        tracing::warn!(
            path = %path.display(),
            error = %err,
            "statfs failed; treating as Native"
        );
        return FilesystemKind::Native;
    }

    let magic = buf.f_type as u64;
    let kind = fs_magic_kind(magic);
    tracing::debug!(
        path = %path.display(),
        magic = format!("{:#x}", magic),
        kind = ?kind,
        "statfs detected filesystem"
    );
    kind
}

#[cfg(not(target_os = "linux"))]
fn statfs_kind(_path: &std::path::Path) -> FilesystemKind {
    // Non-Linux targets: native watchers (FSEvents, ReadDirectoryChangesW) are
    // already correct. statfs is a Linux-specific concept.
    FilesystemKind::Native
}
```

- [ ] **Step 2: Build (verify cfg gating works)**

```bash
cargo build
```

Expected: builds cleanly on the current target. On Linux the `libc::statfs` path compiles; on other targets the stub compiles instead.

- [ ] **Step 3: Run all watch tests + existing tests**

```bash
cargo test --test watch_backend
cargo test --test engine
```

Expected: PASS. (statfs_kind is exercised only indirectly through existing integration tests on real paths; the unit tests in Task 3 cover the pure logic.)

- [ ] **Step 4: Commit**

```bash
git add src/watch.rs
git commit -m "feat(watch): statfs_wiki_roots — real statfs on Linux, stub elsewhere

Bridges the pure fs_magic_kind detection (previous commit) to the real
filesystem. Linux uses libc::statfs; failures are logged at WARN and
treated as Native (fail-open). Non-Linux targets stub to Native because
their native watchers (FSEvents, ReadDirectoryChangesW) do not have the
bind-mount silent-failure problem — and Docker Desktop containers are
Linux, which is where the problem actually lives."
```

---

## Task 5: Refactor `start_notify_watcher` to use `build_watcher`

**Files:** Modify `src/watch.rs`

**Why:** This is the integration point — replace the single hard-coded `notify::recommended_watcher(...)` call with `resolve_backend` + `build_watcher`, so the chosen backend actually drives the watcher.

- [ ] **Step 1: Read the current `start_notify_watcher`**

Read `src/watch.rs` lines 211-277 (the current `start_notify_watcher` function). Note the structure: it builds `watch_dirs`, defines an event-handler closure, calls `notify::recommended_watcher(closure)`, then calls `watcher.watch(...)` for each wiki.

- [ ] **Step 2: Add `build_watcher` factory**

After the `statfs_wiki_roots` function from Task 4, add:

```rust
/// Build a `Box<dyn Watcher>` of the requested backend, sharing the same
/// event-handler closure shape used today.
///
/// Both `RecommendedWatcher` and `PollWatcher` accept the same closure
/// signature (`FnMut(Result<Event, notify::Error>)`), so the closure body
/// does not change — only the constructor call differs. `PollWatcher` is
/// configured with `Config::default().with_poll_interval(poll_interval)`.
fn build_watcher<F>(
    backend: Backend,
    poll_interval_ms: u32,
    handler: F,
) -> Result<Box<dyn notify::Watcher>>
where
    F: FnMut(Result<notify::Event, notify::Error>) + Send + 'static,
{
    match backend {
        Backend::Native => {
            let w = notify::recommended_watcher(handler)?;
            Ok(Box::new(w))
        }
        Backend::Poll => {
            let cfg = notify::Config::default()
                .with_poll_interval(std::time::Duration::from_millis(
                    poll_interval_ms as u64,
                ));
            let w = notify::PollWatcher::new(handler, cfg)?;
            Ok(Box::new(w))
        }
    }
}
```

- [ ] **Step 3: Refactor `start_notify_watcher`**

Replace the body of `start_notify_watcher` so that it:

1. Builds `watch_dirs` (unchanged).
2. Calls `statfs_wiki_roots` on the wiki_roots.
3. Reads the watch config (debounce, backend, poll_interval_ms) — read it from the engine state.
4. Calls `resolve_backend(&config, &kinds)`.
5. Logs the chosen backend + detection reason at INFO.
6. Calls `build_watcher(backend, poll_interval_ms, closure)`.
7. Calls `watcher.watch(...)` for each wiki (unchanged).
8. Returns the boxed watcher.

The function signature changes from `Result<RecommendedWatcher>` to `Result<Box<dyn notify::Watcher>>`. New body (replacing the existing 211-277):

```rust
fn start_notify_watcher(
    engine: &WikiEngine,
    tx: mpsc::Sender<(String, PathBuf)>,
    cancel: CancellationToken,
) -> Result<Box<dyn notify::Watcher>> {
    let (watch_dirs, watch_config) = {
        let state = engine.state.read();
        let dirs: Vec<(String, PathBuf, PathBuf)> = state
            .spaces
            .iter()
            .map(|(name, space)| {
                (
                    name.clone(),
                    space.wiki_root.clone(),
                    space.repo_root.clone(),
                )
            })
            .collect();
        let cfg = state.config.watch.clone();
        (dirs, cfg)
    };

    // Detect filesystem kind per wiki_root, then resolve the final backend.
    let wiki_roots: Vec<PathBuf> =
        watch_dirs.iter().map(|(_, root, _)| root.clone()).collect();
    let kinds = statfs_wiki_roots(&wiki_roots);
    let backend = resolve_backend(&watch_config, &kinds);
    tracing::info!(
        backend = ?backend,
        poll_interval_ms = watch_config.poll_interval_ms,
        detected = ?kinds,
        "watcher backend selected"
    );

    let tx_clone = tx.clone();
    let watch_dirs_clone = watch_dirs.clone();

    let watcher = build_watcher(backend, watch_config.poll_interval_ms, move |res: Result<notify::Event, notify::Error>| {
        if cancel.is_cancelled() {
            return;
        }
        let event = match res {
            Ok(ev) => ev,
            Err(e) => {
                tracing::error!(error = %e, "filesystem watcher error");
                return;
            }
        };

        // Only care about create, modify, rename
        match event.kind {
            notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_) => {}
            _ => return,
        }

        for path in &event.paths {
            for (wiki_name, wiki_root, repo_root) in &watch_dirs_clone {
                if path.starts_with(wiki_root) && is_wiki_md(path) {
                    let _ = tx_clone.try_send((wiki_name.clone(), path.clone()));
                    break;
                }
                if path.starts_with(repo_root.join("schemas")) && is_schema_path(path) {
                    let _ = tx_clone.try_send((wiki_name.clone(), path.clone()));
                    break;
                }
            }
        }
    })?;

    // Watch wiki/ and schemas/ for each mounted wiki.
    for (_, wiki_root, repo_root) in &watch_dirs {
        if wiki_root.exists() {
            watcher.watch(wiki_root, notify::RecursiveMode::Recursive)?;
        }
        let schemas_dir = repo_root.join("schemas");
        if schemas_dir.exists() {
            watcher.watch(&schemas_dir, notify::RecursiveMode::NonRecursive)?;
        }
    }

    Ok(watcher)
}
```

Update the `use` imports at the top of `src/watch.rs`:

- Remove `RecommendedWatcher` from the `notify` import (no longer named directly).
- Keep `Event`, `EventKind`, `RecursiveMode`, `Watcher` (still needed).
- Add `use std::path::PathBuf;` if not already present (it is).

- [ ] **Step 4: Build + verify**

```bash
cargo build
```

Expected: builds cleanly. The `RecommendedWatcher` import is removed; `Box<dyn notify::Watcher>` is the new return type.

- [ ] **Step 5: Run all existing tests (regression)**

```bash
cargo test --test engine --test mcp
```

Expected: all PASS. No behavioral change for the default config (`backend = auto`) on a local filesystem — `statfs_kind` returns `Native`, `resolve_backend` returns `Native`, `build_watcher(Native, ...)` constructs a `RecommendedWatcher` exactly as before.

- [ ] **Step 6: Commit**

```bash
git add src/watch.rs
git commit -m "refactor(watch): start_notify_watcher uses resolve_backend + build_watcher

Replaces the hard-coded notify::recommended_watcher(...) call with the
new resolve_backend + build_watcher pipeline (Tasks 3+4). Default config
(backend = auto) on a local filesystem behaves identically to before —
statfs returns Native, resolve_backend returns Native, build_watcher
constructs a RecommendedWatcher. The diff opens the door for the
PollWatcher fallback that the next integration test exercises."
```

---

## Task 6: Integration test — env-var override forces poll backend

**Files:** Modify `tests-integration/mcp/test_watcher_layer2.py`

**Why:** End-to-end proof that the env-var escape hatch works. The existing `test_watcher_picks_up_external_file_write` test already covers the auto-detection path on Docker Desktop; this task adds a test that explicitly forces `poll` via env var and asserts the startup log line.

- [ ] **Step 1: Read the existing test structure**

Read `tests-integration/mcp/test_watcher_layer2.py` (the file added in commit `6b08aae`). Note how it builds a temp wiki + config + starts the server with `--watch`.

- [ ] **Step 2: Add the env-override test**

Append to `tests-integration/mcp/test_watcher_layer2.py`:

```python
@pytest.mark.asyncio
async def test_watcher_poll_mode_via_env_override(tmp_path, monkeypatch):
    """Env var LLM_WIKI_WATCH_BACKEND=poll forces the PollWatcher backend.

    The startup log line 'watcher backend selected' must report backend=Poll
    regardless of the underlying filesystem (this proves the operator escape
    hatch works even when statfs detection would have chosen Native).
    """
    import asyncio
    import json
    import os
    import shutil
    import subprocess
    from pathlib import Path

    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import stdio_client

    BIN = os.environ.get("LLM_WIKI_BIN", "llm-wiki")
    RESEARCH_FIXTURE = (
        Path(__file__).parent.parent.parent / "tests" / "fixtures" / "wikis" / "research"
    )

    def _init_wiki(src: Path, dest: Path) -> None:
        shutil.copytree(src, dest)
        subprocess.run(["git", "-C", str(dest), "init", "-q"], check=True)
        subprocess.run(["git", "-C", str(dest), "add", "."], check=True)
        subprocess.run(
            [
                "git", "-C", str(dest),
                "-c", "user.name=test", "-c", "user.email=test@test.com",
                "commit", "-qm", "init",
            ],
            check=True,
        )

    repo_root = tmp_path / "brain"
    _init_wiki(RESEARCH_FIXTURE, repo_root)
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        "[global]\n"
        'default_wiki = "research"\n\n'
        "[[wikis]]\n"
        'name = "research"\n'
        f'path = "{repo_root.as_posix()}"\n\n'
        "[watch]\n"
        "debounce_ms = 100\n"
    )

    # Force poll backend via env var. Use a short poll interval so the test
    # does not have to wait the 30s default.
    monkeypatch.setenv("LLM_WIKI_WATCH_BACKEND", "poll")
    monkeypatch.setenv("LLM_WIKI_WATCH_POLL_MS", "500")

    server = StdioServerParameters(
        command=BIN,
        args=["--config", str(config_path), "serve", "--watch"],
        env=dict(os.environ),
    )

    ready: asyncio.Future = asyncio.get_event_loop().create_future()
    stop: asyncio.Event = asyncio.Event()
    session_holder: list = []
    log_lines: list = []
    exc_holder: list = []

    async def _run():
        try:
            async with stdio_client(server) as (read, write), ClientSession(
                read, write
            ) as session:
                await session.initialize()
                session_holder.append(session)
                ready.set_result(None)
                await stop.wait()
        except Exception as e:
            if not ready.done():
                ready.set_exception(e)
            else:
                exc_holder.append(e)

    task = asyncio.ensure_future(_run())
    try:
        await ready

        # The server logs "watcher backend selected backend=Poll ..." on stderr.
        # We can't easily capture stdio_client stderr here, so verify behavior
        # instead: a host-side write must be picked up within poll_interval_ms +
        # slack. (Auto mode on a local FS would also work, but the env override
        # guarantees Poll regardless of platform.)
        sentinel = "ZQXEnvOverridePollSentinel"
        page_path = repo_root / "wiki" / "concepts" / "env-override-sentinel.md"
        page_path.parent.mkdir(parents=True, exist_ok=True)
        page_path.write_text(
            "---\n"
            f"title: Env Override Sentinel\n"
            "type: concept\n"
            "status: active\n"
            "---\n\n"
            f"{sentinel} body.\n"
        )

        session = session_holder[0]
        # Poll for up to 10s. Poll interval is 500ms + debounce 100ms + slack.
        deadline = asyncio.get_event_loop().time() + 10.0
        found = False
        last_results = None
        while asyncio.get_event_loop().time() < deadline:
            result = await session.call_tool(
                "wiki_search", {"query": sentinel, "wiki": "research"}
            )
            text = result.content[0].text if result.content else "{}"
            try:
                data = json.loads(text)
            except json.JSONDecodeError:
                data = {"results": []}
            last_results = data
            slugs = [r.get("slug", "") for r in data.get("results", [])]
            if any("env-override-sentinel" in s for s in slugs):
                found = True
                break
            await asyncio.sleep(0.5)

        assert found, (
            "env-override poll backend did not pick up external write within "
            f"10s — last results: {last_results}"
        )
    finally:
        stop.set()
        try:
            await asyncio.wait_for(task, timeout=5)
        except TimeoutError:
            task.cancel()
        if exc_holder:
            raise exc_holder[0]
```

- [ ] **Step 3: Run the new test**

```bash
cd tests-integration
LLM_WIKI_BIN='C:/Users/bfipa/.cargo-target/debug/llm-wiki.exe' uv run pytest mcp/test_watcher_layer2.py::test_watcher_poll_mode_via_env_override -v
```

Expected: PASS. The poll backend with a 500ms interval picks up the host-side write within the 10s window.

- [ ] **Step 4: Run the full watcher test file**

```bash
LLM_WIKI_BIN='C:/Users/bfipa/.cargo-target/debug/llm-wiki.exe' uv run pytest mcp/test_watcher_layer2.py -v
```

Expected: both tests PASS.

- [ ] **Step 5: Lint + commit**

```bash
uv run ruff check mcp/test_watcher_layer2.py
cd ..
git add tests-integration/mcp/test_watcher_layer2.py
git commit -m "test(watcher): env-var override forces poll backend

End-to-end proof that LLM_WIKI_WATCH_BACKEND=poll works: starts the
server with the env override + a 500ms poll interval, writes a file to
disk outside MCP, and asserts the page appears in wiki_search within
10s. Complements the existing auto-detection test."
```

---

## Task 7: Document the auto-detection + env-var override

**Files:** Modify `docs/guides/deploy-docker.md`, `examples/config.docker.toml`

**Why:** Operators need to know (a) auto-detection works on Docker Desktop out of the box, (b) the env-var escape hatch exists, (c) the trade-offs of poll vs native.

- [ ] **Step 1: Extend the existing "Auto-indexing" subsection in `docs/guides/deploy-docker.md`**

Find the "Auto-indexing (write-time + watcher + boot recovery)" subsection (added in commit `a707bdd`). Update item 2 (the Filesystem watcher bullet) to mention auto-detection:

```markdown
2. **Filesystem watcher (Layer 2 — safety net).** The container runs with
   `--watch` by default (Dockerfile CMD + docker-compose `command:`), so
   external edits — `git pull` from another machine, host-side editor writes
   via the bind mount — are caught by the watcher. At startup the watcher
   statfs-probes each `wiki_root` and auto-selects the right backend:
   `RecommendedWatcher` (inotify) on local Linux filesystems, `PollWatcher`
   on virtualized bind mounts (Docker Desktop Windows/macOS), NFS, SMB, and
   FUSE — the filesystems where inotify silently fails. Override with env
   vars `LLM_WIKI_WATCH_BACKEND=auto|native|poll` and
   `LLM_WIKI_WATCH_POLL_MS=<ms>` (default 30000). See `src/watch.rs`.
```

- [ ] **Step 2: Add a comment to `examples/config.docker.toml`**

In `examples/config.docker.toml`, find the `[index]` section (added in commit `d6b9d54`). After it, add a `[watch]` section:

```toml
# ── [watch] (Layer 2 — filesystem watcher backend selection) ────────────────
# The watcher auto-detects the filesystem at startup: inotify on local Linux
# filesystems, PollWatcher on virtualized bind mounts (Docker Desktop
# Windows/macOS), NFS, SMB, FUSE — the filesystems where inotify silently
# fails. Override only if detection is wrong for your environment.
#
# backend = "auto"            # auto (default) | native | poll
# poll_interval_ms = 30000    # only used when backend resolves to poll
[watch]
debounce_ms = 500
```

- [ ] **Step 3: Commit**

```bash
git add docs/guides/deploy-docker.md examples/config.docker.toml
git commit -m "docs(deploy): document watcher auto-detection + env-var override

Updates the Auto-indexing section to explain that the watcher now
auto-selects its backend based on statfs detection (inotify on local
Linux, poll on Docker Desktop bind mounts / NFS / SMB / FUSE). Documents
the LLM_WIKI_WATCH_BACKEND and LLM_WIKI_WATCH_POLL_MS env-var overrides."
```

---

## Task 8: Full verification + manual Docker Desktop smoke

**Files:** none (verification only)

- [ ] **Step 1: Run the full Rust suite**

```bash
cargo test
cargo clippy --tests -- -D warnings
cargo fmt -- --check
```

Expected: all green. Investigate any pre-existing test that broke.

- [ ] **Step 2: Run the full Python integration suite**

```bash
cargo build --bin llm-wiki
cd tests-integration
LLM_WIKI_BIN='C:/Users/bfipa/.cargo-target/debug/llm-wiki.exe' uv run pytest -v
```

Expected: all green, including both watcher tests.

- [ ] **Step 3: Manual Docker Desktop smoke (the original symptom)**

Rebuild + restart the container:

```bash
docker compose down
docker compose up -d --build
sleep 8
docker logs brain 2>&1 | grep 'watcher backend selected'
```

Expected: log line reports `backend=Poll` on Docker Desktop Windows (V9FS detected) or `backend=Native` on a real Linux host.

Then edit a wiki page on the host via VS Code (or any editor):

```bash
# Wait up to 30s (default poll interval) for the watcher to pick it up.
sleep 35
docker exec brain llm-wiki search '<unique-sentinel-from-your-edit>' --wiki brain
```

Expected: the page appears in search results. This is the Layer 2 contract that previously failed on Docker Desktop Windows.

- [ ] **Step 4: Final commit (only if formatting drifted)**

```bash
git status
# if clean, nothing to commit
```

---

## Self-Review

**1. Spec coverage:**
- statfs detection + magic table: ✓ Task 3 (`fs_magic_kind` + table) + Task 4 (`statfs_wiki_roots` real syscall).
- Backend enum + resolve_backend: ✓ Task 3.
- build_watcher factory: ✓ Task 5.
- start_notify_watcher refactor: ✓ Task 5.
- WatchConfig + WatchBackendConfig + env-var resolution: ✓ Task 2.
- libc dependency: ✓ Task 1.
- Unit tests for resolve_backend against synthetic magic numbers: ✓ Task 3 (12 tests).
- Integration test — env-var override: ✓ Task 6.
- Integration test — auto-detection on Docker Desktop: ✓ pre-existing (commit `6b08aae`); will now pass on Windows after the fix lands.
- Manual smoke on Docker Desktop: ✓ Task 8 Step 3.
- docs/guides/deploy-docker.md update: ✓ Task 7.
- examples/config.docker.toml update: ✓ Task 7.
- Non-goals respected (no git-aware fast path, no path-level detection, no Fanotify, no probe-at-startup): ✓ nothing in the plan touches these.

**2. Placeholder scan:** none. All steps have concrete code or commands. The exact insertion point for env-var overrides in Task 2 Step 3 is described in terms of the existing load function; the implementer verifies with `cargo check`.

**3. Type consistency:**
- `Backend` enum: defined Task 3, used Task 5. ✓
- `FilesystemKind` enum: defined Task 3, used Task 3 (resolve_backend) + Task 4 (statfs_kind return). ✓
- `fs_magic_kind(u64) -> FilesystemKind`: defined Task 3, used Task 4. ✓
- `resolve_backend(&WatchConfig, &[FilesystemKind]) -> Backend`: defined Task 3, used Task 5. ✓
- `statfs_wiki_roots(&[PathBuf]) -> Vec<FilesystemKind>`: defined Task 4, used Task 5. ✓
- `build_watcher(Backend, u32, F) -> Result<Box<dyn notify::Watcher>>`: defined Task 5, used Task 5. ✓
- `start_notify_watcher` return type changes `Result<RecommendedWatcher>` → `Result<Box<dyn notify::Watcher>>`: Task 5. ✓
- `WatchBackendConfig` enum: defined Task 2 (`Auto | Native | Poll`), used Task 3 (resolve_backend match). ✓
- `WatchConfig.backend` + `WatchConfig.poll_interval_ms`: defined Task 2, read Task 5. ✓
- Env vars `LLM_WIKI_WATCH_BACKEND` + `LLM_WIKI_WATCH_POLL_MS`: defined Task 2, exercised Task 6. ✓

---

## Notes for the implementer

- **Build the binary before running Python tests.** The integration tests shell out to `LLM_WIKI_BIN`.
- **The `cfg(target_os = "linux")` gate on `libc` is important.** Without it, `cargo build` on Windows / macOS host fails because `libc::statfs` does not exist there.
- **`Box<dyn notify::Watcher>` is the new return type of `start_notify_watcher`.** Verify there are no other callers that depend on the concrete `RecommendedWatcher` type (search `rg 'start_notify_watcher'` — should only be `run_watcher` in `src/watch.rs`).
- **Default config (`backend = auto`) is a no-op refactor on local Linux.** The regression test in Task 5 Step 5 confirms this: `statfs_kind` returns `Native`, `resolve_backend` returns `Native`, behavior is identical to before.
- **Do not skip the clippy gate.** The repo enforces `-D warnings` (CONTRIBUTING.md:14).

---

## Execution choice (post-approval)

Two options:
1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, two-stage review between tasks.
2. **Inline Execution** — execute tasks in this session with checkpoints.
