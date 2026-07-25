# Merge `wiki_content_write` + `wiki_ingest` (Single-Call Write Pipeline)

**Status:** Draft
**Date:** 2026-07-25
**Author:** THP (via brainstorming)
**Scope:** brain-mcp vnext — MCP write API for wiki pages
**Depends on:** `docs/specs/2026-07-25-auto-index-design.md` (this spec assumes
`ops::index_after_write` exists; build order is #1 → #2)
**Out of scope:** semantic claims write path; `wiki_ingest` removal; `wiki_content_new` merge

---

## Context

### Symptom (originally mis-diagnosed)
A new wiki page (`procedural/research/thai-stock-factsheet.md`) was reported as
"missing from the web console". The first hypothesis was that the file had no
git history — i.e. that the writing agent had skipped the commit step.

### Verified facts

| # | Fact | Evidence |
|---|------|----------|
| 1 | The file **does** have git history | `data/wikis/brain` commit `07735c8` "ingest: procedural/research/thai-stock-factsheet.md — +1 pages, +0 assets" (2026-07-25 02:22:27 +0000) |
| 2 | `git ls-files` lists both `wiki/` and `site/content/` copies | 69 tracked files in the brain wiki repo |
| 3 | `wiki_content_write` only writes the file to disk | `src/ops/content.rs:213` returns `WriteResult { bytes_written, path, slug }` — no commit, no index, no validation |
| 4 | The two-step flow is only enforced by documentation | `~/.agents/skills/brain/references/tool-reference.md:127` says "Always pair with `wiki_ingest`" — but this is advisory, not enforced |
| 5 | The tool description itself never mentions commit/index | `src/mcp/tools.rs:218` — "Write content to a page in the wiki tree" |

### Real problem (design-level footgun)

The original symptom was caused by issue #1 (no index update on write). But
**verification surfaced a separate, more serious design problem**: the
two-step `wiki_content_write → wiki_ingest` contract is a footgun that
depends on every caller remembering the second step. Any agent — current or
future, this LLM or another — can write a file and forget to ingest, leaving
it on disk but invisible to search, graph, lint, and the Console.

This is not a hypothetical. The skill's own `anti-patterns.md` already lists
5 examples of agents doing exactly this. Documentation warnings do not
enforce contracts; default behavior does.

### Industry pattern

Every mature write tool the user is familiar with — Obsidian, VS Code,
`git commit`, filesystem `save` — treats "make the change durable and
discoverable" as a single user-facing action. None of them split "save the
file" from "make it searchable" into two steps that the user has to
remember to pair. The split exists in brain-mcp as an implementation
artifact (bulk-ingest of external trees is a real use case), not because
users benefit from the two-step shape.

---

## Design

### Goal
Make `wiki_content_write` a single-call write pipeline that — by default —
writes the file, validates frontmatter, commits to git, updates the tantivy
index, and refreshes the web content. The two-step shape survives as an
explicit opt-out for genuine bulk-write flows.

### Approach

Add two optional parameters to `wiki_content_write`:

| Parameter | Type | Default | Effect |
|-----------|------|---------|--------|
| `commit`  | bool | `true`  | When true, run the full pipeline (validate + git commit + index + web sync). When false, behave exactly as today (file on disk only). |
| `redact`  | bool | `false` | Forwarded to `ops::ingest` when `commit=true`. Runs the redaction pass on the body before validation. Same semantics as `wiki_ingest { redact: true }`. |

The default flow becomes:

```
1. canonicalize URI                         (existing)
2. write file to disk                       (existing ops::content_write)
3. if commit:
   3a. ops::ingest(&[path], redact)         (existing pipeline — validate + git::commit)
   3b. ops::index_after_write(&[path])      (from spec #1 — tantivy index)
   3c. sync_web_content                     (existing — Hugo mirror + notify_refresh)
```

When `commit=false`, behavior is byte-identical to today. This preserves
the bulk-write escape hatch: write N files with `commit=false`, then one
`wiki_ingest { path: "directory" }` call handles the whole tree.

### Why merge into `content_write` and not add a new tool

A new `wiki_save` tool (alternative B) would leave two tools doing
overlapping things, which is its own footgun — agents would have to
remember which to use. Merging the behavior into the existing tool, with
the safe default, means "do the right thing unless you explicitly opt out".
This matches how mature tools (Obsidian, git) handle it.

### Backward compatibility

| Caller pattern | Old behavior | New behavior |
|----------------|--------------|--------------|
| `content_write` alone | file on disk only | **file + commit + index + web sync** (behavior change — but the change is the entire point) |
| `content_write` then `ingest` | file + commit + index | file + commit + index + (ingest sees unchanged commit → idempotent no-op) |
| `content_write(commit=false)` | n/a | file on disk only (matches old `content_write` exactly) |
| `content_write(commit=false)` then `ingest` | n/a | matches old two-step flow exactly |

The only genuine breaking case is a caller that wrote a file via
`content_write` and **intentionally** never ingested it, relying on it
staying uncommitted. Search of the codebase and skill suggests no such
caller exists — the documented contract is always "pair with ingest".

### Key components

1. **`ops::content_write`** (`src/ops/content.rs:213`) — add `commit: bool`
   and `redact: bool` parameters. When `commit=true`, after the existing
   `markdown::write_page`, call `ops::ingest(&[&result.path], redact, ...)`
   and then `ops::index_after_write(&[result.path])`. Extend `WriteResult`
   with `commit_sha: Option<String>` and `index_report: Option<IndexReport>`
   so the handler can surface them.

2. **`handle_content_write`** (`src/mcp/handlers.rs:252`) — read `commit`
   (default true) and `redact` (default false) from args. Remove the
   standalone `sync_web_content` call (it is now part of the commit=true
   path; when `commit=false` there is nothing to sync). Surface
   `commit_sha` and `index_report` in the response JSON.

3. **Tool schema** (`src/mcp/tools.rs:218`) — add `commit` and `redact`
   optional params. Rewrite the description:
   > "Write a page to the wiki tree. By default also validates frontmatter,
   > commits to git, updates the search index, and refreshes the web
   > mirror — i.e. a complete durable write in one call. Set `commit=false`
   > for bulk-write flows that defer ingestion to a later `wiki_ingest` call.
   > Bare slugs are canonicalized from frontmatter type into the Blueprint
   > layout."

4. **Skill brain updates** (local files at `~/.agents/skills/brain/`,
   not version-controlled alongside this repo). Listed here for
   completeness; done as a follow-up after the code ships:
   - `SKILL.md` — collapse the two-step Markdown write flow into a single
     `wiki_content_write` call. Keep `wiki_ingest` documented for bulk /
     external-directory use.
   - `references/tool-reference.md` — update the `wiki_content_write` row
     (remove "Does not validate, index, or commit" caveat; document the new
     defaults).
   - `references/anti-patterns.md` — refresh the 5 examples that show the
     old two-step shape.
   - `references/architecture.md` — update the 2 examples in the write-flow
     section.

5. **Repo docs:**
   - `docs/specifications/tools/content-operations.md` — update the
     behavior description for `wiki_content_write`.
   - `docs/specs/2026-07-25-auto-index-design.md` — add a note in
     "Non-Goals" pointing to this spec as the follow-on that pulls
     `index_after_write` into the default write path.

### Data flow (new, default `commit=true`)

```
client → wiki_content_write(uri, content, commit=true, redact=false)
  → canonicalize URI
  → ops::content_write(file)              ← write to disk
  → ops::ingest(&[path], redact=false)    ← validate + git::commit (auto_commit)
  → ops::index_after_write(&[path])       ← tantivy index (from spec #1)
  → sync_web_content                      ← Hugo mirror + notify_refresh
  → response {
      bytes_written, path, slug, uri,
      commit_sha,                         ← NEW
      index_report                        ← NEW
    }
```

### Data flow (opt-out, `commit=false`)

```
client → wiki_content_write(uri, content, commit=false)
  → canonicalize URI
  → ops::content_write(file)              ← write to disk
  → response { bytes_written, path, slug, uri }
  (later: client calls wiki_ingest { path: "..." } for the bulk commit)
```

---

## Dependency on Spec #1

This spec assumes `ops::index_after_write(wiki_name, changed_paths)` exists
— which is the deliverable of `2026-07-25-auto-index-design.md`.

**Build order:** #1 (auto-index) → #2 (merge).

If #2 were shipped without #1, the merged pipeline would still leave the
search index stale after each write — reproducing the original symptom. The
merge only delivers its promised UX once writes also trigger indexing.

---

## Files Touched

| File | Change | Risk |
|------|--------|------|
| `src/ops/content.rs` (`pub fn content_write`, line 213; `pub struct WriteResult`, line 128) | Add `commit`, `redact` params; call `ops::ingest` + `ops::index_after_write` when `commit=true`; extend `WriteResult` with `commit_sha: Option<String>` and `index_report: Option<IndexReport>` | Medium — core write path |
| `src/mcp/handlers.rs` (`handle_content_write`, line 252) | Read new args; remove standalone `sync_web_content`; surface `commit_sha`, `index_report` in response | Low |
| `src/mcp/tools.rs` (line 218, `wiki_content_write` Tool def) | Add 2 optional params; rewrite description | Low |
| `tests/mcp.rs` | Update tests that assume 2-step write+ingest flow; add test for new single-call pipeline | Low |
| `tests/mcp_auth_boundary_v1.rs` | Update auth-boundary tests if they assert on the response shape of `wiki_content_write` | Low |
| `tests/ops/content.rs` | Update unit tests for `ops::content_write` to cover `commit=true`/`false` paths | Low |
| `docs/specifications/tools/content-operations.md` | Update behavior description | None |
| `docs/specs/2026-07-25-auto-index-design.md` | Add cross-reference note | None |
| `~/.agents/skills/brain/SKILL.md` | Collapse two-step Markdown flow to one call | None (local files; follow-up after code ships) |
| `~/.agents/skills/brain/references/tool-reference.md` | Update `wiki_content_write` row | None (local files; follow-up after code ships) |
| `~/.agents/skills/brain/references/anti-patterns.md` | Refresh 5 examples | None (local files; follow-up after code ships) |
| `~/.agents/skills/brain/references/architecture.md` | Update 2 examples | None (local files; follow-up after code ships) |

### Files NOT touched

- `src/ops/ingest.rs` — reused as-is
- `src/mcp/handlers.rs` (`handle_ingest`) — `wiki_ingest` keeps its current
  contract (bulk, dry_run, redact-only, re-ingest after external edit)
- `src/ops/content.rs` (`content_new`) — out of scope; scaffolded pages
  have no meaningful body to validate/commit until the user writes content
- Semantic claim handlers — separate write path

---

## Testing

### Unit tests
- `ops::content_write(commit=true)` returns a `WriteResult` with
  `commit_sha: Some(_)` and `index_report` showing 1 page updated.
- `ops::content_write(commit=false)` returns `commit_sha: None`,
  `index_report: None`, and the file exists on disk.
- `ops::content_write(commit=true, redact=true)` runs the redaction pass
  (verified by injecting a known secret pattern and checking it is
  replaced in the committed bytes).

### Integration tests
- **Single-call write is searchable immediately:** call
  `wiki_content_write(uri=X, content=Y)` → `wiki_search(Y snippet)` returns
  X without any further call.
- **Backward-compat (idempotent):** call `wiki_content_write(uri=X)`
  followed by `wiki_ingest(path=X)` → second call reports 0 pages changed
  (commit hash unchanged).
- **Bulk opt-out:** call `wiki_content_write(commit=false)` N times, then
  `wiki_ingest(path=directory)` once → all N pages committed and indexed
  in a single git commit.
- **Response shape:** `wiki_content_write(commit=true)` response includes
  `commit_sha` and `index_report`; `commit=false` response matches the
  pre-change shape (plus the new optional fields as null).

### Manual smoke
- Write `thai-stock-factsheet.md` via a single `wiki_content_write` call →
  page appears in `http://localhost:8080/#/search` and `wiki_search`
  without any further action.

---

## Trade-offs and Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Behavior change breaks a caller that intentionally skips ingest | Low | Medium | Codebase + skill search finds no such caller; documented contract is "always pair". The change enforces the documented contract. |
| Existing tests assume 2-step flow | High | Low | Update tests — the new behavior is the intended one |
| Skill brain still documents old flow until updated | High (initially) | Medium | Ship skill update as a follow-up PR immediately after code lands |
| Bulk-write flow slows down if `commit=true` is accidentally used | Low | Medium | Opt-out via `commit=false` is documented; bulk callers already use `wiki_ingest` directly |
| `redact=true` surprises an agent that didn't expect body mutation | Low | Low | Default `false`; documented in tool description |
| Two callers race on the same file | Low | Low | Git commit + tantivy update are both idempotent on unchanged content |

---

## Explicit Non-Goals

- Removing `wiki_ingest` — it remains the right tool for bulk directory
  ingest, `dry_run`, redact-only passes, and re-ingestion after external
  edits.
- Merging `wiki_content_new` + ingest — scaffolded pages have placeholder
  bodies; committing them adds noise. Out of scope.
- Adding lint enforcement for "always pair write with ingest" — the merged
  default makes this unnecessary.
- Refactoring `ops::ingest` internals — reused unchanged.
- Touching the semantic claim write path (`brain_capture` /
  `brain_confirm` / `brain_supersede`) — separate store, separate concern.

---

## Open Questions

None at design time. Implementation plan will resolve the exact shape of
`IndexReport` (depends on spec #1's `index_after_write` signature) during
step 1.
