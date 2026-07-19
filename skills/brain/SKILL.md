---
name: brain
description: Use when working with the user's brain-mcp / llm-wiki knowledge server from Claude Code, Claude Desktop, Codex, Codex Desktop, Zed, Cursor, or any MCP client. Trigger for reading or writing long-term memory, profile pages, concepts, entities, sources, decisions, procedures, semantic claims, event ledger, propose/confirm/supersede flows, purge registry, wiki schemas, MCP setup (stdio or HTTP), remote MCP troubleshooting, ACP workflows, Hugo web UI refreshes, index rebuilds, audit history, and any task involving the brain MCP tools (`profile_get`, `semantic_search`, `semantic_get`, `procedural_find`, `procedural_get`, `graph_neighbors`, `audit_history`, `brain_status`, `brain_search`, `brain_get`, `brain_capture`, `brain_confirm`, `brain_supersede`, `brain_propose`, `brain_ingest_source`, `brain_extract`, or the `llm-wiki` CLI).
---

# Brain MCP

A long-term knowledge system built on top of `brain-mcp` (a Rust binary, crate
`llm-wiki-engine`). The server exposes 39 MCP tools that turn a folder of
Markdown files plus an event ledger into a searchable, auditable, recoverable
memory for agents.

## Core Rule

Treat `brain-mcp` as the **single source of truth** for the operator's
long-term knowledge. Use MCP tools when available; fall back to the `llm-wiki`
CLI only for server setup, repair, or operations that have no MCP equivalent.

## Five Iron Rules

1. **No silent writes.** Every write that changes durable user memory or
   semantic claims must go through propose → confirm. Never write profile
   pages, semantic claims, or procedures without first proposing the change
   unless the user explicitly said "save this now".
2. **The Event Ledger is the semantic authority.** Markdown + Git is the
   authored document layer; the append-only event ledger is the sole
   authority for claim state. A readable claim is
   `project(replay(ledger)) − purge_registry.denied_ids`. Never reconstruct
   claims from Markdown alone. See `references/architecture.md`.
3. **Type decides flow.** Profile pages use the propose/commit gate;
   semantic claims use capture/confirm or propose/extract; procedures
   require a `verification` block before promotion. Wrong type = wrong
   flow = broken audit. See `references/type-system.md`.
4. **Read before write.** Before proposing a change to an existing page or
   claim, read the current state (`profile_get`, `semantic_get`,
   `brain_get`, `brain_search`) so the proposal is a real diff, not a
   blind overwrite.
5. **Irreversible operations require explicit user consent.** `wiki_schema`
   with `action: remove`, `wiki_spaces_remove`, and any purge-style
   operation are destructive. Confirm with the user before running them,
   and explain the blast radius.

## Live Tools vs BLUEPRINT Intent

The MCP server today exposes 39 tools. The BLUEPRINT (§6) describes
additional tools that are **design intent, not yet wired**:
`memory_propose`, `memory_commit`, `profile_propose_update`,
`procedure_propose`, `procedure_promote`, `procedure_demote`,
`consolidate_run`, `consolidate_apply`, `audit_diff`, `lint_run`.

The "propose → confirm" discipline in Iron Rule #1 is enforced today via
the **wired** tools, not the aspirational ones:

- Semantic claims → `brain_capture` / `brain_propose` → `brain_confirm`
  → `brain_supersede`
- Markdown pages (incl. profile, procedure) → `wiki_content_write`
  → `wiki_ingest` (with `redact: true` when secrets may exist)
- External sources → `brain_ingest_source` → `brain_extract`

Do NOT call `memory_propose`, `profile_propose_update`,
`procedure_promote`, `consolidate_run`, or any BLUEPRINT-only tool by
name — they will return "unknown tool". Verify the live tool list in
`references/tool-reference.md` (verified against `src/mcp/tools.rs`)
when in doubt.

## Source of Truth

The wiki repository created by `llm-wiki spaces create ... --name brain` has
this shape:

```text
brain/
  wiki/              # source-of-truth Markdown content (authored layer)
    profile/
    concepts/
    entities/
    sources/
    projects/
    decisions/
    procedural/
  schemas/           # JSON schemas + body templates
  inbox/             # ingest staging
  raw/               # immutable archive (originals preserved)
  site/              # generated Hugo web UI scaffold and mirror
  wiki.toml
  .git/
```

In vnext, the semantic layer is **additionally** backed by:

- An append-only **event ledger** (sole claim authority).
- A **purge registry** (irreversible deny authority).
- A **claim snapshot** (deterministic read model).
- An **object store** (canonical bytes, SHA-256 addressed).

