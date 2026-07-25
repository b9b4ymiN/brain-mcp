# Watcher Bind-Mount Reliability (statfs Detection + Poll Fallback)

**Status:** Draft
**Date:** 2026-07-25
**Author:** THP (via brainstorming)
**Scope:** brain-mcp vnext — `src/watch.rs` + `src/config.rs::WatchConfig`
**Out of scope:** git-aware poll fast path; Fanotify backend; path-level detection inside `watch()` calls; semantic claims / event ledger

---

## Context

### Symptom
After enabling `--watch` in the container (Layer 2 of the auto-index design),
host-side file edits are not picked up by the watcher inside the container
on **Docker Desktop for Windows** (and macOS). MCP writes still refresh the
index correctly (Layer 1, single-call write pipeline), and `auto_rebuild`
(Layer 3) recovers the index on the next boot, but there is a stale window
between a host-side edit and the next container restart during which
`wiki_search` returns outdated results.

### Root cause
`notify::RecommendedWatcher` is a thin wrapper over the OS-native event
source — `inotify` on Linux. Inside a Docker Desktop (Windows/macOS)
container, the bind mount is served through a virtualized filesystem layer
(9P on WSL2, gRPC-FUSE on macOS) that **silently does not deliver inotify
events** for writes originating on the host. `inotify_add_watch()` succeeds
without an error code, so the failure is not detectable from the return
value — the watcher simply never receives events.

This is a well-known cross-cutting issue that affects every inotify-based
tool inside a Docker Desktop container (webpack-dev-server, Vite, nodemon,
Spring DevTools, …). Industry consensus: native watchers do not work
across these virtualized bind mounts, and the only reliable fix is to fall
back to polling.

### Industry pattern (research)

The most rigorous implementation in the wild is the **Zed editor**
(`zed-industries/zed#51340`):

> "Rather than probing inotify to see if it fails, the editor proactively
> detects the underlying filesystem type. On Linux, it utilizes
> `libc::statfs()` to check for known-broken filesystem magic numbers
> associated with network and virtual drives."

Zed checks for `V9FS_MAGIC` (WSL interop), `NFS_SUPER_MAGIC`, `CIFS_MAGIC_NUMBER`,
`SMB`, and `FUSE_SUPER_MAGIC`. On any of those, it switches to
`notify::PollWatcher`. It also exposes env-var overrides
(`ZED_FILE_WATCHER_MODE`, `ZED_FILE_WATCHER_POLL_MS`).

Other relevant references:
- `notify` crate: `PollWatcher` is documented as the universal fallback
  for network shares and pseudo-filesystems. Default poll interval is 30s.
- Node.js ecosystem (`chokidar`): `CHOKIDAR_USEPOLLING=true` is the standard
  Docker Desktop workaround, applied via env var.
- Spring Boot: `spring.devtools.restart.poll-interval` (explicit poll mode).
- Vite / Webpack / nodemon: all expose a `usePolling` / `poll` option.

The pattern is consistent: **auto-detect via statfs, fall back to polling,
allow override via config / env.**

---

## Design

### Goal
Make the watcher (Layer 2) detect file changes reliably on every supported
deployment: native Linux host, Docker Desktop Windows, Docker Desktop
macOS, NFS, SMB, and any other filesystem that does not deliver native
events — without sacrificing the zero-cost event-driven behavior on local
filesystems.

### Approach
At watcher startup, **statfs-probe each mounted `wiki_root`** and look up
its filesystem magic number against a small table of known-broken virtual
/ network filesystems. If **any** watched path sits on a broken filesystem,
the watcher uses `notify::PollWatcher` for the whole tree (single backend
per watcher; we do not mix). If all watched paths are on local filesystems,
it uses `notify::RecommendedWatcher` as today.

The choice can be overridden by config (`[watch] backend = auto|native|poll`)
and by env var (`LLM_WIKI_WATCH_BACKEND`, `LLM_WIKI_WATCH_POLL_MS`). The
env var wins over the config file, mirroring the existing pattern used for
`LLM_WIKI_CONFIG` and the `[provider]` env-var indirection.

