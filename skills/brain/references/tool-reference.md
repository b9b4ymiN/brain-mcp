---
title: "Brain MCP Tool Reference"
summary: "The 39-tool matrix: args, tier, returns, when-to-use, gotchas, MCP<->CLI mapping."
read_when:
  - Picking the right tool for a job
  - Needing exact required/optional args for a tool call
  - Deciding whether a call is auto-approvable (read-only) or needs confirmation (destructive)
  - Mapping an MCP tool to its CLI equivalent
audience: agent (Claude/ZCode) operating the brain MCP server
canonical_source: "src/mcp/tools.rs::tool_list and src/mcp/tools.rs::annotations_for"
last_verified_against_code: "2026-07-19"
---

# Brain MCP Tool Reference

This is the deep playbook for the 39 tools exposed by the `brain-mcp` server
(crate `llm-wiki-engine`, binary `llm-wiki`). The canonical source of truth is
`src/mcp/tools.rs` in the repo: `tool_list()` defines the schemas,
`annotations_for()` defines the tier of every tool. When this file and the
code disagree, the code wins — re-verify and update this file.

## How to Read This Reference

### Tier model — why it matters

Every tool carries four MCP annotation hints (`readOnlyHint`,
`destructiveHint`, `idempotentHint`, `openWorldHint`). Clients use these to
decide whether to auto-approve a call or prompt the user. The brain-mcp code
collapses them into four working tiers, set in
`src/mcp/tools.rs::annotations_for`:

| Tier | `read_only` | `destructive` | `idempotent` | `open_world` | Client behavior |
|------|-------------|---------------|--------------|--------------|-----------------|
| **1. Read-only** | `true` | `false` | `true` | `false` | Safe to auto-approve; no state mutation. |
| **2. Write-additive** | `false` | `false` | `false` | `false` | Mutates durable state (filesystem, git, ledger, index); not safely retryable. Confirm intent before the user's "yes" is implicit. |
| **3. Write-idempotent** | `false` | `false` | `true` | `false` | Mutates state but is safe to retry with identical args (rebuild, set-default, config set). |
| **4. Destructive** | `false` | `true` | `false` | `false` | Removes data. **ALWAYS confirm with the user before invoking.** |

#### Annotation policy (`annotations_for`)

The function in `src/mcp/tools.rs` (lines 85–123) sets annotations in a
single post-processing pass keyed by tool name, rather than threading a
fourth argument through every `Tool::new`. The default arm is
`write_additive()` — the most conservative non-read-only profile — so any
future tool that is not explicitly listed is never accidentally hinted as
read-only. Two non-obvious classifications to remember:

- `wiki_schema` is multi-action. Its `action: remove` path with
  `delete_pages: true` can delete a schema file **and** every `.md` page of
  that type from disk, so the entire tool is classified **destructive** by
  its most dangerous action — even though `list`/`show`/`validate` are
  benign read paths. Splitting it into separate read/write tools is
  deferred.
- `wiki_spaces_list` is classified **write-additive** in `annotations_for`
  (line 115). This is conservative — the tool itself performs no mutation,
  but it sits in the space-management family, and the conservative default
  prevents a future change to it from being silently auto-approved.
  **Treat `wiki_spaces_list` as read-only in practice** (it lists entries
  from the registry and changes no durable state), but expect the MCP hint
  to advertise it as additive. This is the single discrepancy between
  behavior and annotation; flagged here, not silently "fixed".

### Per-call discipline

1. **Read before write.** Tier-1 reads are free. Always read the current
   state (`wiki_content_read`, `brain_get`, `profile_get`) before any
   write so your proposal is a diff, not a blind overwrite.
2. **Tier-4 always asks.** `wiki_schema` (with `action: remove`) and
   `wiki_spaces_remove` require explicit user confirmation regardless of
  `dry_run`. Explain the blast radius first.
3. **Idempotency keys are required** for the `brain_*` mutation tools.
   Supply a fresh `operation_id` (UUIDv7 or UUIDv4 recommended) on every
   new logical operation. Replaying the same `operation_id` with a
   different payload returns `IDEMPOTENCY_CONFLICT` (see ADR-0001 §2,
   "atomic write topology") with no mutation.

### Conventions

- **Required args** in tables below are the literal `required` array from
  the `schema(...)` call in `tool_list()`. Everything else is optional.
- "Returns" describes the *primary* payload shape; many tools also emit a
  human-readable summary line.
- Example calls use the MCP tool-call shape (`tool_name(arg: "value")`),
  not the CLI shape. CLI equivalents are in §6.

---

## Tier 1: Read-Only Tools (20 tools)

Auto-approvable. No durable state mutation. Safe to retry.

