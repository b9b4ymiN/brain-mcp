# Auto-Index for Wiki Writes (Write-Time + Watcher + auto_rebuild)

**Status:** Draft
**Date:** 2026-07-25
**Author:** THP (via brainstorming)
**Scope:** brain-mcp vnext — wiki layer (Markdown page index + Console refresh)
**Out of scope:** semantic claims / event ledger; FTS5 search_claims path; transactional refactor; observability stack

---

## Context

### Symptom
A new wiki page (`procedural/research/thai-stock-factsheet.md`) was written to disk
via `wiki_content_write`, but it did not appear in the Console web UI
(`http://localhost:8080/#/home`) nor in `wiki_search` results. The page only
became visible after a manual `wiki_index_rebuild`.

### Root cause (3 layers, all missing/broken)

| # | Layer | Status at time of incident | Industry-standard expectation |
|---|-------|-----------------------------|-------------------------------|
| 1 | **Write-time index** in `ops::content_write` / `ops::ingest` | ❌ Not present — only writes file + commits + syncs Hugo site, never calls `index_manager.update` | Update index in the same transaction as the write (Obsidian, Logseq, any tantivy/Lucene app) |
| 2 | **Filesystem watcher** (`src/watch.rs`) | ✅ Implemented, ✅ Debounced, incremental `index_manager.update` — **but never started**: container CMD is `serve --http :8080` (no `--watch` flag) | Catch external edits (vim, git pull, sync) that bypass the write path |
| 3 | **Startup sweep / `index.auto_rebuild`** | `auto_rebuild = false` (default per `config.rs:82`) | Defense-in-depth recovery: if the index is stale at boot, rebuild automatically |

### Why these layers all matter

The current watcher is the **only** path that updates the tantivy index. When
it is not running, no write — internal or external — refreshes the search
index. This is fragile by design: a single missing flag at deploy time
silently breaks the entire "knowledge is searchable" promise.

### Industry pattern (research)

Obsidian and Logseq — the two mature local-knowledge apps — both use:

- **Write-time indexing as primary** — in-app edits update the index
  immediately inside the write path, giving read-after-write consistency.
- **Filesystem watcher as backup** — catches edits made by other tools
  (external editor, sync clients, git operations).

Tantivy itself is designed for **incremental `add/delete + commit`** (LSM-like,
same as Lucene), so write-time indexing is the natural pattern. Full rebuilds
should be reserved for schema changes and corruption recovery.

---

## Design

### Goal
Make the search index and Console UI reflect wiki writes immediately,
without requiring a manual rebuild or a running background watcher — while
keeping the watcher as a safety net for external edits.

### Approach
Three layered defenses, each independently sufficient:

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 1 — Write-time index (PRIMARY)                        │
│   ops::content_write / ops::ingest call index_after_write   │
│   immediately after git::commit succeeds.                   │
│   Guarantees read-after-write consistency for MCP writes.   │
├─────────────────────────────────────────────────────────────┤
│ Layer 2 — Filesystem watcher (SAFETY NET)                   │
│   notify crate watches wiki_root; debounced incremental     │
│   index_manager.update on external file changes.            │
│   Catches: vim edits, git pull, sync, anything not via MCP. │
├─────────────────────────────────────────────────────────────┤
│ Layer 3 — auto_rebuild on startup (RECOVERY)                │
│   index.auto_rebuild = true in config.toml.                 │
│   If index is stale/corrupt at boot, rebuild automatically. │
└─────────────────────────────────────────────────────────────┘
```

### Key components

1. **`ops::index_after_write(wiki_name, changed_paths)`** (new helper, in
   `src/ops/index.rs` — the module already exists alongside
   `ops::index_rebuild`):
   - Calls `space.index_manager.update(wiki_root, repo_root, last_commit, ...)`
   - Calls `crate::web::sync_installed_hugo_content(repo_root, wiki_root)`
   - Fires `notify_web_refresh(web_refresh_tx, wiki_name)` if a channel is
     available
   - Returns a small report `{updated, deleted, web_synced}` for the handler
     to surface in the MCP response
   - **Idempotent**: when `last_commit` is unchanged, `update()` is a no-op.

2. **`ops::content_write`** (in `src/ops/content.rs:213`) — append
   `index_after_write(&[result.path])` after the existing `git::commit`.

3. **`ops::ingest`** (in `src/ops/ingest.rs:11`) — after the existing
   commit block, collect the `changed_paths` actually written and call
   `index_after_write(&paths)`. When `auto_commit = false`, skip indexing
   and log a warning.

4. **`docker-compose.yml`** — change the service `command` (or the
   Dockerfile `CMD`) to:
   ```
   serve --http :8080 --watch
   ```
   This enables the watcher inside the container. `notify` uses inotify on
   the Linux container side; bind-mounted host volumes relay events
   normally.

5. **`config/config.toml`** — add:
   ```toml
   [index]
   auto_rebuild = true
   ```
   This makes startup rebuild the index when staleness is detected, even
   when the watcher had died in a previous run.

6. **Watcher dedup** — no change to `src/watch.rs`. The watcher will
   continue to call `index_manager.update`, which is idempotent on
   unchanged commit hashes. The cost of the duplicate call is negligible
   (one diff against `last_commit` returns immediately).

### Data flow (new)

**MCP write path (Layer 1):**
```
client → wiki_content_write / wiki_ingest
  → validate file(s) + write to disk
  → git::commit                                (existing)
  → ops::index_after_write(changed_paths)      (NEW)
      → index_manager.update(last_commit, ...)
      → web::sync_installed_hugo_content(...)
      → notify_web_refresh(web_refresh_tx, ...)
  → response (now includes index report)