```
┌─────────────────────────────────────────────────────────────────────┐
│ run_watcher (existing loop, unchanged)                              │
│   ┌─────────────────────────────────────────────────────────────┐   │
│   │ start_notify_watcher (REFACTORED)                           │   │
│   │   resolve_backend(watch_dirs, config) → Backend             │   │
│   │     • config.backend = auto|native|poll  (default auto)     │   │
│   │     • env override: LLM_WIKI_WATCH_BACKEND                  │   │
│   │     • if auto: statfs() each wiki_root → check magic table  │   │
│   │     • poll wins ties (any broken → poll)                    │   │
│   │   build_watcher(backend, tx, cancel, poll_interval_ms)      │   │
│   │     • Native → RecommendedWatcher                           │   │
│   │     • Poll   → PollWatcher(poll_interval_ms, default 30s)   │   │
│   └─────────────────────────────────────────────────────────────┘   │
│   debounce + classify + ingest (UNCHANGED)                          │
└─────────────────────────────────────────────────────────────────────┘
```

### Key components

1. **`Backend` enum** (`src/watch.rs`, new)
   ```rust
   enum Backend { Native, Poll }
   ```

2. **`fn resolve_backend(watch_dirs, config) -> Backend`** (`src/watch.rs`, new)
   - Reads `config.watch.backend` (already resolved through env override).
   - `auto`: `statfs()` each `wiki_root`; if **any one** returns a
     known-broken magic → `Poll` (whole-watcher), else `Native`.
     Rationale: a single `run_watcher` task drives one backend; mixing
     per-wiki backends would double the test surface for no practical
     benefit since brain-mcp wikis sit on one filesystem in practice.
     `statfs()` failures are logged at WARN and treated as `Native`
     (fail-open — do not block startup).
   - `native` / `poll`: returned verbatim, bypassing statfs entirely.

3. **`fn build_watcher(backend, tx, cancel, poll_interval) -> Box<dyn Watcher>`**
   (`src/watch.rs`, new)
   - Factory that constructs either `RecommendedWatcher` or `PollWatcher`
     with the same event-handler closure used today (the closure body does
     not change — it classifies events and forwards paths to the channel).
   - `PollWatcher` is built with `Config::default().with_poll_interval(Duration::from_millis(poll_interval))`.

4. **`start_notify_watcher`** (`src/watch.rs`, refactor)
   - Replace the single `notify::recommended_watcher(...)` call with:
     `let backend = resolve_backend(&watch_dirs, &config);`
     `let mut watcher = build_watcher(backend, ...)?;`
   - Add a startup `tracing::info!` line announcing the chosen backend and
     the detection reason (e.g. `"watcher backend: poll (V9FS detected on
     wiki brain)"`) so operators can verify the detection in `docker logs`.

5. **`WatchConfig`** (`src/config.rs`, extend)
   ```rust
   pub struct WatchConfig {
       pub debounce_ms: u32,                       // existing, default 500
       pub backend: WatchBackendConfig,            // new, default auto
       pub poll_interval_ms: u32,                  // new, default 30000
   }

   #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
   #[serde(rename_all = "lowercase")]
   pub enum WatchBackendConfig {
       #[default]
       Auto,
       Native,
       Poll,
   }
   ```

6. **Env-var resolution** (`src/config.rs`)
   - After loading the config file, override `config.watch.backend` from
     `LLM_WIKI_WATCH_BACKEND` (`auto` / `native` / `poll`, case-insensitive).
   - Override `config.watch.poll_interval_ms` from `LLM_WIKI_WATCH_POLL_MS`
     (parsed as `u32`; invalid values fall back to default with a WARN log).
   - Mirrors how `console_dev_bootstrap_*_env` resolves credential env vars
     today, keeping the existing "config file is the default, env var wins"
     pattern.

### statfs() detection table

The known-broken magic numbers (Linux; values from `<linux/magic.h>`):