| Tool | Required args | Optional args | Returns | When to use | Example call | Gotcha |
|------|---------------|---------------|---------|-------------|--------------|--------|
| `wiki_search` | `query` | `type`, `no_excerpt`, `include_sections`, `top_k`, `wiki`, `cross_wiki`, `format` (`json`/`llms`) | Ranked `PageRef[]` + facets (`type`, `status`, `tags`) | Keyword discovery over titles, summaries, tags, bodies | `wiki_search(query: "mixture of experts", type: "concept", top_k: 5)` | `final_score = bm25 × status_multiplier × confidence` is applied inside the collector — top-k is the true top-k. The `type` facet is **unfiltered** even when `--type` is active. |
| `wiki_list` | — | `type`, `status`, `page`, `page_size`, `wiki`, `format` | Paginated page list | Browsing by type/status; bulk inventory | `wiki_list(type: "procedure", status: "draft", page_size: 50)` | Pagination is 1-based. `format: "llms"` produces a compact one-line-per-page list for in-context use (not the same as `wiki_export`). |
| `wiki_content_read` | `uri` | `no_frontmatter`, `list_assets`, `backlinks`, `wiki` | Page body as text (default) **or** JSON `{content, backlinks[]}` with `backlinks: true` | Reading a known page or bundle asset | `wiki_content_read(uri: "concepts/moe", backlinks: true)` | With `backlinks: true` the response becomes JSON. Bare slug, short URI, and full `wiki://name/slug` are all accepted. When a page has `superseded_by`, the output includes a notice pointing to the replacement. |
| `wiki_history` | `slug` | `limit`, `follow`, `wiki` | Git commit list (hash, date, message, author) | Freshness audit, blame | `wiki_history(slug: "concepts/moe", limit: 5)` | For bundles/sections, logs the `index.md`. `follow: true` (default) tracks renames across flat→bundle migration. |
| `wiki_stats` | — | `wiki` | Health dashboard: page/type/status counts, orphans, density, staleness, communities, diameter/radius/center | Quick wiki health check | `wiki_stats()` | `communities` is `null` when pages < `graph.min_nodes_for_communities` (default 30). Diameter/radius are skipped when `local_count > graph.max_nodes_for_diameter` (default 2000); see `structural_note`. |
| `wiki_graph` | — | `format` (`mermaid`/`dot`/`llms`), `root`, `depth`, `type`, `relation`, `output`, `cross_wiki`, `wiki` | Graph representation in chosen format | Visual or LLM-readable structure analysis | `wiki_graph(root: "concepts/moe", depth: 2, format: "llms")` | `format: "llms"` returns a natural-language description (clusters, hubs, isolated nodes) — use it when interpreting rather than rendering. Without `cross_wiki`, links to other wikis render as external placeholder nodes. |
| `wiki_resolve` | `uri` | `wiki` | `{slug, wiki, wiki_root, path, exists, bundle}` | Look up the on-disk file path before a direct write | `wiki_resolve(uri: "concepts/moe")` | No CLI equivalent. For a not-yet-existing slug, `exists: false` and `path` is the would-be flat path. |
| `wiki_lint` | — | `rules`, `severity`, `wiki` | `{total, errors, warnings, findings[]}` with absolute `path` per finding | Health audit, CI gate, finding broken links | `wiki_lint(rules: "orphan,broken-link", severity: "error")` | Rules: `orphan`, `broken-link`, `broken-cross-wiki-link`, `missing-fields`, `stale`, `unknown-type`, `articulation-point`, `bridge`, `periphery`. `periphery` is skipped when graph is too large. Each finding's `path` is absolute — use it for direct edits without a follow-up `wiki_resolve`. |
| `wiki_suggest` | `slug` | `limit`, `wiki` | Suggested related pages to link | Fixing orphans, enriching graph | `wiki_suggest(slug: "concepts/moe", limit: 5)` | Cheap graph query; safe to call any time you're about to write a page and want to know what to link to. |
| `profile_get` | — | `section` (`rules`/`identity`/`style`/`stack`/`constraints`), `wiki` | Operator constitution content | Session bootstrap (mandatory first call) | `profile_get(section: "rules")` | Loaded every session. Read **before** any write to know the operator's hard rules. |
| `semantic_search` | `query` | `top_k`, `type` (`concept`/`entity`/`source`/`project`/`decision`), `wiki`, `format` | Ranked pages (Blueprint read-tier alias) | Concept-tier discovery with type filter | `semantic_search(query: "scaling laws", type: "concept")` | Today this is a **BM25 alias** for `wiki_search` with semantic-type filtering; vector/rerank tier is not wired yet. Use it for intent clarity, not for embedding search. |
| `semantic_get` | `page_id` | `with_backlinks`, `wiki` | Page content (semantic-tier read) | Reading a concept/entity/source/project/decision page | `semantic_get(page_id: "concepts/moe", with_backlinks: true)` | Blueprint alias for `wiki_content_read` with semantic-tier framing. Functionally equivalent today. |
| `procedural_find` | `intent` | `context`, `top_k`, `wiki`, `format` | Matching procedure runbooks | Finding a runbook by goal | `procedural_find(intent: "deploy brain-mcp to oracle cloud")` | Filters to `type: procedure`. Use this rather than `wiki_search --type procedure` when you want intent-oriented ranking. |
| `procedural_get` | `proc_id` | `wiki` | Procedure runbook content | Reading a specific procedure | `procedural_get(proc_id: "procedural/deployment/deploy-brain-mcp")` | Procedures require a `verification` block before promotion — read this to know the validation steps. |
| `graph_neighbors` | `page_id` | `depth`, `edge_types`, `wiki` | Pages within N hops of root | Local neighborhood query | `graph_neighbors(page_id: "concepts/moe", depth: 1)` | Use for a single-hop "what's around this?" query. For full subgraph rendering, use `wiki_graph`. |
| `audit_history` | `path` | `limit`, `wiki` | Audit trail for a page | Combining git + ledger audit | `audit_history(path: "concepts/moe", limit: 20)` | Distinct from `wiki_history`: this reads the audit ledger (claim events + git shas). Use when you need the semantic trail, not just the git log. |
| `wiki_index_status` | — | `wiki` | `{stale, built}` | Is the search index fresh? | `wiki_index_status()` | If `stale: true`, search results may be out of date — run `wiki_index_rebuild` (tier 3). |
| `brain_status` | — | — | Ledger head, claim count, schema version | Session bootstrap (mandatory) | `brain_status()` | Schema only — no args. If schema version is degraded, **do not write**; read `references/architecture.md` and escalate. |
| `brain_search` | `query` | `domain`, `top_k` (default 10) | Confirmed claims matching subject/predicate substring | Looking up what the brain "knows" about a topic | `brain_search(query: "tesla", domain: "stocks")` | Searches **confirmed** claims only (`project(replay(ledger)) − purge_registry.denied_ids`). Proposed-but-unconfirmed claims are invisible here. |
| `brain_get` | `subject` | `domain` | Claims for a subject | Looking up everything about a specific subject | `brain_get(subject: "AAPL")` | Returns the claim set for one subject. Use `brain_search` when you don't know the exact subject string. |

---

## Tier 2: Write-Additive Tools (14 tools)

Mutates durable state (filesystem, git, ledger, index, or registry). Not
safely retryable — supply a fresh operation_id where required.