Markdown and Git still represent authored document content with full audit
history. They are not, however, the authority for claim state. See
`references/architecture.md`.

## When to Use What (Quick Router)

| Intent | Tool | Notes |
|--------|------|-------|
| Read operator constitution | `profile_get` | Loaded every session |
| Discover pages by keyword | `wiki_search` | BM25, fast |
| Discover concepts semantically | `semantic_search` | Hybrid when vector tier is on; else BM25 alias |
| Read a specific page | `wiki_content_read` or `semantic_get` | Use slug or `wiki://` URI |
| Look up confirmed claims | `brain_search`, `brain_get` | Queries the semantic brain |
| Capture a user utterance as a claim | `brain_capture` → `brain_confirm` | Two-step, idempotent |
| Propose an AI-derived claim | `brain_propose` → `brain_confirm` | Always status `proposed`, never auto-confirmed |
| Replace a stale claim | `brain_supersede` | Pass superseded confirm op ids |
| Ingest an external source | `brain_ingest_source` → `brain_extract` | Quarantine chunks, then extract claims |
| Write a durable Markdown page | `wiki_content_write` → `wiki_ingest` | Schema-validates + indexes + commits |
| Inspect graph neighbors | `graph_neighbors` | Single hop; use `wiki_graph` for full subgraph |
| Audit a page | `audit_history`, `wiki_history` | Git + ledger trail |
| Check system health | `brain_status`, `wiki_index_status`, `wiki_stats` | Always cheap, always safe |

For the full 39-tool matrix with arguments, tiers, examples, and gotchas,
see `references/tool-reference.md`.

## Session Bootstrap (mandatory first calls)

Start every working session by running, in order:

1. `wiki_spaces_list` — confirm the brain wiki is mounted and is the default.
2. `wiki_content_read(uri: "entities/brain-instance")` — read this
   instance's deployment descriptor if one exists (shape, endpoint,
   restart command, config-writability). If it doesn't exist yet, use the
   detection heuristic in `references/deployment-shapes.md` and propose
   creating one once you've determined the shape.
3. `profile_get` — load the operator constitution (identity, hard rules,
   style, stack, constraints) into context. **If this returns an empty
   page list**, the profile has never been seeded on this instance —
   don't silently proceed as if there are no rules; surface it and offer
   to seed `profile/` from whatever source the operator has (e.g. an
   existing CLAUDE.md).
4. `brain_status` — confirm ledger head, claim count, and schema version
   look healthy.
5. `wiki_index_status` for the relevant wiki if later `wiki_search` /
   `wiki_list` results look stale.

Skip step 1–4 only when the user has explicitly narrowed the task to a
single, well-scoped operation that does not touch memory or profile.

## Write Discipline

Any write that touches durable memory follows this shape:

```text
read current state
  → propose / capture (returns diff + confirm token / proposal op id)
    → user confirms OR user says "save this"
      → confirm / commit / supersede (returns new state + git sha / event seq)
        → verify by reading back
```

For Markdown content the flow is:

1. `wiki_content_write` (or `wiki_content_new` for scaffold) — writes the file.
2. `wiki_ingest` — validates frontmatter against the type schema, updates the
   search index, and commits to git when `ingest.auto_commit` is true.
   Use `redact: true` when the body may contain secrets.

For semantic claims the flow is:

1. `brain_capture` (user utterance) or `brain_propose` (AI-derived).
   Both return a proposal operation id and leave the claim in `proposed`.
2. `brain_confirm` to promote to confirmed, OR
3. `brain_supersede` to replace an existing confirmed claim with a new one.

Never bypass these flows by editing Markdown directly when a ledger-level
claim is what you actually intend to change.

## Content Placement

Use the README layout unless a local wiki defines a different convention:

```text
profile/
  identity.md
  hard-rules.md
  soft-preferences.md
  style-guide.md
  stack.md
  constraints.md

concepts/
entities/
sources/
projects/
decisions/

procedural/
  deployment/
  development/
  troubleshooting/
```

Type carries the epistemic distinction. Folder is for human organization
only. For the full type → schema → store mapping and the acid test that
separates semantic from procedural, see `references/type-system.md`.

## Frontmatter Standards

Keep frontmatter valid YAML, schema-compatible, and explicit. Prefer
explicit `title`, `type`, and `status`. Two canonical examples:

Profile (constitution):

```markdown
---
title: "Hard Rules"
type: profile
section: rules
priority: hard
status: active
created: 2026-05-23
last_verified: 2026-05-23
---
```