| Magic constant | Hex | Filesystem | Why broken |
|----------------|-----|------------|-----------|
| `V9FS_MAGIC` | `0x012ff7b5` | 9P (WSL2 interop, Docker Desktop Windows bind mount) | inotify_add_watch succeeds, no events delivered |
| `NFS_SUPER_MAGIC` | `0x6969` | NFS | Network FS, no kernel-side event delivery |
| `CIFS_MAGIC_NUMBER` | `0xff534d42` | SMB / CIFS | Network FS, no kernel-side event delivery |
| `FUSE_SUPER_MAGIC` | `0x65735546` | FUSE (gRPC-FUSE on Docker Desktop macOS) | Userspace FS, inotify unreliable across the bridge |

Everything else (ext4, xfs, btrfs, zfs, apfs, overlay2, tmpfs) is treated
as `Native` — events propagate reliably. `overlay2` (Docker's default
storage driver) explicitly stays `Native` because inotify on the merged
view works; the host-side writes that fail are the bind-mount case, which
is V9FS/FUSE, already covered.

Detection runs at watcher startup only (per `serve` invocation). If the
operator repoints a wiki at a new filesystem mid-run, they restart the
container — same as every other config change.

### Data flow

**Boot / `serve --watch`:**
```
engine::open → mount wikis → spaces populated
server::serve → spawn run_watcher
run_watcher → start_notify_watcher
  → for each wiki in spaces: statfs(wiki_root) → magic
  → resolve_backend(config + magic results) → Backend
  → tracing::info!("watcher backend: {backend:?}")
  → build_watcher(backend, tx, cancel, poll_interval)
  → watcher.watch(wiki_root, Recursive)
  → return watcher handle to run_watcher
run_watcher enters existing event loop (unchanged)
```

**Event path (unchanged):**
```
file change (event from inotify OR poll tick)
  → notify callback → classify_event → tx.send((wiki, path))
  → debounce_ms window
  → index_manager.update + sync_hugo + notify_web_refresh
```

### Why this layering is correct

- **Zero cost on native Linux** — production deployment (Linux server,
  bare metal or VM) keeps the fast inotify path. No regression.
- **Reliable on Docker Desktop** — Windows + macOS dev workflows get
  polling automatically, with no operator action required. Layer 1 still
  covers MCP writes instantly; Layer 2 now actually covers host-side
  edits too.
- **Explicit escape hatch** — if detection is wrong (rare kernel quirk,
  new filesystem type), the operator can force `backend = "native"` or
  `"poll"` without code changes, via either config file or env var.
- **Single-backend-per-watcher** keeps the event loop simple. Mixing
  per-path backends is unnecessary for brain-mcp (every wiki sits on the
  same filesystem in practice) and would double the test surface.

---

## Files Touched

| File | Change | Risk |
|------|--------|------|
| `src/watch.rs` | Add `Backend` enum, `resolve_backend`, `build_watcher`; refactor `start_notify_watcher` to call them | Medium — touches the production watcher hot path; covered by new unit tests + existing watcher integration test |
| `src/config.rs` | Extend `WatchConfig` with `backend` + `poll_interval_ms`; add `WatchBackendConfig` enum; add env-var resolution for `LLM_WIKI_WATCH_BACKEND` + `LLM_WIKI_WATCH_POLL_MS` | Low — additive config fields with serde defaults |
| `Cargo.toml` | Add `libc` dependency (already a transitive dep via notify on Linux, but pin as direct for `statfs`) | Low — `libc` is ubiquitous and stable |
| `tests/watch_backend.rs` (new) | Unit tests for `resolve_backend` against synthetic magic numbers + config overrides | None |
| `tests-integration/mcp/test_watcher_layer2.py` (existing) | Extend with a "poll mode" smoke variant that writes via host fs and asserts the page appears within `poll_interval_ms + slack` | None |
| `docs/guides/deploy-docker.md` | Document the auto-detection + env-var override | None |
| `examples/config.docker.toml` | Comment that `backend = "auto"` is the default and works on Docker Desktop out of the box | None |