| Tool | Required args | Optional args | Returns | When to use | Example call | Gotcha |
|------|---------------|---------------|---------|-------------|--------------|--------|
| `wiki_spaces_create` | `path`, `name` | `description`, `force`, `set_default`, `wiki_root` | New wiki repo + registry entry | Initializing a new wiki | `wiki_spaces_create(path: "~/wikis/brain", name: "brain", set_default: true)` | Creates the full scaffold (`wiki.toml`, `schemas/`, `inbox/`, `raw/`, `wiki/`) and an initial git commit `create: <name>`. Re-run: same name = silent skip; different name = error (use `force: true`). Hot-mounted if server is running. **On Docker compose (Shape B) with a `:ro`-mounted config, this fails with `Read-only file system (os error 30)` after the scaffold already succeeded** — see `references/deployment-shapes.md` § EROFS for the workaround. |
| `wiki_spaces_register` | `path`, `name` | `description`, `wiki_root` | Registry entry (no files created) | Adopting an existing repo | `wiki_spaces_register(path: "/abs/path", name: "research")` | Reads `wiki.toml` for effective `wiki_root`. If you pass `--wiki-root` and it conflicts with `wiki.toml`, errors — edit `wiki.toml` first. The `wiki_root` directory must already exist. |
| `wiki_spaces_list` | — | `name` | List of registered wikis (path, description, default flag) | Inspecting the registry | `wiki_spaces_list()` | **Behaviorally read-only** but classified write-additive by `annotations_for` (conservative default for the space-management family). With `name`, returns a single-element list (or empty if not found). `*` marks the default in text output. |
| `wiki_content_write` | `uri`, `content` | `wiki` | Confirmation (file written) | Writing a markdown page body | `wiki_content_write(uri: "concepts/moe", content: "---\ntitle: MoE\n---\n...")` | Does **not** validate, index, or commit. Always pair with `wiki_ingest` for the full pipeline. Bare slugs are canonicalized from frontmatter type into the Blueprint layout. |
| `wiki_content_new` | `uri` | `section`, `bundle`, `name`, `type`, `wiki` | `{uri, slug, path, wiki_root, bundle}` | Scaffolding a new page or section | `wiki_content_new(uri: "concepts/moe", name: "Mixture of Experts")` | Returns the absolute `path` — use it for direct file edits before `wiki_ingest`. Missing parent sections are auto-created. `bundle: true` creates `folder/index.md` (pages only); sections are always directories. |
| `wiki_content_commit` | — | `slugs` (comma-separated), `message`, `wiki` | Commit result | Committing pending writes to git | `wiki_content_commit(slugs: "concepts/moe", message: "expand MoE section")` | Over **MCP**, omitting `slugs` commits all pending changes — no error (verified 2026-07-19). The "no slugs + no `--all` = error" rule is **CLI-only** (`llm-wiki content commit` requires explicit slugs or `--all`); the MCP schema has no required args at all. For bundles/sections, the entire folder is staged recursively. Default message: `commit: <slug>, <slug>` or `commit: all`. |
| `wiki_ingest` | `path` | `dry_run`, `redact`, `wiki` | `{pages_validated, unchanged_count, assets_found, warnings, commit, redacted}` | Validate + index + (optionally) commit | `wiki_ingest(path: "concepts/moe", redact: true)` | The full content pipeline. Normal run validates only git-changed files since last indexed commit; `dry_run: true` validates all. `redact: true` is **lossy** — original values are replaced. `commit` is empty when `ingest.auto_commit` is false. |
| `wiki_export` | `wiki` | `path`, `format` (`llms-txt`/`llms-full`/`json`), `status` (`active`/`all`) | `{path, pages_written, bytes, format}` | Publishing to `llms.txt` ecosystem, offline analysis | `wiki_export(wiki: "brain", format: "llms-full")` | Writes a file — does not stream content back. Default path is `<wiki-root>/llms.txt`. `status: "active"` (default) excludes archived. Distinct from `format: "llms"` on `wiki_search`/`wiki_list`/`wiki_graph`. |
| `brain_capture` | `operation_id`, `utterance`, `subject`, `predicate`, `value`, `domain` | `claim_kind` (default `user_assertion`) | Proposed claim with operation id | Capturing a user utterance as a proposed claim | `brain_capture(operation_id: "u7-...", utterance: "Tesla reports Q3 EPS of $0.62", subject: "TSLA", predicate: "eps_q3_2025", value: "0.62", domain: "stocks")` | Captures leave the claim in `proposed`. Follow with `brain_confirm` to promote. Replay with same `operation_id` + same payload returns the stored outcome; same `operation_id` + different payload returns `IDEMPOTENCY_CONFLICT` (ADR-0001 §2). |
| `brain_confirm` | `operation_id`, `proposal_operation_id` | — | Confirmed claim | Promoting a proposal to confirmed | `brain_confirm(operation_id: "u7-confirm-...", proposal_operation_id: "u7-capture-...")` | Requires a valid proposal op id from `brain_capture` or `brain_propose`. The `operation_id` here is the **confirm** operation's id (fresh), not the proposal's. |
| `brain_supersede` | `operation_id`, `proposal_operation_id`, `superseded_claim_operation_ids` | — | Superseded claim set | Replacing stale confirmed claim(s) with a new one | `brain_supersede(operation_id: "u7-super-...", proposal_operation_id: "u7-prop-...", superseded_claim_operation_ids: "u7-confirm-...,u7-confirm-...")` | `superseded_claim_operation_ids` is a **comma-separated string** of confirm op ids. The new claim must already be proposed (via `brain_propose`). |
| `brain_propose` | `operation_id`, `subject`, `predicate`, `value`, `domain`, `method` | `model`, `prompt_version`, `claim_kind` (default `inference`), `evidence_capture_operation_ids` | Proposed claim (status `proposed`) | Recording an AI-derived inference | `brain_propose(operation_id: "u7-prop-...", subject: "TSLA", predicate: "sentiment", value: "bullish", domain: "stocks", method: "llm_extraction", evidence_capture_operation_ids: "u7-c1,u7-c2")` | Always `proposed`, never auto-confirmed — even with evidence. Omit `evidence_capture_operation_ids` for an unsupported/no-evidence proposal (which the extraction policy will refuse to yield as actionable). The `ExtractionPolicy` rejects supported proposals lacking evidence (prompt-injection resistance). |
| `brain_ingest_source` | `operation_id` | `text`, `url`, `file_path`, `max_chunk_bytes` (default 4000), `wiki` | Quarantined chunk captures (one per paragraph-packed chunk) | Staging an external source for extraction | `brain_ingest_source(operation_id: "u7-src-...", url: "https://example.com/article", max_chunk_bytes: 4000)` | Exactly one of `text`/`url`/`file_path` required. URLs are SSRF-guarded (http/https only). Each chunk gets `{operation_id}-chunk-N`. File paths resolve relative to wiki root or absolute inside it. |
| `brain_extract` | `capture_operation_id`, `method` | `model`, `prompt_version`, `local_only` | Proposed claims derived from the chunk | Running AI extraction over a quarantined chunk | `brain_extract(capture_operation_id: "u7-src-...-chunk-0", method: "llm_extraction")` | Validates evidence spans against actual rendition bytes (sha256 proof). `local_only: true` denies the provider call outright — useful when policy forbids egress. Errors: `MalformedEvidenceSpan`, `QuoteHashMismatch`, `SchemaInvalid`, `UnsupportedWithoutEvidence` (see `src/extraction.rs::ProposeError`). |