```

**External edit path (Layer 2, unchanged behavior, just enabled):**
```
editor/git pull → file change
  → notify::RecommendedWatcher fires
  → debounced (watch.debounce_ms)
  → index_manager.update
  → web::sync_installed_hugo_content
  → notify_web_refresh
```

**Boot path (Layer 3, new default):**
```
container start → engine::open
  → index_manager.status
  → if stale && auto_rebuild → rebuild (existing code path)
```

### Why this layering is correct

- **Read-after-write consistency** is the property users actually care
  about ("I wrote it, I expect to find it"). Layer 1 delivers it without
  depending on any other component.
- **External edits** are a real workflow for THP (VS Code, git pull from
  another machine). Layer 2 catches them.
- **Recovery** matters because the watcher can die (OOM, panic, OS event
  queue overflow). Layer 3 ensures a stale index is fixed on the next
  boot rather than silently serving stale results.

---

## Files Touched

| File | Change | Risk |
|------|--------|------|
| `src/ops/index.rs` | Add `index_after_write` helper (module already exists alongside `ops::index_rebuild`) | Low — composes existing primitives |
| `src/ops/content.rs` (line 213, `pub fn content_write`) | Call `index_after_write` after commit | Low — additive |
| `src/ops/ingest.rs` (line 11, `pub fn ingest`) | Call `index_after_write` after commit, when `auto_commit` | Low — additive |
| `src/mcp/handlers.rs` (`handle_content_write`, `handle_ingest`) | Replace the current `sync_web_content` call with `index_after_write` (which does tantivy index + Hugo sync + web refresh as one unit) and surface the index report in the response JSON | Low |
| `docker-compose.yml` | Add `--watch` to the service command | Low — flag is already supported |
| `config/config.toml` | Add `[index] auto_rebuild = true` | Low — flag is already supported |
| `README.md` / `docs/guides/deploy-docker.md` | Note the new defaults and what they do | None |

### Files NOT touched

- `src/watch.rs` (watcher logic — left as-is, idempotent dedup via commit hash)
- `src/mcp/handlers.rs` semantic claim handlers (`brain_capture`, `brain_confirm`,
  `brain_supersede`) — those modify the event ledger, not wiki pages
- FTS5 `search_claims` path — separate index, separate concern
- Subject Validator, semantic store, projection layer

---

## Testing

### Unit tests
- `ops::index_after_write` returns the expected `{updated, deleted}` for a
  freshly written file.
- Calling `index_after_write` twice with the same commit is a no-op on the
  second call.
- `auto_commit = false` path skips indexing and does not panic.

### Integration tests
- Write a page via the MCP handler → immediately `wiki_search` finds it
  (read-after-write contract).
- Ingest a directory via the MCP handler → all new pages appear in
  `wiki_search`.
- Stop the watcher → write a page → `wiki_search` still finds it (Layer 1
  works without watcher).
- Edit a page on disk directly (bypassing MCP) with watcher running →
  `wiki_search` reflects the change after debounce window.
- Boot with a stale index and `auto_rebuild = true` → index is rebuilt
  before the server accepts requests (or at least before search is served).

### Manual smoke
- Write `thai-stock-factsheet.md` again via `wiki_content_write` → appears
  in `http://localhost:8080/#/search` without manual rebuild.

---

## Trade-offs and Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Write latency increases by tantivy commit cost | High | Low (ms-scale) | Acceptable for single-user local app; tantivy commits are fast |
| Watcher and write-path race on the same file | Medium | None | `index_manager.update` is idempotent on unchanged commit hash |
| Docker bind-mount on Windows host does not relay inotify events | Medium | Medium | Verified path: container is Linux, mount is plain volume, inotify works inside the container. If broken in practice, Layer 1 still covers MCP writes |
| `auto_rebuild = true` slows startup on large wikis | Low | Low | brain wiki is small (~14 pages); rebuild is sub-second |
| Forgetting to index when `auto_commit = false` | Medium | Medium | Log a warning at INFO level; document the trade-off in the handler response |
| Existing tests assume no indexing in write path | Medium | Low | Update affected tests; the contract change is the entire point of this work |

---

## Explicit Non-Goals

- Indexing the **semantic claims** store (event ledger) — that is a
  separate index (`search_claims` / FTS5) with its own write path
  (`brain_confirm`, `brain_supersede`). Out of scope here.
- Refactoring the watcher for transactional semantics — it stays as a
    debounce-and-update loop.
- Adding batch indexing API or bulk-import fast paths.
- Observability stack (metrics, tracing spans around indexing).
- Auto-rebuild triggers beyond startup (e.g., on-corruption-detect).
- Removing the watcher — it remains a safety net.

---

## Open Questions

None at design time. Implementation plan will resolve file-path details
(exact location of `content_write` in the ops tree) during step 1.

---

## Follow-on: pulled into the default write path

The `ops::index_after_commit` helper delivered by this spec is reused by
spec #2 (`docs/specs/2026-07-25-merge-write-ingest-design.md`), which merges
`wiki_content_write` + `wiki_ingest` into a single-call write pipeline. After
spec #2 ships, every default `wiki_content_write` call automatically triggers
indexing — making the Layer 1 helper the engine of the merged pipeline.

(Note: spec #2 calls it `index_after_commit`, not `index_after_write`. The
naming was finalized during spec #1's implementation.)