### Files NOT touched

- `src/server.rs` — `serve()` already starts the watcher correctly when
  `--watch` is passed; only the watcher construction changes.
- `src/ops/*.rs` — the ingest / index path triggered by the watcher is
  unchanged.
- FTS5 / semantic claims path — separate concern.

---

## Testing

### Unit tests (`tests/watch_backend.rs`)

- `resolve_backend(config_native) → Native` (forced).
- `resolve_backend(config_poll) → Poll` (forced).
- `resolve_backend(config_auto, all_local_magic) → Native`.
- `resolve_backend(config_auto, one_v9fs_magic) → Poll`.
- `resolve_backend(config_auto, mixed_nfs_and_ext4) → Poll` (any broken wins).
- `resolve_backend(config_auto, statfs_returns_error) → Native` (fail-open + warn log).
- `build_watcher(Native, ...)` returns a `RecommendedWatcher`-backed handle.
- `build_watcher(Poll, ...)` returns a `PollWatcher`-backed handle.

`statfs()` itself is exercised indirectly through a thin wrapper that
takes the magic number as input, so the unit tests stay platform-agnostic.

### Integration test (extend `test_watcher_layer2.py`)

- Existing test already covers the host-side-write → search contract on a
  real `serve --watch` process. After the change, this test must pass on
  Docker Desktop Windows (where it currently fails) — that is the
  end-to-end proof that detection + fallback works.
- Add a `test_watcher_poll_mode_env_override` variant that starts the
  server with `LLM_WIKI_WATCH_BACKEND=poll` and asserts the startup log
  line `watcher backend: Poll`.

### Manual smoke

- Docker Desktop Windows: `docker compose up --build`; edit a wiki page
  on the host via VS Code; within 30s (default poll interval) the page
  appears in `wiki_search` with no container restart.
- Linux native: same edit; appears within debounce_ms (~500ms) — confirms
  no regression on the fast path.

---

## Trade-offs and Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| statfs() magic numbers change across kernel versions | Low | Medium (silent fallback to native) | Hardcode the 4 widely-used constants; log WARN on unknown magic; operator can force `poll` |
| PollWatcher 30s latency too slow for power users | Medium | Low | Documented env override (`LLM_WIKI_WATCH_POLL_MS=5000`); 30s default matches notify upstream |
| statfs() syscall fails | Low | Low | Fail-open to `Native` + WARN log; does not block startup |
| Detection runs only at startup; mid-run FS change undetected | Low | Low | Restart container (same as any other config change); documented |
| Mixed-filesystem wikis (one local, one NFS) | Low | Low | Poll wins (safe default); operator can split wikis across containers if needed |
| `libc` direct dependency adds build weight | Low | None | Already a transitive dep via notify on Linux; pinning as direct is a no-op for binary size |

---

## Explicit Non-Goals

- **git-aware poll fast path** (`git status --porcelain` instead of stat
  every file). YAGNI until the wiki grows past ~5k files; the current
  PollWatcher cost at brain-mcp's scale is negligible. Add when needed.
- **Path-level detection** inside each `watch()` call (per-directory
  backend selection). brain-mcp has one wiki per filesystem in practice;
  per-wiki detection is sufficient.
- **Fanotify backend** (notify's experimental Linux backend that scales
  better than inotify). API not stable in notify 8; revisit when notify 9
  ships.
- **Probe-at-startup** (write a sentinel file, see if an event arrives).
  Zed tried this and abandoned it because `inotify_add_watch()` succeeds
  silently on broken filesystems — statfs() is more reliable.
- **Removing `RecommendedWatcher`** entirely. Native events remain the
  fast path on real Linux deployments.

---

## Open Questions

None at design time. Implementation plan resolves:
- Exact place to read env vars in `config.rs` (likely alongside the
  existing `[provider]` env-var indirection).
- Whether to log the detected magic number per wiki (yes, at DEBUG level,
  for debuggability).