Procedure (executable runbook; verification is mandatory):

```markdown
---
title: "Deploy brain-mcp"
type: procedure
status: draft
verified_count: 0
failure_count: 0
verification:
  - "llm-wiki --version exits 0"
  - "MCP client can list tools"
risk_level: medium
tags: [deployment, mcp]
---
```

For the full per-type frontmatter reference, see
`references/type-system.md`.

## CLI Fallback

Use the CLI for setup, repair, ops, and any task without an MCP tool.
**Exact commands depend on the deployment shape** (local process, Docker
compose, or systemd bare-metal) — see `references/deployment-shapes.md`
before running anything below on a host you haven't confirmed the shape
of. The commands here are the Shape A (local dev) form:

```bash
llm-wiki spaces create ~/wikis/brain --name brain --set-default
llm-wiki serve                                      # stdio MCP
llm-wiki serve --http :47778                        # HTTP MCP (endpoint: /mcp)
llm-wiki serve --http :47778 --watch --web --web-port 1414 --web-bind 0.0.0.0
llm-wiki index rebuild --wiki brain                 # rebuild tantivy
llm-wiki web install --wiki brain --force           # rebuild Hugo mirror
```

CLI to MCP mapping is provided in `references/tool-reference.md`. Per-shape
start/restart/upgrade/backup commands and the Docker `:ro`-config gotcha
are in `references/deployment-shapes.md`. Deep Shape C topology (Oracle
Cloud Always Free, systemd, Tailscale) is in `references/workflows/deployer.md`.

## Role Playbooks

Deep, scenario-driven walkthroughs for each audience live in
`references/workflows/`:

- **Operator** (`operator.md`) — read profile, search knowledge, capture
  claims, ingest sources, crystallize sessions. The day-to-day user.
- **Developer** (`developer.md`) — navigate the codebase, add a new tool,
  run tests, work with CodeGraph. Anyone contributing to `brain-mcp`.
- **Deployer** (`deployer.md`) — Oracle VM provisioning, systemd units,
  MCP client config (Claude Desktop, Codex, Cursor, Zed), backups,
  troubleshooting.

When the task clearly maps to one role, read the matching playbook first.
When unclear, start with the operator playbook.

## Reference Index

| Reference | When to read |
|-----------|--------------|
| `references/architecture.md` | You need the why: 3 stores, event-ledger authority, epistemic model |
| `references/tool-reference.md` | You need the how: 39-tool matrix with args, tiers, examples |
| `references/type-system.md` | You are deciding which type a new page or claim should be |
| `references/anti-patterns.md` | You are about to do something risky; check before acting |
| `references/deployment-shapes.md` | You need to know how THIS host runs brain-mcp before running a command |
| `references/workflows/operator.md` | Day-to-day memory and knowledge tasks |
| `references/workflows/developer.md` | Contributing to brain-mcp source |
| `references/workflows/deployer.md` | Deep Shape C (systemd bare-metal) setup/repair walkthrough |

## Troubleshooting (First Response)

- MCP client cannot list tools → use absolute binary path, restart client,
  run `llm-wiki index rebuild --wiki brain`, run `llm-wiki serve` to see
  startup errors.
- Remote clients get `ECONNREFUSED` → check the process is actually up
  for this shape (`systemctl status brain-mcp` on Shape C, `docker ps` /
  `docker compose logs -f brain` on Shape B), then
  `ss -ltnp | grep -E ':47778|:8080|:1414'`.
- A write tool (e.g. `wiki_spaces_create`) fails with
  `Read-only file system (os error 30)` → you're on Shape B (Docker
  compose) with a `:ro`-mounted config. See
  `references/deployment-shapes.md` § EROFS for the workaround — do not
  remount config as writable.
- Web UI opens but pages are missing → do **not** edit `site/content/`;
  run `llm-wiki web install --wiki brain --force` then restart the
  service for this shape (see `references/deployment-shapes.md`).
- `brain_status` shows degraded schema version → do not write; read
  `references/architecture.md` and escalate before any mutation.
- The web console (Shape B/C with the bundled Svelte SPA) is reachable at
  the same host/port as `/mcp` — e.g. `http://127.0.0.1:8080/` — login
  uses the same bootstrap secret. Useful for a human to eyeball the inbox
  review queue or the galaxy graph without going through MCP calls.

For a deeper install/deploy reference, read the brain-mcp repo's `README.md`
and `BLUEPRINT.md`. When this skill and the repo disagree, the repo wins.