---

## Tier 3: Write-Idempotent Tools (3 tools)

Mutates state but safe to retry with identical args — calling twice with
the same input produces the same end state as calling once.

| Tool | Required args | Optional args | Returns | When to use | Example call | Gotcha |
|------|---------------|---------------|---------|-------------|--------------|--------|
| `wiki_index_rebuild` | — | `wiki` | Rebuilt tantivy index report | After deletes, after schema changes, when `wiki_index_status` reports stale | `wiki_index_rebuild(wiki: "brain")` | Validates nothing — pure index rebuild from committed files. Contrast `wiki_ingest --dry-run` which validates everything. |
| `wiki_config` | `action` (`get`/`set`/`list`) | `key`, `value`, `global`, `wiki` | Config value (get), confirmation (set), full resolved config (list) | Tuning `defaults.*`, `ingest.*`, `validation.*` | `wiki_config(action: "set", key: "defaults.search_top_k", value: "15", global: true)` | `set` without `global` writes to per-wiki `wiki.toml` of the default wiki (or `--wiki`). Global-only keys (`index.*`, `serve.*`, `logging.*`) reject `--wiki`. Idempotent: setting the same key/value twice yields the same file. |
| `wiki_spaces_set_default` | `name` | — | Confirmation | Switching the active default wiki | `wiki_spaces_set_default(name: "brain")` | Alias for `wiki_config set global.default_wiki <name>`. Hot-effective when server is running. |

### Why "idempotent" matters here

- A retry after a transient failure (network blip, timeout) is safe — no
  duplicate side effects, no duplicated commits, no doubled ledger entries.
- For `wiki_config set`, setting `key=X value=Y` twice leaves the file in
  the same state as setting it once.
- For `wiki_index_rebuild`, the resulting index is deterministic given the
  same committed files.
- For `wiki_spaces_set_default`, the registry's `default_wiki` field ends
  up at the requested value.

This is **not** the same as "no effect" — the tools do mutate state, they
just don't compound.

---

## Tier 4: Destructive Tools (2 tools)

**ALWAYS confirm with the user before invoking.** Explain the blast radius
first. These tools remove data; the MCP `destructiveHint: true` is set so
clients must not auto-approve.

| Tool | Required args | Optional args | Returns | When to use | Example call | Gotcha |
|------|---------------|---------------|---------|-------------|--------------|--------|
| `wiki_spaces_remove` | `name` | `delete` | Removal confirmation | Decommissioning a wiki | `wiki_spaces_remove(name: "old-experiment")` | **Refuses if the wiki is the current default** — set a new default first. With `delete: true` the entire wiki directory is deleted from disk (irreversible). Without it, only the registry entry is removed (files stay). Server hot-unmounts immediately; in-flight requests complete normally. |
| `wiki_schema` | `action` (`list`/`show`/`add`/`remove`/`validate`) | `type`, `template`, `schema_path`, `delete`, `delete_pages`, `dry_run`, `wiki` | Varies by action | Inspecting or managing type schemas | (read paths) `wiki_schema(action: "list")` / (write paths) `wiki_schema(action: "remove", type: "obsolete-type", delete_pages: true, dry_run: true)` | **Whole tool is classified destructive because of `action: remove` with `delete_pages: true`** — that path deletes the schema file **and** every `.md` page of that type from disk. The benign read paths (`list`, `show`, `validate`) inherit the destructive hint as a worst-case-safe measure. **Always pass `dry_run: true` first** when removing, and confirm the count of affected pages with the user. Cannot remove the `default` type. |

### Blast radius detail for `wiki_schema action: remove`

1. Counts pages of the target type in the index.
2. If `dry_run: true` → reports counts and stops. **Always do this first.**
3. Removes pages of the type from the tantivy index.
4. If `delete_pages: true` → **deletes the `.md` files from disk**.
5. If `[types.<type>]` exists in `wiki.toml` → removes the entry.
6. If `delete: true` → modifies or deletes the schema file itself.

Steps 4 and 6 are irreversible. There is no undo. The git history may
still contain the file content, but `wiki_ingest` will not see it again
until the type is re-registered and the files restored.

---

## When to Use X Over Y

Concrete decision criteria for the common cross-tool choices.

### `semantic_search` vs `wiki_search` vs `brain_search`

