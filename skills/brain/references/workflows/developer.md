# Developer Playbook

Audience: a **contributor** to brain-mcp source — Rust developer, sometimes
Python integration tester, working with CodeGraph to navigate a 265-file
Rust + Python + TypeScript codebase. You add tools, fix bugs, run tests,
and ship changes through the `vnext/phase-0` branch.

This playbook is the deep reference for working inside the engine. It
covers the codebase map, CodeGraph navigation, the canonical
"add-a-tool" procedure, the test matrix, schemas, engine internals,
common failures, and the contribution workflow. For the per-tool
argument matrix read `references/tool-reference.md` first; for the
three-stores epistemic model read `references/architecture.md` first.
When this playbook and the repo disagree, the repo wins — re-verify
against `src/` and update this file.

The crate is `llm-wiki-engine` (binary `llm-wiki`), currently at
`version = "0.4.15"` in `Cargo.toml`. The active development branch is
`vnext/phase-0`. Recent commit shape: `feat(phase-f): F3.4 — ...` and
`fix(phase-f): F3.2 review — ...`.

Sections: [Codebase Map](#codebase-map) · [CodeGraph](#navigation-with-codegraph) · [Add a Tool](#adding-a-new-mcp-tool--end-to-end) · [Tests](#running-tests) · [Schemas](#working-with-schemas) · [Engine Internals](#engine-internals-quick-reference) · [Debugging](#debugging-common-failures) · [Contributing](#contributing-workflow) · [Performance](#performance-and-profiling) · [Further Reading](#further-reading).

---

## Codebase Map

The repo at `C:/Programing/AI2.0/jarvis/brain-mcp-vnext` is indexed by
CodeGraph: **265 files, 4 774 nodes, 12 742 edges** (152 Rust, 46
TypeScript, 44 Python, 13 Svelte, 7 YAML, 3 JavaScript). The engine is
Rust; integration tests are Python (pytest + uv); the web console is
Svelte + TypeScript.

### `src/` top-level modules

| File | Purpose |
|---|---|
| `main.rs` | Binary entry point; dispatches CLI subcommands. |
| `lib.rs` | Crate root; re-exports public modules. |
| `cli.rs` | Clap CLI definition; subcommands mirror most MCP tools. |
| `engine.rs` | `EngineState` + `SpaceContext` — orchestrates spaces, indexes, graph; the central read-lock surface for handlers. |
| `git.rs` | libgit2 wrapper (`commit`, `commit_paths`, `init_repo`); retry/backoff against index lock (MAX_RETRIES=3, BACKOFF_MS=[100,200,400]). |
| `graph.rs` | Petgraph `WikiGraph` + `WikiGraphCache`; concept graph build + community detection + topology metrics. |
| `ingest.rs` | Validate + index + commit pipeline; `IngestOptions`, redaction hook, line-ending normalization. |
| `index_manager.rs` | `SpaceIndexManager` — Tantivy index lifecycle, incremental + full rebuild, staleness tracking. |
| `index_schema.rs` | `IndexSchema` — Tantivy schema computed from the type registry (not hardcoded). |
| `links.rs` | `ParsedLink` — `[[wikilink]]` + `[text](slug)` body link walker and `wiki://` URI resolution. |
| `frontmatter.rs` | YAML frontmatter parse into `BTreeMap` (untyped; the type registry validates). |
| `markdown.rs` | Comrak rendering helpers. |
| `slug.rs` | `Slug` type; canonicalization and bare-slug → Blueprint layout rehoming. |
| `type_registry.rs` | `SpaceTypeRegistry` — discovers types from `schemas/*.json` via `x-wiki-types`; per-type validators and aliases. |
| `config.rs` | Global + per-wiki TOML config; `ResolvedConfig` merge. |
| `spaces.rs` | Space registry: create / register / list / remove / set-default. |
| `space_builder.rs` | Constructs a `SpaceContext` from a repo path (mounts index + graph + registry). |
| `search.rs` | BM25 query collector + facets + status/confidence multipliers. |
| `default_schemas.rs` | Bundled default schemas (profile, concept, semantic, procedure, paper, skill, doc, section). |
| `projection.rs` | Derived projection plumbing (rebuildable from canonical layers). |
| `console.rs` | HTTP console surface (read-only ops dashboard). |
| `consolidation.rs` | Consolidation dry-run contract (BLUEPRINT §11 drill target). |
| `evolution.rs` | Schema evolution / migration contract. |
| `deployment.rs` | Deployment metadata + environment probe. |
| `galaxy.rs` | Galaxy graph renderer for the web console (3D force layout). |
| `observability.rs` | Structured JSONL logging; tool-call audit records. |
| `server.rs` | Top-level server orchestration (MCP + ACP + Web lifecycle, graceful shutdown). |
| `watch.rs` | `notify`-based filesystem watcher; debounced re-ingest + smart schema rebuild. |
| `web.rs` | Hugo web UI mirror management; `web install / serve / build / status`. |
| `api.rs` | HTTP API handlers (`map_semantic_error`, health, galaxy, static, trust, console). |
| `semantic.rs` | vnext semantic layer — SQLite event ledger, purge registry, claim snapshot, AES-256-GCM object encryption. The sole claim authority. |
| `recovery.rs` | Backup / restore / upgrade / rollback contract (encrypted backup of objects/ledger/Git/config). |
| `trust.rs` | Trust + ops views: contradictions, stale, orphan, retrieval trace, provenance questions, destructive warnings. |
| `provider.rs` | `AiProvider` trait — provider-agnostic request/response; `OutboundPolicy` is deny-by-default; `ProviderError` retry classification. |
| `extraction.rs` | Evidence-linked extraction pipeline; `ExtractionPolicy` enforces quote-hash proof and prompt-injection resistance. |
| `source_ingest.rs` | `brain_ingest_source` — quarantines URL/file/text into chunk captures (SSRF-guarded). |

### `src/mcp/` — the MCP layer (5 files)

| File | Owns |
|---|---|
| `mcp/mod.rs` | `McpServer` struct, `engine()` / `manager()` / `semantic_store` accessors, rmcp `ServerHandler` impl, tool-list registration. |
| `mcp/tools.rs` | `tool_list()` (39 declarations), `annotations_for()` (tier policy), `call()` dispatcher with `catch_unwind`, schema helpers (`schema`, `str_prop`, `opt_str`, `opt_bool`, `opt_int`). |
| `mcp/handlers.rs` | One `pub fn handle_<name>(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult` per tool — argument parsing + `ops::` call + result formatting. |
| `mcp/helpers.rs` | `ToolResult` struct, `ok_text` / `err_text` wrappers, argument helpers (`arg_str`, `arg_str_req`, `arg_bool`, `arg_usize`), `resolve_wiki_name`, URI resource-change notification helpers. |
| `mcp/auth.rs` | `Capability` enum (Read / Capture / Propose / Confirm / Purge / Admin), `AuthPrincipal`, `AuthPolicy` (per-tool capability map + per-path entries, TLS enforcement). |

### `src/ops/` — shared business logic (CLI + MCP call the same code)

| File | Owns |
|---|---|
| `ops/mod.rs` | Re-exports. |
| `ops/spaces.rs` | `spaces_create`, `spaces_register`, `spaces_list`, `spaces_remove`. |
| `ops/content.rs` | `content_read`, `content_write`, `content_new`, `content_commit`. |
| `ops/ingest.rs` | `ingest` (validate + index + optional commit; `dry_run`, `redact`). |
| `ops/index.rs` | `rebuild_index`, `index_status`. |
| `ops/search.rs` | `search` (BM25 + facets). |
| `ops/graph.rs` | `graph` (render mermaid/dot/llms). |
| `ops/history.rs` | `history` (shell `git log`, NUL-delimited). |
| `ops/lint.rs` | `lint` (orphan, broken-link, missing-fields, stale, articulation-point, bridge, periphery). |
| `ops/schema.rs` | `schema list / show / add / remove / validate` + template rendering. |
| `ops/stats.rs` | `stats` (page counts, topology, staleness buckets). |
| `ops/suggest.rs` | `suggest` (three strategies + edge field suggestion). |
| `ops/export.rs` | `export` (llms-txt / llms-full / json) + `ExportFormat` / `ExportOptions`. |
| `ops/config.rs` | `config get / set / list`. |
| `ops/logs.rs` | `logs tail / list / clear`. |
| `ops/redact.rs` | `redact_body` — secret-pattern redaction pass (lossy). |

### `src/acp/` — ACP (Agent Client Protocol) layer

| File | Owns |
|---|---|
| `acp/mod.rs` | ACP module root; agent builder pattern (no `LocalSet`/channel). |
| `acp/server.rs` | ACP server lifecycle + cooperative cancellation + session cap. |
| `acp/helpers.rs` | NDJSON stdio framing; shared ACP utilities. |
| `acp/ingest.rs`, `acp/graph.rs`, `acp/lint.rs`, `acp/research.rs` | Six ACP workflows (ingest, graph, lint, research, use, lifecycle). |

### Tests

| Directory | Covers |
|---|---|
| `tests/` (~75 Rust files) | Rust unit + integration tests via `cargo test`. Includes `tests/mcp.rs` (tool count assertion: `assert_eq!(tools.len(), 39)`), `tests/brain_tools_v1.rs`, `tests/semantic_*.rs`, `tests/ops/` (one file per ops module), `tests/mcp_auth_boundary_v1.rs`, plus contract suites (`*_v1.rs`). Filesystem tests use `tempfile::tempdir()`. |
| `tests-integration/engine/` | Pytest suite — CLI subprocess (engine surface): content, search, ingest, lint, schema, spaces, stats, etc. |
| `tests-integration/mcp/` | Pytest suite — MCP stdio via the official `mcp` SDK: `test_brain_tools.py`, `test_content.py`, `test_search.py`, `test_negative.py`, `test_structural.py`, etc. |
| `tests-integration/acp/` | Pytest suite — ACP NDJSON stdio via `asyncio`: `test_lifecycle.py`, `test_session_cap.py`, `test_research.py`, etc. |
| `tests-integration/governance/` | Pytest — eval contract conformance (`test_eval_contract.py`). |
| `evals/v1/` | Evaluation harness: `run.py`, `cases/`, `contracts/`, `manifest.json`, `metrics.json`. |

> The three pytest suites (`engine/`, `mcp/`, `acp/`) intentionally
> reuse file names across folders; collecting them in one pytest
> process causes import mismatch. Always run them as separate suites
> (see [Running Tests](#running-tests)).

### Docs and supporting trees

| Path | Holds |
|---|---|
| `docs/specifications/engine/` | `engine-state.md`, `graph.md`, `index-management.md`, `ingest-pipeline.md`, `server.md`, `watch.md`. |
| `docs/specifications/model/` | `type-system.md`, `epistemic-model.md`, `page-content.md`, `global-config.md`, `wiki-toml.md`, `wiki-repository-layout.md`, `types/`. |
| `docs/specifications/tools/` | One spec per tool family: `search.md`, `content-operations.md`, `schema-management.md`, `export.md`, etc. |
| `docs/implementation/` | `mcp-tool-pattern.md` (canonical add-a-tool recipe), `rust.md`, `engine.md`, `graph-cache.md`, `index-manager.md`, `lock-patterns.md`, `petgraph-live.md`, `tantivy.md`, `type-registry.md`, `schema-change-detection.md`, `manager-pattern.md`. |
| `docs/decisions/` | Decision records grouped by release (`0.1.1/` … `0.4.1/`, `backlog/`). |
| `docs/adr/` | `0001-semantic-authority-time-privacy.md` — the vnext authority contract. |
| `docs/guides/` | `ci-cd.md`, `release.md`, `custom-types.md`. |
| `docs/architecture/`, `docs/security/`, `docs/plans/`, `docs/improvements/`, `docs/bug-reports/`, `docs/baseline/` | Cross-cutting docs. |
| `schemas/` | Default JSON schemas + body templates: `base.json`, `profile.json`, `concept.json` + `concept.md`, `semantic.json`, `procedure.json`, `paper.json` + `paper.md`, `skill.json`, `doc.json` + `doc.md`, `section.json` + `section.md`, `query-result.md`. |
| `BLUEPRINT.md` | Full design doc — stores, retrieval pipeline, deployment, ADRs 001–007, phased rollout, §11 performance budget. |
| `CHANGELOG.md` | Keep-a-Changelog format; `[Unreleased]` lands features before versioning. |
| `web/console/` | Svelte + TypeScript web console (galaxy graph, entity pages, search, ops dashboard) + Playwright e2e under `e2e/`. |
| `.github/workflows/` | `ci.yml`, `release.yml`, `integration.yml`, `dependabot.yml`. |

---

## Navigation with CodeGraph

CodeGraph is the structural index over the 265-file codebase. It is
cheaper than `grep` for any "where does X get called / what does X
touch" question, and it can answer flow questions grep cannot
(see `codegraph_trace` below). Prefer it as the first move.

### The five tools, in order of reach

| Tool | Use when | Returns |
|---|---|---|
| `codegraph_context` | **Start here for any task.** Describe the task in one sentence. | Entry points + related symbols + key code snippets in one call. Usually enough — no further search/Read/Grep needed. |
| `codegraph_search` | You know a symbol name (or partial) and want its location. | Locations only (file:line); no code. Use before `codegraph_node`/`explore` so you have exact names. |
| `codegraph_node` | You want one symbol's details + its **trail** (callers + callees with file:line). | Symbol + trail. Pass `includeCode: true` for source. Walk hop-by-hop by node-ing a trail entry. |
| `codegraph_explore` | You want **verbatim source of several related symbols** in one call. | Grouped source (line-numbered, byte-identical to Read). Far cheaper than N `Read` calls. |
| `codegraph_callers` / `codegraph_callees` / `codegraph_trace` / `codegraph_impact` | Impact analysis: who calls X, what X calls, the path from A to B, the blast radius of changing X. | Targeted lists / chains. |

Budget note: `codegraph_explore` is capped at 1 call per project in
this session — make it count. `codegraph_node` and `codegraph_context`
are not capped.

### Worked example: "Find every handler that touches `brain_*` tools"

The wrong way: `grep` for `brain_` across `src/`. The right way:

1. **Start with context.** Ask CodeGraph to assemble the picture:

   ```
   codegraph_context(
     task: "How are brain_* MCP tools dispatched, handled, and annotated?"
   )
   ```

   This returns `tool_list`, `call`, `annotations_for`, the
   `handle_brain_*` family, and the `McpServer` accessors in one shot.

2. **Walk the trail.** Node the dispatcher to see every handler it
   calls, each with a file:line:

   ```
   codegraph_node(symbol: "call", includeCode: false)
   ```

   The trail lists `handlers::handle_brain_status`,
   `handlers::handle_brain_capture`, … — every brain_* handler with
   its location in `src/mcp/handlers.rs`.

3. **Confirm the handler set is complete.** Cross-check against
   `annotations_for` — its `brain_capture | brain_confirm | …`
   match arm is the authoritative list of brain mutations:

   ```
   codegraph_node(symbol: "annotations_for", includeCode: true)
   ```

4. **Find the business logic each handler reaches.** Callees of one
   handler to confirm it talks to the semantic store, not directly
   to the ledger:

   ```
   codegraph_callees(symbol: "handle_brain_capture")
   ```

5. **Trace a flow end-to-end.** "How does an MCP call reach the
   SQLite ledger?" — a path grep cannot find:

   ```
   codegraph_trace(from: "call", to: "SemanticStore")
   ```

   If the trace breaks at dynamic dispatch, the tool says where and
   points you to the next `codegraph_node` to bridge the hop.

### When to fall back to Read

Use `Read` (or `Edit`) only after CodeGraph has pointed you at the
exact file:line. For batch inspection of several files at once prefer
`codegraph_explore` — it returns verbatim source (identical to Read)
without re-reading the whole context per file.

---

## Adding a New MCP Tool — End-to-End

This is the canonical contribution. The authoritative recipe lives at
`docs/implementation/mcp-tool-pattern.md`; this section reproduces it
concretely with a worked example. Every tool touches **the same 4–6
files** in the same order.

Worked example: a hypothetical `wiki_export_index` tool that exports
just the Tantivy index report for a wiki (a thin sibling of
`wiki_export`).

### Step 0 — Decide name, tier, and args

Before writing code, decide:

| Decision | Options | `wiki_export_index` choice |
|---|---|---|
| **Name** | `wiki_*` for engine surface, `brain_*` for semantic, bare alias for Blueprint read tier. | `wiki_export_index` |
| **Tier** | 1 read-only / 2 write-additive / 3 write-idempotent / 4 destructive. See `references/tool-reference.md`. | 1 (read-only — produces a report, mutates nothing) |
| **Args** | Required vs optional; default wiki; `format` if useful. | required: none; optional: `wiki`. |
| **CLI mirror?** | Most engine tools have a CLI twin; Blueprint aliases do not. | Yes: `llm-wiki index export`. |

The tier choice determines the match arm you add to `annotations_for()`
in step 3.

### Step 1 — Declare the tool in `tool_list()` (`src/mcp/tools.rs`)

Add a `Tool::new(...)` entry to the `vec![...]` inside `tool_list()`,
near the existing `wiki_export` declaration:

```rust
Tool::new(
    "wiki_export_index",
    "Export the Tantivy index report for a wiki (page counts, staleness, fields)",
    schema(
        json!({
            "wiki": opt_str("Target wiki name (default: from config)"),
        }),
        &[],  // no required parameters
    ),
),
```

Schema helpers (defined at the top of `tools.rs`):

| Helper | Shape |
|---|---|
| `str_prop(desc)` | `{"type": "string", "description": desc}` — use for required strings. |
| `opt_str(desc)` | Same shape — used for optional strings; whether a param is required is controlled by the `&[...]` array, not the helper. |
| `opt_bool(desc)` | `{"type": "boolean", "description": desc}`. |
| `opt_int(desc)` | `{"type": "integer", "description": desc}`. |

The `schema(props, required)` wrapper (top of `tools.rs`) emits the
standard `{"type":"object","properties":...,"required":[...]}` envelope.

### Step 2 — Add the tier in `annotations_for()` (`src/mcp/tools.rs`)

If you skip this step, your tool falls through to the default
`_ => write_additive()` arm (line 121) and clients will treat it as
non-read-only. For a read-only tool, extend the first match arm:

```rust
fn annotations_for(name: &str) -> ToolAnnotations {
    match name {
        // Read-only tools — no environment mutation.
        "wiki_search" | "wiki_list" | "wiki_content_read" | "wiki_history" | "wiki_stats"
        | "wiki_graph" | "wiki_resolve" | "wiki_lint" | "wiki_suggest" | "profile_get"
        | "semantic_search" | "semantic_get" | "procedural_find" | "procedural_get"
        | "graph_neighbors" | "audit_history" | "wiki_index_status" | "brain_status"
        | "brain_search" | "brain_get"
        | "wiki_export_index"   // ← NEW: read-only index report
            => read_only(),
        // … (brain_* mutations, destructive, idempotent, additive arms unchanged) …
        _ => write_additive(),
    }
}
```

For other tiers use the matching constructor: `write_additive()` (the
default), `write_idempotent()` (for `rebuild`/`set`-style tools), or
`write_destructive()` (rare; only if the tool removes data).

### Step 3 — Implement the handler (`src/mcp/handlers.rs`)

Add a `pub fn handle_<name>(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult`.
The handler parses args, resolves the wiki, calls an `ops::` function,
and formats the result:

```rust
// ── Index export ──────────────────────────────────────────────────────────────

/// Handle `wiki_export_index` — return the Tantivy index report for a wiki.
pub fn handle_export_index(server: &McpServer, args: &Map<String, Value>) -> ToolHandlerResult {
    let engine = server.engine();
    let wiki_name = resolve_wiki_name(&engine, args)?;

    let report = ops::index_report(&engine, &wiki_name)
        .map_err(|e| format!("{e}"))?;

    let s = serde_json::to_string_pretty(&report).map_err(|e| format!("{e}"))?;
    ok_text(s)
}
```

Argument helpers (from `helpers.rs`):

| Helper | Returns | Notes |
|---|---|---|
| `arg_str(args, key)` | `Option<String>` | Optional string param. |
| `arg_str_req(args, key)` | `Result<String, String>` | Required string; errors with `missing required parameter: <key>`. |
| `arg_bool(args, key)` | `bool` | Defaults to `false` if absent or non-bool. |
| `arg_usize(args, key)` | `Option<usize>` | Optional unsigned int. |
| `resolve_wiki_name(&engine, args)` | `Result<String, String>` | Resolves the `wiki` arg or falls back to `global.default_wiki`. |

Return helpers:

| Helper | Use |
|---|---|
| `ok_text(String)` | Success with text content (the common case). |
| `ok_json(json, text_fallback)` | Both structured JSON and a pretty-printed text fallback — use for read tools that return JSON. |
| `err_text(String)` | Used by the dispatcher, not by handlers — handlers return `Err(String)`. |

**Lock-drop pattern.** If the op needs `&WikiEngine` for write paths
(e.g. it calls `refresh_index`), drop the engine read lock before
calling it:

```rust
let engine = server.engine();
let wiki_name = resolve_wiki_name(&engine, args)?;
drop(engine);  // release read lock

let report = ops::my_write_op(&server.manager, &wiki_name, ...)
    .map_err(|e| format!("{e}"))?;
```

### Step 4 — Implement business logic in `src/ops/<module>.rs`

Handlers must not contain business logic — they parse, call, format.
The `ops::` function is shared with the CLI. For `wiki_export_index`,
add to `src/ops/index.rs`:

```rust
pub fn index_report(engine: &EngineState, wiki_name: &str) -> Result<IndexReport> {
    let space = engine.space(wiki_name)?;
    let manager = &space.index_manager;
    let reader = manager.reader()?;
    // …collect page counts, staleness, schema fields…
    Ok(report)
}
```

Per-wiki state lives on `SpaceContext`:

```rust
let space = engine.space(wiki_name)?;
space.type_registry   // SpaceTypeRegistry — validators, aliases
space.index_schema    // IndexSchema — tantivy field handles
space.wiki_root       // PathBuf — wiki/ directory
space.repo_root       // PathBuf — repository root
space.index_manager   // Arc<SpaceIndexManager> — index lifecycle
space.graph_cache     // WikiGraphCache — petgraph-live snapshot/backed
```

### Step 5 — Add the dispatch arm in `call()` (`src/mcp/tools.rs`)

Inside `call()`, add the match armino the alphabetical neighborhood of
the tool name (the existing list is grouped: spaces → config → content
→ search → ingest → index → graph → schema → export → aliases → brain_*):

```rust
pub fn call(server: &McpServer, name: &str, args: &Map<String, Value>) -> ToolResult {
    let _span = tracing::info_span!("tool_call", tool = name).entered();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match name {
        // … existing arms …
        "wiki_export" => handlers::handle_export(server, args),
        "wiki_export_index" => handlers::handle_export_index(server, args),  // ← NEW
        // …
        _ => Err(format!("unknown tool: {name}")),
    }));
    // … (panic / ok / err handling unchanged) …
}
```

The dispatcher wraps every call in `catch_unwind`; a panicking handler
returns `internal error: tool panicked` rather than tearing down the
server (see [Debugging Common Failures](#debugging-common-failures)).

### Step 6 — Write integration tests under `tests-integration/mcp/`

Add a pytest module that drives the MCP stdio transport via the
official `mcp` SDK. Follow the shape of `tests-integration/mcp/test_export.py`:

```python
# tests-integration/mcp/test_export_index.py
import pytest
from .conftest import mcp_call

@pytest.mark.asyncio
async def test_export_index_returns_report(mcp_session):
    result = await mcp_call(mcp_session, "wiki_export_index", {"wiki": "brain"})
    assert "pages_indexed" in result.text
```

Also add a Rust contract test that asserts the tool is declared and
returns the right tier:

```rust
// tests/mcp.rs
#[test]
fn wiki_export_index_tool_exists_and_is_read_only() {
    let tools = tool_list();
    let t = tools.iter().find(|t| t.name == "wiki_export_index")
        .expect("wiki_export_index must be declared");
    assert_eq!(t.annotations.unwrap().read_only_hint, Some(true));
    // …and bump the count assertion:
    assert_eq!(tools.len(), 40);  // was 39
}
```

### Step 7 — Document

| Artifact | What to add |
|---|---|
| `docs/specifications/tools/<name>.md` | New spec: args, returns, tier, examples, errors. |
| `references/tool-reference.md` (this skill) | New row in the matching tier table; new MCP↔CLI mapping row. |
| `CHANGELOG.md` | Entry under `[Unreleased] → Added`. |
| `README.md` "Current Tool Surface" | Append the tool name. |

### Pre-PR checklist

- [ ] Tool definition in `tools.rs` with the correct schema.
- [ ] Handler in `handlers.rs` follows the parse → ops → format pattern.
- [ ] Tier added to `annotations_for()` (do not rely on the additive default).
- [ ] Dispatch arm in `call()`.
- [ ] Business logic in `src/ops/<module>.rs`, shared with CLI where applicable.
- [ ] CLI subcommand in `cli.rs` + dispatch in `main.rs` (if a CLI twin exists).
- [ ] Tool-count assertion in `tests/mcp.rs` updated (`assert_eq!(tools.len(), N)`).
- [ ] Integration test under `tests-integration/mcp/`.
- [ ] Spec in `docs/specifications/tools/`.
- [ ] `references/tool-reference.md` row added.
- [ ] `CHANGELOG.md` entry.

---

## Running Tests

The test matrix spans Rust (`cargo test`) and Python (`make
validate-py-*`). Commands below are taken verbatim from
`docs/guides/ci-cd.md`, the repo `Makefile`, `CONTRIBUTING.md`, and
`README.md`.

### Rust — unit + integration

| Command | What it does |
|---|---|
| `cargo build` | Debug build. |
| `cargo build --release` | Release build. |
| `cargo build --release --locked` | Release build with locked `Cargo.lock` (CI shape). |
| `cargo test` | All Rust unit + integration tests under `tests/`. |
| `cargo test --doc` | Rustdoc examples. |
| `cargo fmt --check` | Formatting check (must pass with zero diff). |
| `cargo fmt` | Auto-format. |
| `cargo clippy --all-targets -- -D warnings` | Lint — must pass with zero warnings. |
| `cargo clippy -- -D warnings` | Lint (CONTRIBUTING variant). |

Windows MSVC toolchain (from `README.md`):

```powershell
rustup toolchain install 1.95-x86_64-pc-windows-msvc --component rustfmt --component clippy
cargo +1.95-x86_64-pc-windows-msvc test
```

The repo pins Rust **1.95** via `rust-toolchain.toml` (note:
`CONTRIBUTING.md` mentions 1.93.0 in `.tool-versions` — the
`rust-toolchain.toml` and CI use 1.95; treat 1.95 as authoritative, see
[Discrepancies](#discrepancies)).

### Python — integration suites (pytest + uv)

The integration project lives in `tests-integration/` and is managed by
`uv`. Three suites cover all transports. Run them via the repo
`Makefile`:

| Make target | Suite | Transport |
|---|---|---|
| `make validate-py` | All three | (engine + mcp + acp) |
| `make validate-py-engine` | `tests-integration/engine/` | CLI subprocess |
| `make validate-py-mcp` | `tests-integration/mcp/` | MCP stdio (official `mcp` SDK) |
| `make validate-py-acp` | `tests-integration/acp/` | ACP NDJSON stdio (`asyncio`) |

Each `validate-py-*` target depends on `build` (debug binary) and
passes `BINARY=$(CURDIR)/target/debug/llm-wiki` to the inner
`tests-integration/Makefile`. The inner targets are `test-engine`,
`test-mcp`, `test-acp`, and `test` (all three).

Run a single suite directly with uv (from `README.md`):

```bash
cd tests-integration
uv sync --group dev
```

Linux/macOS:

```bash
export LLM_WIKI_BIN=/path/to/llm-wiki
uv run pytest engine/ -v
uv run pytest acp/ -v
uv run pytest mcp/ -v
```

Windows:

```powershell
New-Item -ItemType Directory -Force -Path C:\tmp | Out-Null
$env:PYTHONUTF8 = "1"
$env:LLM_WIKI_BIN = "C:\path\to\llm-wiki.exe"
uv run pytest engine/ -v
uv run pytest acp/ -v
uv run pytest mcp/ -v
```

> Run the three suites **separately**. The test files intentionally
> reuse names across `engine/`, `acp/`, and `mcp/`, so collecting all
> folders in one pytest process causes import mismatch.

### GitHub Actions — `.github/workflows/integration.yml`

Triggered automatically on push to `main` (and PRs to `main`) that
touch `src/**`, `tests-integration/**`, `Cargo.toml`, `Cargo.lock`, or
the workflow itself. Also runnable manually from the Actions tab →
**Integration Tests** → **Run workflow** with a `suite` input:

| `suite` value | Runs |
|---|---|
| `all` (default) | engine + mcp + acp |
| `engine` | engine only |
| `mcp` | mcp only |
| `acp` | acp only |

The workflow (Ubuntu runner, Rust 1.95, `Swatinem/rust-cache@v2`):

1. `cargo build --locked` (debug binary).
2. `uv sync --group dev` (Python deps).
3. `make -C tests-integration test-<suite> BINARY=$(pwd)/target/debug/llm-wiki`.

No external tools required (`jq`, `mcptools`, etc.). Dependencies are
declared in `tests-integration/pyproject.toml`.

Other workflows:

| File | Purpose |
|---|---|
| `.github/workflows/ci.yml` | Rust fmt + clippy + cargo test on PR. |
| `.github/workflows/release.yml` | Triggered by `vx.y.z` tag — builds 5 targets, creates GitHub release, publishes to crates.io. |
| `.github/workflows/dependabot.yml` | Dependency bump automation. |

### Pre-release checklist (`make pre-release`)

The repo `Makefile` exposes a single target that mirrors
`docs/guides/release.md` — it runs `cargo test`, `cargo test --doc`,
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo build --release --locked`. Add `make validate-py` and trigger
the **Integration Tests** workflow (`suite: all`) on the release
branch before opening the final PR.

### Local smoke test

After building, start the server and exercise one tool manually:

```bash
cargo build --release
./target/release/llm-wiki spaces create ~/wikis/brain --name brain --set-default
./target/release/llm-wiki index rebuild --wiki brain
./target/release/llm-wiki serve        # stdio MCP — pipe JSON-RPC, or use a client
```

For HTTP transport:

```bash
./target/release/llm-wiki serve --http :47778
curl -X POST http://127.0.0.1:47778/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

On Windows use `.\target\release\llm-wiki.exe`.

---

## Working with Schemas

Schemas are JSON Schema (Draft 2020-12) files in `schemas/`. The engine
discovers types from them automatically via the `x-wiki-types`
extension; the type registry validates frontmatter on ingest. Full
reference: `docs/specifications/model/type-system.md` and
`docs/guides/custom-types.md`.

### Schema anatomy

A schema declares: standard JSON Schema (`type`, `properties`,
`required`, `additionalProperties`), plus three engine extensions:

| Extension | Purpose |
|---|---|
| `x-wiki-types` | Maps a frontmatter `type` value to this schema. The engine scans `schemas/*.json` for this key — dropping a file in is enough to register a new type. |
| `x-index-aliases` | Field-name aliases. `{"subject": "title"}` makes the index see `title` regardless of the frontmatter name; search works uniformly across types. |
| `x-graph-edges` | Declares outgoing graph edges. Fields named here are indexed as keyword slug-lists and become labeled directed edges in the petgraph projection. |

Example (excerpted from `docs/guides/custom-types.md`):

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Meeting notes",
  "type": "object",
  "required": ["title", "type"],
  "properties": {
    "title": { "type": "string" },
    "type": { "type": "string" },
    "concepts": { "type": "array", "items": { "type": "string" } }
  },
  "x-wiki-types": { "meeting": "Meeting notes with attendees and action items" },
  "x-graph-edges": {
    "concepts": {
      "relation": "discussed-in",
      "direction": "outgoing",
      "target_types": ["concept"]
    }
  },
  "additionalProperties": true
}
```

### The `default` type invariant

The `default` type (the catch-all page schema) cannot be removed
(`wiki_schema action: remove` refuses it). Every schema must:

- Be valid JSON Schema Draft 2020-12.
- Require at least `title` and `type` in its `required` array.
- Declare its type name via `x-wiki-types` (otherwise it is invisible
  to the type registry unless wired through `wiki.toml`).

### Adding a custom schema file

1. **Create** `schemas/<type>.json` (Draft 2020-12, `x-wiki-types`,
   required `title`+`type`).
2. **Register** explicitly (optional — only if you want the file
   copied into a wiki's `schemas/` from elsewhere):

   ```bash
   llm-wiki schema add <type> /path/to/<type>.json --wiki brain
   ```

3. **Validate** the schema and its index resolution:

   ```bash
   llm-wiki schema validate <type> --wiki brain
   llm-wiki schema show <type> --template   # frontmatter template
   ```

4. **Write pages** with `type: <type>` in their frontmatter; `wiki_ingest`
   validates against the schema and indexes the declared edges.

Discovery is automatic: the engine scans `schemas/*.json` at space
mount time. Use `wiki.toml` `[types.<type>]` only to **remap** a type
to a non-default schema file:

```toml
[types.meeting]
schema = "schemas/my-custom-meeting.json"
description = "Custom meeting schema"
```

### Body templates

A body template at `schemas/<type>.md` (plain Markdown, no frontmatter)
is used by `wiki_content_new` to scaffold page structure. The engine
prepends the scaffolded frontmatter automatically. Example:
`schemas/meeting.md`:

```markdown
## Attendees



## Agenda



## Action Items

```

The existing default schemas ship both files: `concept.json` +
`concept.md`, `paper.json` + `paper.md`, `doc.json` + `doc.md`,
`section.json` + `section.md`. `profile.json`, `semantic.json`,
`procedure.json`, `skill.json` ship without a body template.

### Schema change detection

The watcher rebuilds the type registry when schemas change
(`src/watch.rs` "smart schema rebuild"). For the contract see
`docs/implementation/schema-change-detection.md`.

### Inspecting and removing schemas

```bash
llm-wiki schema list                       # all registered types
llm-wiki schema show <type>                # JSON Schema
llm-wiki schema show <type> --template     # frontmatter template
llm-wiki schema validate <type>            # validate the schema + index resolution
```

`wiki_schema action: remove` is **tier 4 (destructive)** — its
`delete_pages: true` path deletes the schema file **and** every `.md`
page of that type from disk. Always pass `dry_run: true` first and
confirm the affected page count with the user. Cannot remove the
`default` type. See `references/tool-reference.md` §"Tier 4" for the
full blast-radius sequence.

---

## Engine Internals Quick Reference

One paragraph per module — enough to know where to look. For the full
spec read `docs/specifications/engine/*.md`.

### `engine.rs` — orchestrator

Holds `EngineState`: the registry of mounted `SpaceContext`s (one per
wiki) plus the global config. `SpaceContext` bundles everything a
handler needs for one wiki — `type_registry`, `index_schema`,
`index_manager`, `wiki_root`, `repo_root`, `graph_cache`, and a
`GenerationCache<CommunityData>` that shares the graph generation key.
The `engine()` accessor on `McpServer` returns a read-locked
`EngineState`; handlers clone what they need and `drop(engine)` before
any write op that needs `&WikiEngine`.

### `git.rs` — libgit2 wrapper

Owns `init_repo`, `commit`, `commit_paths`, `log` (NUL-delimited
parsing), and history walking. Retry/backoff is built in
(`MAX_RETRIES = 3`, `BACKOFF_MS = [100, 200, 400]`) to absorb
transient `.git/index.lock` contention. The signature falls back to
`llm-wiki <llm-wiki@localhost>` when no git identity is configured.
See [Debugging Common Failures](#debugging-common-failures) for the
lock-contention recovery path.

### `index_manager.rs` + `index_schema.rs` — Tantivy

`SpaceIndexManager` owns the Tantivy `Index`, `IndexReader`, and
`IndexWriter`; tracks an `AtomicU64` generation counter so the graph
cache can key on it. Supports full rebuild (`rebuild_all_pages`) and
incremental update from git deltas; reports staleness by kind
(`StalenessKind`). `IndexSchema` is **computed from the type
registry**, not hardcoded — adding a schema field updates the Tantivy
schema automatically. See `docs/implementation/tantivy.md` and
`docs/implementation/index-manager.md`.

### `graph.rs` — petgraph concept graph

Builds a `DiGraph<PageNode, Edge>` (petgraph) from frontmatter edges
(`x-graph-edges`) plus body `[[wikilink]]` / `[text](slug)` links.
Warm-started via **petgraph-live**: `WikiGraphCache` is either
`NoSnapshot` (in-memory only) or `WithSnapshot` (snapshot-backed,
reloaded from disk on startup). `GenerationCache<CommunityData>` is
co-keyed so community detection survives cache invalidation. Provides
topology metrics (diameter, radius, center, articulation points,
bridges) for `wiki_stats` and `wiki_lint`. See
`docs/decisions/0.4.0/petgraph-live.md` and
`docs/implementation/petgraph-live.md`.

### `ingest.rs` — validate + index + commit pipeline

The pipeline behind `wiki_ingest`. `IngestOptions` carries `dry_run`,
`auto_commit`, `changed_paths` (incremental) or `None` (full audit),
and an optional redaction pass. Normalizes line endings (CRLF→LF)
before validation, walks the target tree with `walkdir`, validates
each file's frontmatter against the type registry, calls the index
manager to update Tantivy, and (when `auto_commit`) calls `git::commit`
or `git::commit_paths`. See `docs/specifications/engine/ingest-pipeline.md`.

### `links.rs` — wikilink and URI resolution

`ParsedLink::parse` classifies a raw link string as `Local(slug)` or
`CrossWiki { wiki, slug }` (for `wiki://` URIs). The body walker
extracts both `[[slug]]` and `[text](slug)` forms (CommonMark body
links — see `docs/decisions/0.2.0/commonmark-body-links.md`). The
graph builder consults this module to construct edges; cross-wiki
links resolve at graph build time (no schema change needed — see
`docs/decisions/0.2.0/cross-wiki-links.md`). Code-block false positives
are a known shared limitation (the walker is not a full Markdown
parser).

### `provider.rs` + `extraction.rs` — AI provider for `brain_extract`

`provider.rs` defines the `AiProvider` trait — a provider-agnostic
boundary (prompt + sampling params, no endpoint/model in the wire
shape). `ProviderConfig` holds endpoint + key reference + model names
as **config** (not durable schema) and carries a kill switch.
`OutboundPolicy` is deny-by-default: `local_only` requests and
requests carrying detected secrets are denied before they reach the
provider. `ProviderError` covers every failure mode with an
`is_retryable` classification.

`extraction.rs` is the evidence-linked extraction pipeline.
`ExtractionPolicy` validates each candidate: evidence spans must be
well-formed, a candidate with no evidence is `unsupported`, and the
source is **never treated as instructions to the worker**
(prompt-injection resistance). Each evidence span carries a `quote_hash
= sha256(rendition_bytes[byte_start..byte_end])` that is verified
against the actual chunk bytes — a `QuoteHashMismatch` means the span
does not mechanically prove what it claims to quote. The pipeline
yields `ExtractionProposal`s + an `ExtractionAudit`; there is **no
commit path** — the worker physically cannot confirm; only the
semantic store can.

### `semantic.rs`, `recovery.rs`, `trust.rs` — vnext semantic layer

`semantic.rs` is the isolated semantic authority spike: SQLite is the
sole semantic-transition authority, with append-only event ledger,
monotonic purge registry, deterministic JSON claim snapshot
(replaceable, projector-only), and an AES-256-Gcm object store
(SHA-256 addressed). One serialized writer per owner ledger; all
commands enter via `BEGIN IMMEDIATE`. This is the authority behind
every `brain_*` tool.

`recovery.rs` defines the backup / restore / upgrade / rollback
contract (encrypted automated backup covers objects/ledger/Git/config;
clean-host restore drill syncs the purge registry before any decrypt;
RPO/RTO is recorded and monitored). `trust.rs` surfaces trust + ops
views: `TrustFlag` (contradiction, stale, orphan), `RetrievalTrace`,
`ProvenanceQuestion`/`ProvenanceAnswer`, `JobSummary`, `BackupHealth`,
and `DestructiveWarning` (hard-purge preview with irreversible flag
and single-use nonce).

See `docs/adr/0001-semantic-authority-time-privacy.md` for the
authority contract and `references/architecture.md` §"The vnext
Authority Model" for the readable-claim formula.

---

## Debugging Common Failures

### `unknown tool: <name>`

| Field | Value |
|---|---|
| Symptom | Tool call returns the string `unknown tool: <name>`. |
| Cause | The dispatcher in `src/mcp/tools.rs::call()` hit its fallback arm (`_ => Err(format!("unknown tool: {name}"))`, line 708). |
| Most likely reason | You added the `Tool::new(...)` declaration and the handler, but forgot the dispatch arm in `call()`. |
| Recovery | Add the missing `"name" => handlers::handle_name(server, args),` arm. If the tool is genuinely new, also bump the `tools.len()` assertion in `tests/mcp.rs`. If the tool is from a newer version than the running server, rebuild and restart `llm-wiki serve`. |

### Schema validation failure on ingest

| Field | Value |
|---|---|
| Symptom | `wiki_ingest` returns warnings like `missing required field` or `invalid type`. |
| Cause | Frontmatter does not satisfy the page's type schema. |
| Recovery | Run `wiki_ingest` with `dry_run: true` first — it validates all files and reports every violation without committing. Run `wiki_schema action: show` with `template: true` on the target type to see required fields. Fix the frontmatter, re-run `wiki_ingest`. |

### Git index lock contention

| Field | Value |
|---|---|
| Symptom | `wiki_content_commit` or `wiki_ingest` (with `auto_commit`) fails with a git index lock error (`index.lock exists`). |
| Cause | Another git operation is in flight (concurrent ingest, external git client, or a crashed process). |
| Engine mitigation | `src/git.rs` already retries with backoff (`MAX_RETRIES = 3`, `BACKOFF_MS = [100, 200, 400]`) before surfacing the error. |
| Recovery | Wait and retry — most contention resolves in under a second. If a stale `.git/index.lock` remains after a confirmed crash, verify no git process is running, then remove it manually. The brain-mcp server is the only writer in normal operation. |

### Panicking handler

| Field | Value |
|---|---|
| Symptom | Tool call returns `internal error: tool panicked`. |
| Cause | The handler panicked; `call()` caught it via `std::panic::catch_unwind(AssertUnwindSafe(|| match name { ... }))` and returned the canned error string (lines 738–747). |
| Recovery | Check the structured logs — the dispatcher emits `tracing::error!(tool = name, "tool handler panicked")` on the `tool_call` span. Reproduce locally with `cargo test` and `RUST_BACKTRACE=1`. Common causes: an `unwrap()` on a `None`/`Err` from the engine, an out-of-range slice, or an unhandled `?` from an ops function that should have been `.map_err(|e| format!("{e}"))?`. |

### `IDEMPOTENCY_CONFLICT`

| Field | Value |
|---|---|
| Symptom | A `brain_*` write returns `IDEMPOTENCY_CONFLICT` (HTTP 409 `conflict`). |
| Cause | The same `(owner_id, client_id, operation_id)` was replayed with a **different** tool or payload. The server returns the conflict without mutating state (ADR-0001 §2). |
| Recovery | Generate a fresh `operation_id` (UUIDv7 or UUIDv4) for the new logical operation. Do **not** retry with a changed payload and the same id — that is the conflict. If you intended to **replay** after a network blip, reuse the same `operation_id` **and** the same payload byte-for-byte; the server returns the stored outcome identically. |
| HTTP surface | `src/api.rs::map_semantic_error` maps `SemanticError::IdempotencyConflict` → `409 conflict`. |

### Stale search index

| Field | Value |
|---|---|
| Symptom | `wiki_search` / `wiki_list` results do not include a page you just wrote; `wiki_index_status` reports `stale: true`. |
| Recovery | `wiki_index_rebuild` (tier 3, idempotent — safe to retry). On a healthy wiki `wiki_ingest` keeps the index in sync; staleness usually means an ingest was skipped or interrupted. |

### Capability denied

| Field | Value |
|---|---|
| Symptom | `CapabilityDenied` from a `brain_*` write path. |
| Cause | The calling client lacks the required capability (e.g. a propose-only worker tried a confirm/purge). |
| Recovery | Re-issue with a client that holds the right capability (see `src/mcp/auth.rs` `Capability` enum: Read / Capture / Propose / Confirm / Purge / Admin), or escalate to the operator. |

---

## Contributing Workflow

### Branch strategy

The active development branch is **`vnext/phase-0`**. Per
`docs/guides/release.md`, `main` is always releasable (tagged commits
only); feature work lands on a `release/vX.Y.Z` integration branch via
`feat/...` PRs. In practice, phased work (Phase F, etc.) commits
directly to `vnext/phase-0` with the prefix below.

### Commit message conventions

Recent commits follow this shape (from `git log --oneline`):

```
feat(phase-f): F3.4 — external security review + SECURITY.md refresh + Phase F Gate close
feat(phase-f): F3.3 — schema upgrade runner + RPO/RTO recorder
fix(phase-f): F3.2 review — staging panic cleanup + per-blob digests
feat(phase-f): F3.2 — restore drill runner + restore-drill.json + registry fail-closed
feat(phase-f): F3.1 — encrypted backup + BackupReport producer
docs(plan): mark Task F2 CLOSED — Observability + ingest limits
fix(phase-f): F2.3 review — evict empty client entries in IngestRateLimiter (memory bound)
```

Pattern: `<type>(<scope>): <ticket> — <subject>`. Types in use:
`feat`, `fix`, `docs`, `chore`, `refactor`. Scope is the phase
(`phase-f`) or area (`plan`, `engine`). The subject is imperative and
short. For task-review follow-ups, append `review — <what was cleaned
up>`.

For engine changes that ship to release, the `CONTRIBUTING.md` and
release guide also accept the conventional `feat:`, `fix:`, `chore:
bump version to x.y.z` style. Match the surrounding commits.

### PR + CodeGraph impact analysis

Before merging a non-trivial change:

1. Run the relevant impact query in CodeGraph to confirm the blast
   radius matches your expectation:

   ```
   codegraph_impact(symbol: "<changed symbol>", depth: 2)
   ```

2. Run `make pre-release` (Rust) and `make validate-py` (Python) — or
   at least the matching `validate-py-<suite>`.

3. Trigger the **Integration Tests** workflow on your branch with
   `suite: all`.

4. Update `CHANGELOG.md` under `[Unreleased]` (see below).

### CHANGELOG

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Add entries under `[Unreleased]` → `### Added` / `### Changed` /
`### Fixed` / `### Removed`. On release, the entries move under a
versioned heading (`## [0.4.16] — 2026-MM-DD`) and a fresh
`[Unreleased]` is opened.

Example entry for a new tool:

```markdown
## [Unreleased]

### Added
- **`wiki_export_index` tool** — read-only index report (page counts,
  staleness, schema fields) for a wiki; tier-1 read-only.
```

### No LLM dependency rule

From `CONTRIBUTING.md`: the wiki engine makes **zero LLM calls**. All
intelligence is supplied by an external LLM that calls the wiki via
CLI or MCP. Do **not** add any LLM client crate as a dependency. The
`provider.rs` `AiProvider` trait is the boundary; concrete adapters
live in the deployment layer and implement the trait.

### Documentation standards

From `docs/guides/release.md` pre-release checklist:

- All improvement spec files have `status: implemented` and tasks
  checked.
- `docs/specifications/` reflects any changed tool signatures or
  config shapes.
- `docs/guides/` covers every user-facing feature added in this
  release.
- Public Rust types and functions have `///` rustdoc comments;
  `cargo doc --no-deps` emits zero warnings.

---

## Performance and Profiling

The performance budget lives in `BLUEPRINT.md` §11 (reproduced in
`references/architecture.md` §"Performance Budget"). Selected targets:

| Operation | Target | Stretch | Hard limit |
|---|---|---|---|
| `profile_get` | <100 ms | <50 ms | 200 ms |
| `semantic_search` top-10 (1K pages) | <500 ms | <300 ms | 1000 ms |
| `semantic_search` top-10 (10K pages) | <800 ms | <500 ms | 1500 ms |
| `procedural_find` | <300 ms | <150 ms | 600 ms |
| `wiki_ingest` 1 page | <2 s | <1 s | 5 s |
| Full re-embed (1K pages) | <20 min | <10 min | 30 min |
| Consolidate dry-run (1K pages) | <5 min | <2 min | 10 min |

Operating rule (BLUEPRINT §11): **over stretch → optimize. Over
target → investigate. Over hard limit → page.**

### Measuring tool latency

Every tool call emits a `tracing` span `tool_call` keyed on the tool
name (see `src/mcp/tools.rs::call()` line 666). The structured JSONL
audit log records timestamp, client, tool, args hash, duration, and
status — written by `src/observability.rs`.

To measure latency locally:

1. Run the server with structured logging enabled:

   ```bash
   RUST_LOG=debug ./target/release/llm-wiki serve --http :47778
   ```

2. Tail the JSONL log (path from config `logging.*`):

   ```bash
   llm-wiki logs tail --lines 100
   ```

3. Filter for the tool you are profiling and read the `duration_ms`
   field. Compare against the table above.

For end-to-end timing of an integration flow, wrap the Python
integration test with `pytest --durations=10` to surface the slowest
test cases.

For Tantivy-specific profiling see `docs/implementation/tantivy.md`;
for graph cache behavior see `docs/implementation/graph-cache.md` and
`docs/decisions/0.3.0/graph-cache.md` (cache keyed on index
generation; community map co-located; filtered requests bypass the
cache).

---

## Further Reading

Repo docs:

| Source | What it covers |
|---|---|
| `BLUEPRINT.md` | Full design doc: stores, retrieval pipeline, deployment, ADRs 001–007, phased rollout, §11 performance budget. |
| `docs/adr/0001-semantic-authority-time-privacy.md` | vnext authority contract — identity, time, privacy, purge, authorization. |
| `docs/overview.md` | Engine-level overview: tools, type system, epistemic model, layout. |
| `docs/specifications/engine/*.md` | `engine-state.md`, `index-management.md`, `graph.md`, `ingest-pipeline.md`, `server.md`, `watch.md`. |
| `docs/specifications/model/type-system.md` | Type discovery, `x-wiki-types`, `x-index-aliases`, `x-graph-edges`, custom types. |
| `docs/specifications/model/epistemic-model.md` | Why the type taxonomy exists; failure modes. |
| `docs/specifications/tools/*.md` | One spec per tool family. |
| `docs/implementation/mcp-tool-pattern.md` | Canonical add-a-tool recipe (mirrors this playbook). |
| `docs/implementation/rust.md`, `lock-patterns.md`, `petgraph-live.md`, `tantivy.md` | Toolchain, locking, graph cache, Tantivy internals. |
| `docs/decisions/README.md` | Decision records grouped by release (0.1.1 → 0.4.1, backlog). Notable: `0.4.0/petgraph-live.md`, `0.3.0/graph-cache.md`, `0.2.0/cross-wiki-links.md`, `0.1.1/json-schema-validation.md`. |
| `docs/guides/ci-cd.md`, `release.md`, `custom-types.md` | CI patterns, release/hotfix procedures, worked custom-type example. |
| `CONTRIBUTING.md` | Prerequisites, build/test commands, no-LLM-dependency rule, release process. |
| `README.md` | Setup, install, MCP client config, systemd, troubleshooting. |

In this skill (`references/`):

| Reference | When to read |
|---|---|
| `architecture.md` | The why: three stores, event-ledger authority, epistemic model, performance budget. |
| `tool-reference.md` | The how: 39-tool matrix with args, tiers, examples, MCP↔CLI mapping. |
| `type-system.md` | Per-type catalog with required fields, status enums, acid-test examples. |
| `anti-patterns.md` | Risky patterns to refuse before acting. |
| `workflows/operator.md` | Day-to-day memory and knowledge tasks. |
| `workflows/deployer.md` | Deployment, systemd, MCP client config, backups. |

When this playbook and the repo disagree, the repo wins — re-verify
against `src/`, `docs/`, and `BLUEPRINT.md`, then update this file.

---

## Discrepancies

Flagged during verification of this playbook against source (do not
guess — confirm before acting):

- **Rust toolchain version.** `README.md` and `rust-toolchain.toml`
  specify Rust **1.95+**; `CONTRIBUTING.md` mentions 1.93.0 in
  `.tool-versions`; CI (`.github/workflows/integration.yml`) pins
  `1.95`. Treat **1.95** as authoritative (rust-toolchain.toml + CI).
- **Tool count.** `tests/mcp.rs` line 41 asserts `tools.len() == 39`.
  The brief for this playbook said "39 MCP tools" and `tool-reference.md`
  says "39-tool matrix"; the BLUEPRINT and `SKILL.md` use the
  aspirational "~40". The actual `tool_list()` count is **39**.
- **`wiki_spaces_list` annotation.** Behaviorally read-only (it lists
  entries and mutates nothing) but classified **write-additive** in
  `annotations_for()` (conservative default for the space-management
  family). This is a deliberate, flagged discrepancy — see
  `references/tool-reference.md` §"Annotation policy".