| Use | Tool | Why |
|-----|------|-----|
| Keyword discovery across all page types | `wiki_search` | BM25 across titles, summaries, tags, bodies; facets on type/status/tags. The default workhorse. |
| Discovery scoped to semantic types (concept/entity/source/project/decision) | `semantic_search` | Today a BM25 alias with semantic-type filter; use for intent clarity. Will become the hybrid vector+BM25 entry point when the vector tier is wired. |
| Looking up **confirmed claims** about a subject | `brain_search` | Queries the event-ledger-backed semantic brain, not the markdown index. Use when you want epistemic facts (`TSLA.eps_q3 = 0.62`), not document text. |

Rule of thumb: documents → `wiki_search`; concepts → `semantic_search`;
facts/claims → `brain_search`.

### `semantic_get` vs `wiki_content_read` vs `brain_get`

| Use | Tool | Why |
|-----|------|-----|
| Read the markdown body of a known page | `wiki_content_read` | The general-purpose reader. Supports `backlinks`, `no_frontmatter`, `list_assets`. |
| Read a concept/entity/source/project/decision page (Blueprint tier) | `semantic_get` | Functionally equivalent today; signals intent in the call site and aligns with the Blueprint read-tier naming. |
| Read all claims for one subject | `brain_get` | Returns the ledger-derived claim set, not page text. |

### `procedural_find` vs `wiki_search --type procedure`

| Use | Tool | Why |
|-----|------|-----|
| Find a runbook by **goal** ("deploy X", "recover from Y") | `procedural_find` | Intent-oriented; takes an `intent` string and optional `context`. Filters to procedures implicitly. |
| Find a runbook by **keyword** that may also appear in non-procedure pages | `wiki_search --type procedure` | Use when you want BM25 ranking over procedure bodies and titles, with facets. |

Rule: if the user said "how do I…" use `procedural_find`; if they quoted a
specific term, use `wiki_search`.

### `graph_neighbors` vs `wiki_graph`

| Use | Tool | Why |
|-----|------|-----|
| Single-hop "what's around this page?" | `graph_neighbors` | Cheap, focused, returns neighbor pages. |
| Full subgraph or whole-wiki graph for rendering/analysis | `wiki_graph` | Supports `format` (mermaid/dot/llms), `root`, `depth`, `relation`, `cross_wiki`. |

### `audit_history` vs `wiki_history`

| Use | Tool | Why |
|-----|------|-----|
| Git log of a page (commits, dates, authors) | `wiki_history` | Pure VCS view. |
| Combined git + ledger audit trail (includes semantic events) | `audit_history` | Use when the page has associated claim events and you need the full trail. |

### `brain_capture` vs `brain_propose`

| Use | Tool | Why |
|-----|------|-----|
| The user said something worth remembering verbatim | `brain_capture` | Takes the exact `utterance` text; default `claim_kind: user_assertion`. Captures the provenance (who said it). |
| The agent derived a claim by inference (LLM extraction, deduction) | `brain_propose` | Always `proposed`, never auto-confirmed. Requires `method` (e.g. `llm_extraction`). Takes optional `evidence_capture_operation_ids`. |

Both leave the claim in `proposed`; both need `brain_confirm` to promote.
The distinction is provenance: capture = user utterance, propose =
AI-derived inference.

### `wiki_content_write` + `wiki_ingest` vs `wiki_content_commit`

| Use | Tool(s) | Why |
|-----|---------|-----|
| Create/update a markdown page with full validation + indexing | `wiki_content_write` → `wiki_ingest` | The standard pipeline. `wiki_ingest` validates frontmatter against the type schema, updates the search index, and commits when `ingest.auto_commit` is true. |
| Stage multiple writes, then commit atomically | `wiki_content_write` (×N) → `wiki_content_commit` | Use when you want one commit for several pages, or when `auto_commit` is false. |
| Commit only, skipping re-validation | `wiki_content_commit` | Use when files were edited directly on disk and already-ingested; just commit the changes. |

### `wiki_index_rebuild` vs `wiki_ingest`

| Use | Tool | Why |
|-----|------|-----|
| Index is stale or corrupted; rebuild from committed files | `wiki_index_rebuild` | Validates **nothing** — pure rebuild. |
| Validate changed files, update index, optionally commit | `wiki_ingest` | The standard post-write pipeline. `dry_run: true` for a full validation audit. |

Rule: rebuild = repair; ingest = normal flow.

---

## MCP ↔ CLI Mapping

All 39 tools. The CLI lives in the `llm-wiki` binary. Where no CLI
equivalent exists, it is stated explicitly.

| MCP tool | CLI equivalent | Notes |
|----------|----------------|-------|
| `wiki_spaces_create` | `llm-wiki spaces create <path> --name <n> [--description --force --set-default --wiki-root]` | Hot-mounted when server is running. |
| `wiki_spaces_register` | `llm-wiki spaces register <path> --name <n> [--description --wiki-root]` | For adopting existing repos. |
| `wiki_spaces_list` | `llm-wiki spaces list [<name>] [--format text\|json]` | Behaviorally read-only (see annotation note). |
| `wiki_spaces_remove` | `llm-wiki spaces remove <name> [--delete]` | **Destructive** — confirm first. |
| `wiki_spaces_set_default` | `llm-wiki spaces set-default <name>` | Alias for `config set global.default_wiki`. |
| `wiki_config` | `llm-wiki config get <key>` / `config set <key> <value> [--global --wiki]` / `config list [--global --wiki --format]` | Global-only keys reject `--wiki`. |
| `wiki_content_read` | `llm-wiki content read <slug\|uri> [--no-frontmatter --list-assets --backlinks --format --wiki]` | |
| `wiki_content_write` | `llm-wiki content write <slug\|uri> [--file <src>] [--wiki]` | CLI reads content from stdin by default. |
| `wiki_content_new` | `llm-wiki content new <slug\|uri> [--section --bundle --name --type --dry-run --wiki]` | |
| `wiki_content_commit` | `llm-wiki content commit [<slug>...] --all [-m <msg>] [--wiki]` | No slugs + no `--all` = error. |
| `wiki_search` | `llm-wiki search "<q>" [--type --no-excerpt --top-k --include-sections --all --format --wiki]` | `--all` = `cross_wiki: true`. |
| `wiki_list` | `llm-wiki list [--type --status --page --page-size --format --wiki]` | (CLI subcommand may also be invoked as `llm-wiki pages list` depending on version.) |
| `wiki_ingest` | `llm-wiki ingest <slug\|uri> [--dry-run --redact --format --wiki]` | |
| `wiki_index_rebuild` | `llm-wiki index rebuild [--wiki]` | Idempotent. |
| `wiki_index_status` | `llm-wiki index status [--wiki]` | |
| `wiki_graph` | `llm-wiki graph [--format --root --depth --type --relation --output --cross-wiki --wiki]` | |
| `wiki_history` | `llm-wiki history <slug\|uri> [--limit --no-follow --format --wiki]` | |
| `wiki_stats` | `llm-wiki stats [--wiki --format]` | |
| `wiki_suggest` | `llm-wiki suggest <slug\|uri> [--limit --wiki]` | |
| `wiki_lint` | `llm-wiki lint [--rules --severity --format --wiki]` | CLI exits non-zero on `error` findings — usable as a CI gate. |
| `wiki_resolve` | **No CLI equivalent.** | MCP only. |
| `wiki_schema` | `llm-wiki schema list\|show\|add\|remove\|validate ...` | **Destructive hint** because of `remove --delete-pages`. |
| `wiki_export` | `llm-wiki export [--path --format --status --wiki]` | |
| `profile_get` | **No CLI equivalent.** | MCP only (Blueprint read tier). |
| `semantic_search` | **No CLI equivalent.** | MCP only (Blueprint alias). |
| `semantic_get` | **No CLI equivalent.** | MCP only (Blueprint alias). |
| `procedural_find` | **No CLI equivalent.** | MCP only (Blueprint alias). |
| `procedural_get` | **No CLI equivalent.** | MCP only (Blueprint alias). |
| `graph_neighbors` | **No CLI equivalent.** | MCP only (Blueprint alias). |
| `audit_history` | **No CLI equivalent.** | MCP only (ledger-backed audit). |
| `brain_status` | **No first-class CLI equivalent.** | Use the HTTP console or MCP. The CLI's `llm-wiki status` (if present) reports server/process status, not ledger head. |
| `brain_search` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_get` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_capture` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_confirm` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_supersede` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_propose` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_ingest_source` | **No CLI equivalent.** | MCP only (semantic brain). |
| `brain_extract` | **No CLI equivalent.** | MCP only (semantic brain). |

### CLI-only commands (no MCP equivalent)

These exist only in the `llm-wiki` CLI:

| CLI command | Purpose |
|-------------|---------|
| `llm-wiki serve [--http :PORT] [--watch --web --web-port --web-bind]` | Run the MCP server (stdio or HTTP). |
| `llm-wiki web install --wiki <n> [--force]` | Build / rebuild the Hugo web UI mirror. |
| `llm-wiki logs tail [--lines N]` / `list` / `clear` | Log management. |
| `llm-wiki watch [--wiki <n>]` | Filesystem watcher (standalone auto-ingest). |

---

## Common Argument Patterns

Quick reference for arguments that appear across many tools.

### Slug vs `wiki://` URI

Most content tools accept both:

- Bare slug: `concepts/moe`
- Short URI: `wiki://concepts/moe` (default wiki)
- Full URI: `wiki://research/concepts/moe` (explicit wiki)

When a `wiki://` URI is supplied, the `--wiki`/`wiki` argument is **ignored**
— the wiki is taken from the URI. This is consistent across
`wiki_content_read`, `wiki_content_write`, `wiki_content_new`,
`wiki_content_commit`, `wiki_history`, `wiki_resolve`, `wiki_suggest`,
`wiki_ingest`.

For bundles and assets: `wiki://research/concepts/moe` resolves to
`index.md`; `wiki://research/concepts/moe/diagram.png` resolves to the
co-located asset.

### `--wiki` / `wiki` flag

Targets a specific wiki. Defaults to `global.default_wiki`. Required for
`wiki_export` (the tool has `wiki` in its required array).

### `dry_run`

- `wiki_ingest` with `dry_run: true` → validates **all** files (full audit),
  not just git-changed ones.
- `wiki_content_new` with `dry_run: true` (CLI only) → shows what would be
  created.
- `wiki_schema action: remove` with `dry_run: true` → reports counts of
  pages that would be removed/files that would be deleted, then stops.
  **Always pass this first when removing.**

### `redact`

`wiki_ingest` only. When `true`, runs a redaction pass over file bodies
before validation. **Lossy** — original values are replaced (e.g. GitHub
PATs, API keys matched by configured patterns). The response includes a
`redacted` array listing `{slug, matches: [{pattern_name, line_number}]}`.
Use it whenever the body may contain secrets.

### `format`

Three families:

| Format family | Tools | Values |
|---------------|-------|--------|
| Search/list/graph LLM-friendly | `wiki_search`, `wiki_list`, `wiki_graph`, `semantic_search`, `procedural_find` | `json` (default for some) \| `llms` (compact, in-context) |
| Tool response text/json | `wiki_content_read`, `wiki_ingest`, `wiki_history`, `wiki_stats`, `wiki_lint`, `wiki_config`, `wiki_spaces_list` | `text` \| `json` |
| Graph render | `wiki_graph` | `mermaid` (default) \| `dot` \| `llms` |
| Export | `wiki_export` | `llms-txt` (default) \| `llms-full` \| `json` |

Use `format: "llms"` on search/list/graph when the goal is interpretation;
use `format: "json"` when downstream code parses the response; use
`format: "mermaid"`/`"dot"` when rendering.

### `top_k`

Caps result count. Defaults come from config (`defaults.search_top_k`).
Common on `wiki_search`, `semantic_search`, `procedural_find`,
`brain_search` (default 10), `wiki_suggest`, `wiki_history` (as `limit`),
`graph_neighbors` (as `depth`, sort of), `audit_history` (as `limit`).

### `domain` filter (brain_* only)

`brain_search`, `brain_get` accept an optional `domain` (e.g. `"stocks"`,
`"projects"`). Use it to scope claims to a knowledge vertical.
`brain_capture` and `brain_propose` take `domain` as a **required** string
— every claim is tagged with its domain at write time.

### `operation_id` (brain_* only)

Required on `brain_capture`, `brain_confirm`, `brain_supersede`,
`brain_propose`, `brain_ingest_source`, `brain_extract` (the latter takes
`capture_operation_id` instead). Client-supplied opaque 1–128 byte UTF-8
identifier; UUIDv7 or UUIDv4 recommended. Combined with `client_id` and
`owner_id`, forms the idempotency key (ADR-0001 §3).

---

## Error Codes and Recovery

Common error patterns and how to recover. Sources: `src/mcp/tools.rs`
(dispatcher), `src/extraction.rs::ProposeError`, `src/provider.rs::ProviderError`,
`src/api.rs::map_semantic_error`, and ADR-0001.

### `unknown tool: <name>`

- **Cause:** the dispatcher in `tools.rs::call` hit its fallback arm
  (line 708).
- **Recovery:** verify the tool name against `tool_list()` — the name must
  match exactly (case-sensitive). If the tool is new in a recent version,
  the running server may be older; restart `llm-wiki serve`.

### Schema validation failure on ingest

- **Cause:** `wiki_ingest` found frontmatter that does not satisfy the
  page's type schema. Reported in the `warnings` array of the response.
- **Recovery:** run `wiki_schema action: show` with `template: true` for
  the target type to see required fields. Fix the frontmatter, then re-run
  `wiki_ingest`. Use `dry_run: true` first to validate without committing.

### `ProposeError` variants (from `brain_extract` / extraction policy)

| Variant | Meaning | Recovery |
|---------|---------|----------|
| `SupportedWithoutEvidence` | Proposal claims to be supported but has no evidence spans. | Add `evidence_capture_operation_ids` or mark the proposal unsupported. |
| `UnsupportedWithoutEvidence` | Unsupported proposal with no evidence; the extraction policy refuses to yield it as actionable. | This is by design (prompt-injection resistance). If the claim is genuinely unsupported, capture it as a user assertion via `brain_capture` instead. |
| `MalformedEvidenceSpan` | An evidence span has `byte_end <= byte_start`. | Fix the span range in the proposal. |
| `QuoteHashMismatch` | `sha256(rendition_bytes[byte_start..byte_end])` does not match the span's `quote_hash`. | The span does not mechanically prove what it claims to quote. Recompute the hash against actual rendition bytes (the chunk from `brain_ingest_source`). |
| `SchemaInvalid(String)` | Schema validation of the proposal value failed. | Inspect the embedded message; correct the value shape and retry. |

### `ProviderError` variants (from `brain_extract` with provider egress)

| Variant | Retryable? | Recovery |
|---------|------------|----------|
| `Timeout` | yes | Back off and retry. |
| `RateLimited` | yes | Back off and retry (exponential). |
| `ServerError(u16)` | yes | Retry on 5xx. |
| `PartialStream` | yes | Retry. |
| `Outage` | yes | Retry; check provider status. |
| `InvalidJson(String)` | **no** | Same input will fail again — fix prompt/output handling. |
| `QuotaExhausted` | **no** | Top up quota. |
| `Disabled` | **no** | Kill switch engaged — operator must re-enable. |

Use `brain_extract` with `local_only: true` to deny the provider call
outright when policy forbids egress.

### `IDEMPOTENCY_CONFLICT` (ADR-0001 §2)

- **Cause:** the same `(owner_id, client_id, operation_id)` was replayed
  with a **different** tool or payload. The server returns the conflict
  without mutating state.
- **Recovery:** generate a fresh `operation_id` for the new logical
  operation. If you intended to **replay** (e.g. after a network blip),
  reuse the same `operation_id` **and** the same payload — the server
  returns the stored outcome byte-identically.
- **HTTP surface:** mapped to `409 Conflict` with code `conflict` (see
  `src/api.rs::map_semantic_error`). Other SemanticError variants:
  `MissingDependency`/`ObjectUnavailable` → `404 not_found`;
  `InvalidClaim`/`InvalidCapture`/`InvalidInterval` → `400 invalid_request`;
  `UnsupportedInference` → `422 unsupported_inference`;
  `CapabilityDenied`/`Denied` → `403 forbidden`;
  `Disabled` → `503 unavailable`;
  `CorruptLedger`/`Io`/`Database`/`DatabaseContention`/`Serialization` →
  `500 internal_error` (infrastructural, detail never surfaced).

### Git lock contention

- **Symptom:** `wiki_content_commit` or `wiki_ingest` (with `auto_commit`)
  fails with a git index lock error. Usually another git operation is in
  flight (concurrent ingest, external git client, crashed process).
- **Recovery:** wait a moment and retry. If a stale `.git/index.lock`
  remains after a confirmed crash, it can be removed manually — but only
  after verifying no git process is running. The brain-mcp server itself
  is the only writer in normal operation.

### Stale search index

- **Symptom:** `wiki_search`/`wiki_list` returns results that don't match
  recently written pages; `wiki_index_status` reports `stale: true`.
- **Recovery:** `wiki_index_rebuild` (tier 3, idempotent). On a healthy
  wiki, `wiki_ingest` keeps the index in sync automatically — staleness
  usually means an ingest was skipped or interrupted.

### Capability denied / forbidden

- **Symptom:** `CapabilityDenied` from `brain_*` write paths.
- **Recovery:** the calling client lacks the required capability (e.g. a
  propose-only worker tried `purge_execute`). Re-issue with a client that
  has the right capability, or escalate to the operator.

---

## Annotations Reference

The four MCP annotation hints and how brain-mcp sets them per tool. All
definitions live in `src/mcp/tools.rs` (lines 50–123).

### The four hints

| Hint | Meaning | Brain-mcp usage |
|------|---------|-----------------|
| `readOnlyHint` | True if the tool does not modify its environment. | True **only** for the 20 Tier-1 tools. False everywhere else. |
| `destructiveHint` | True if the tool may perform destructive (irreversible) operations. | True **only** for `wiki_spaces_remove` and `wiki_schema`. False everywhere else. |
| `idempotentHint` | True if repeating the same call has the same effect as a single call. | True for Tier-1 (read-only tools are trivially idempotent) **and** Tier-3 (`wiki_index_rebuild`, `wiki_config`, `wiki_spaces_set_default`). False for Tier-2 and Tier-4. |
| `openWorldHint` | True if the tool may interact with entities outside its local environment (network, external APIs). | **False for all brain-mcp tools.** Even `brain_extract` (which may call an AI provider) is classified `open_world: false` at the MCP layer — the outbound egress is policy-gated inside the engine, not advertised as an open-world MCP call. |

### Profile constructors

```rust
fn read_only()        -> { read_only: true,  destructive: false, idempotent: true,  open_world: false }
fn write_additive()   -> { read_only: false, destructive: false, idempotent: false, open_world: false }
fn write_idempotent() -> { read_only: false, destructive: false, idempotent: true,  open_world: false }
fn write_destructive()-> { read_only: false, destructive: true,  idempotent: false, open_world: false }
```

### Per-tool assignment (`annotations_for`)

- **read_only():** `wiki_search`, `wiki_list`, `wiki_content_read`,
  `wiki_history`, `wiki_stats`, `wiki_graph`, `wiki_resolve`, `wiki_lint`,
  `wiki_suggest`, `profile_get`, `semantic_search`, `semantic_get`,
  `procedural_find`, `procedural_get`, `graph_neighbors`, `audit_history`,
  `wiki_index_status`, `brain_status`, `brain_search`, `brain_get`.
- **write_additive():** `brain_capture`, `brain_confirm`,
  `brain_supersede`, `brain_propose`, `brain_ingest_source`,
  `brain_extract`, `wiki_spaces_create`, `wiki_spaces_register`,
  `wiki_spaces_list`, `wiki_content_write`, `wiki_content_new`,
  `wiki_content_commit`, `wiki_ingest`, `wiki_export`.
  **Plus the default arm** (`_ => write_additive()`): any future tool not
  explicitly listed inherits additive — never accidentally read-only.
- **write_idempotent():** `wiki_index_rebuild`, `wiki_config`,
  `wiki_spaces_set_default`.
- **write_destructive():** `wiki_spaces_remove`, `wiki_schema`.

### Annotation attachment mechanism

Annotations are attached in a **single post-processing pass** after
`tool_list()` constructs the raw tools, not threaded through every
`Tool::new` call. This keeps the 39 declarations readable (3-arg
`Tool::new`) and centralizes policy in one match expression. The mechanism
(lines 653–659):

```rust
tools
    .into_iter()
    .map(|tool| {
        let name = tool.name.clone();
        tool.with_annotations(annotations_for(&name))
    })
    .collect()
```

### Why this matters for clients

A client (Claude Desktop, Cursor, Codex, etc.) reads these hints to decide
whether to auto-approve a tool call or prompt the user. The brain-mcp
policy is:

- **Auto-approve**: Tier-1 (read-only) — no risk.
- **Confirm-or-auto-approve with intent**: Tier-2 (additive) — depends on
  client policy. The skill's "no silent writes" rule means the agent
  should still follow propose → confirm for durable memory even if the
  client would auto-approve.
- **Safe to retry**: Tier-3 (idempotent) — clients may retry on transient
  failure without prompting again.
- **Always prompt**: Tier-4 (destructive) — clients must not auto-approve.
  The agent must additionally explain the blast radius and get explicit
  user consent before invoking.

---

## Quick Decision Cheat Sheet

| You want to… | Use |
|--------------|-----|
| Bootstrap a session | `wiki_spaces_list` → `profile_get` → `brain_status` → `wiki_index_status` |
| Discover pages by keyword | `wiki_search` |
| Discover concepts by type | `semantic_search` |
| Read a specific page | `wiki_content_read` (or `semantic_get` for Blueprint framing) |
| Read claims about a subject | `brain_get` (one subject) or `brain_search` (substring) |
| Read the operator constitution | `profile_get` |
| Write a markdown page | `wiki_content_write` → `wiki_ingest` |
| Scaffold a new page | `wiki_content_new` → write body → `wiki_ingest` |
| Commit pending writes | `wiki_content_commit` |
| Capture a user utterance as a claim | `brain_capture` → `brain_confirm` |
| Record an AI-derived inference | `brain_propose` → `brain_confirm` |
| Replace a stale claim | `brain_propose` (new) → `brain_supersede` |
| Ingest an external source | `brain_ingest_source` → `brain_extract` |
| Find a procedure runbook | `procedural_find` |
| Inspect graph neighbors | `graph_neighbors` |
| Render a full subgraph | `wiki_graph` |
| Audit a page (git + ledger) | `audit_history`; pure git → `wiki_history` |
| Check wiki health | `wiki_stats`, `wiki_lint` |
| Check index freshness | `wiki_index_status`; rebuild → `wiki_index_rebuild` |
| Check brain health | `brain_status` |
| Tune config | `wiki_config` |
| Export wiki for publishing | `wiki_export` |
| Inspect type schemas | `wiki_schema action: list/show` |
| Validate a type schema | `wiki_schema action: validate` |
| **Remove a wiki** (destructive) | `wiki_spaces_remove` — **confirm first** |
| **Remove a type** (destructive) | `wiki_schema action: remove` with `dry_run: true` first — **confirm first** |

---

*Verified against `src/mcp/tools.rs` (`tool_list` and `annotations_for`)
on 2026-07-19. Tier counts: 20 read-only, 14 write-additive, 3
write-idempotent, 2 destructive — total 39 tools. The brief said "~40";
the actual count is 39. Flagged discrepancy: `wiki_spaces_list` is
behaviorally read-only but classified write-additive by `annotations_for`
(conservative default for the space-management family).*
