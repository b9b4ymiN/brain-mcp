# brain-mcp Architecture

Audience: any agent or human who needs the WHY before doing anything
non-trivial. For per-tool arguments see `tool-reference.md`; for type choice
see `type-system.md`; for risky patterns see `anti-patterns.md`.

Every claim below is traceable to `BLUEPRINT.md`,
`docs/adr/0001-semantic-authority-time-privacy.md`, `docs/overview.md`,
`docs/specifications/model/*.md`, or `schemas/*.json` in the brain-mcp repo.
Where the BLUEPRINT names a tool that is not wired into the running MCP
server, this file says so.

---

## At a Glance

brain-mcp is a long-term knowledge engine for agents, built on a Rust binary
(crate `llm-wiki-engine`, binary `llm-wiki`) that turns a folder of Markdown
plus an append-only event ledger into a searchable, auditable, recoverable
memory. Three stores sit on top of that engine: **Profile** (operator
constitution loaded every session), **Semantic** (declarative knowledge:
concepts, entities, sources, projects, decisions), and **Procedural**
(executable runbooks with explicit verification). The "vnext" phase adds an
event-ledger authority layer on top of the original Markdown+Git foundation
so every semantic transition is replayable and every privacy purge is
irreversible and provable.

```
   Clients (MCP / ACP / CLI): Claude Desktop, Claude Code, Codex, Zed, Cursor
                              | MCP over stdio or HTTP
                              v
            +-----------------------------------+
            |       brain-mcp gateway           | policy, permission,
            | Propose-Commit gate / Audit /     | audit, hybrid
            | Hybrid Retrieval                  | retrieval
            +--+-----------------------------+--+
               |                             |
               v                             v
     +-----------------+          +------------------+  (vnext, ADR-01)
     | llm-wiki engine |          | Authority layer  |
     | Tantivy libgit2 |          | Event ledger     |
     | Petgraph Comrak |          | Purge registry   |
     +--------+--------+          | Claim snapshot   |
              |                   | Object store     |
              +--------+----------+------------------+
                       |
                       v
     +-------------------------------------------+
     | Canonical filesystem ~/wikis/brain (git)  |
     | profile/ concepts/ entities/ sources/     |
     | projects/ decisions/ procedural/          |
     | schemas/ inbox/ raw/ + .git authored layer|
     +---------------------+---------------------+
                           |
                           v
     +-------------------------------------------+
     | Derived projections (rebuildable):        |
     | Tantivy index, Petgraph, vector, web UI   |
     +-------------------------------------------+
```

Rule of thumb: Markdown + Git is the authored document layer; the event
ledger is the sole authority for claim state. Every projection is
rebuildable. Nothing downstream is a source of truth.

---

## The Three Stores

Three stores, each with its own schema, lifecycle, and write discipline. The
`type` field on each page distinguishes them; folders are organizational only.

### Profile (Constitution)

| Aspect | Value |
|---|---|
| Purpose | Operator identity, hard rules, soft preferences, style, stack, constraints. Loaded every session. |
| Lifecycle | NO time-based decay. Supersession via `supersedes`. Versioned through git. |
| Page type | `profile` (schema: `schemas/profile.json`) |
| Storage layout | `profile/{identity,hard-rules,soft-preferences,style-guide,stack,constraints}.md` |
| Size budget | 1-3 KB total. Loads in <100 ms. Cached ~5 minutes. |
| Indexing | Not in the vector index. Scanned directly to keep latency low and avoid stale-index risk. |
| Canonical example | "Hard rule: no emoji in commit messages." |

Profile `section` enum (`profile.json`): `rules`, `identity`, `style`,
`stack`, `constraints`. `priority`: `hard` or `soft`. `status`: `active` or
`superseded`. Profile changes are the most privileged writes (BLUEPRINT 7).

### Semantic (Concept Wiki)

| Aspect | Value |
|---|---|
| Purpose | Declarative knowledge. "What is X?", "Why does Y matter?", "How do Z and W relate?" |
| Lifecycle | No hard delete. Stale flag when all sources older than 12 months AND no inbound link in 6 months. Contradictions flagged `contested` for human review; never auto-resolved. |
| Page types | `concept`, `query-result` (schema: `schemas/concept.json`); `entity`, `source`, `project`, `decision` (schema: `schemas/semantic.json`) |
| Storage layout | `semantic/{concepts,entities,sources,projects,decisions}/...` (or `concepts/`, `entities/` etc. at the wiki root) |
| Size budget | Scales to 10,000+ pages. Chunked at 512 tokens with 64-token overlap for embedding. |
| Indexing | BM25 (Tantivy) plus optional dense + sparse vectors and cross-encoder rerank. |
| Canonical example | "Reciprocal Rank Fusion (RRF) with k=60 fuses BM25 and dense ranks." |

### Procedural (Runbook)

| Aspect | Value |
|---|---|
| Purpose | Executable workflows with explicit verification. "How to do X" plus how to prove it worked. |
| Lifecycle | Promotion: `draft` to `verified` requires the `verification` block to pass at least 3 times. Demotion: `failure_count` >= 2 in 30 days triggers `deprecated` and the procedure stops being recommended. NO time-based decay. |
| Page type | `procedure` (schema: `schemas/procedure.json`) |
| Storage layout | `procedural/{deployment,development,troubleshooting}/...` |
| Required field | `verification` (pass/fail checks; mandatory before promotion) |
| Other fields | `inputs`, `outputs`, `preconditions`, `postconditions`, `rollback`, `risk_level: low|medium|high`, `verified_count`, `failure_count`, `related_procedures` |
| Canonical example | "Deploy brain-mcp to Oracle VM" with curl health check + tool count check. |

The acid test that separates procedural from semantic (BLUEPRINT 2.4) is
covered with worked examples in `type-system.md`.

---

## Epistemic Model

The `type` field carries the epistemic axis. Folder structure is organizational
and decided by the wiki owner; the engine does not couple folders to types.

### The three epistemic roles

| Role | Type(s) | Answers |
|---|---|---|
| Synthesized knowledge | `concept` | "What do we know about X?" |
| Source provenance | `paper`, `article`, `documentation`, `clipping`, `transcript`, `note`, `data`, `book-chapter`, `thread` | "What does this specific source claim?" |
| Reasoning output | `query-result` | "What did we conclude at time T for question Q?" |

Additional semantic types round out the model: `entity` (people,
organizations, products, systems), `project` (durable project context), and
`decision` (ADR-style records). These are declarative like `concept` but
carry their own status lifecycle and graph edges (schema: `semantic.json`).

### Why separation matters

Mixing these roles collapses provenance. The failure modes (quoted from
`docs/specifications/model/epistemic-model.md`):

| Collapsed into one | Problem |
|---|---|
| Sources merged into concepts | Cannot ask "which source claims this?" - provenance lost |
| Query results merged into concepts | Conclusions presented as facts - reasoning not auditable |
| `raw/` indexed alongside pages | Unprocessed content pollutes search |

A concept page cites its sources via the `sources` field (graph edge
`fed-by`); a source page records what one document said; a query-result
traces back to both. The Petgraph projection makes these relationships
navigable via `graph_neighbors` and `wiki_graph`.

### Why type, not folder

With type as the axis, `wiki_list --type concept` and
`wiki_search --type paper` work regardless of where the file lives. The full
argument is in `docs/specifications/model/epistemic-model.md`.

---

## The vnext Authority Model

`docs/adr/0001-semantic-authority-time-privacy.md` freezes the authority
contracts that vnext adds on top of the Markdown+Git foundation. Read it
before any non-trivial write, schema change, or recovery operation.

### No dual source of truth

Each layer has exactly one kind of authority.

| Layer | Authority | Mutable? | Recovery rule |
|---|---|---:|---|
| Raw object store | Canonical bytes/evidence addressed by SHA-256 | No, except irreversible purge | Verify hash; never reconstruct bytes from a claim |
| Human-authored Markdown | Canonical authored document content with Git history | New revisions only | Git restores authored text, not claim state |
| Event ledger | Sole order and authority for semantic transitions | Append-only | Replay through a selected ledger head |
| Purge registry | Sole deny/decryption authority for purged IDs and key epochs | Monotonic append-only | Sync before any read/decrypt; stale/unavailable = fail closed |
| Claim snapshot | Deterministic semantic read model | Projector-only replacement | Rebuild from ledger head minus denied IDs at purge epoch |
| Generated wiki | Readable materialized view | Replaceable | Regenerate from claim snapshot and citations |
| Tantivy/Petgraph/vector | Search/graph projections | Replaceable | Delete and rebuild from canonical layers |
| AI output | Proposal material only | Ephemeral or proposal event | Never becomes confirmed memory without a policy event |

Consequence for agents: Markdown alone is not enough to reconstruct semantic
state. The event ledger is the claim authority. If `brain_status` reports a
degraded schema version or an unhealthy ledger head, stop writing and
escalate.

### Readable claim formula

The readable state at any moment is a pure function of three inputs:

```text
snapshot  = project(replay(events[1..ledger_head]), schema_version)
readable  = snapshot - purge_registry.denied_ids(purge_epoch)
checksum  = sha256(canonical(snapshot) || ledger_head || purge_epoch || schema_version)
```

Reading is therefore deterministic and reproducible. The purge registry is
synced from independent targets before any read or decrypt; a stale,
unavailable, or checksum-invalid registry keeps the service sealed
(fail closed).

### Atomic write topology (7-step normal flow)

One serialized semantic writer process per owner ledger. Multiple clients are
supported because all commands enter the same service inside one SQLite
`BEGIN IMMEDIATE` transaction.

```text
1. Validate authenticated authority, capability, schema, valid time,
   provenance, policy.
2. Normalize the request; calculate canonical request SHA-256.
3. Resolve (owner_id, client_id, operation_id) inside the transaction.
4. Stage object bytes off the canonical path; SHA-256, fsync, atomic rename.
5. Append exactly one ordered semantic event + transactional-outbox work
   in the SQLite WAL.
6. Commit; idempotent consumers then update Git/Markdown, Tantivy, Petgraph.
7. Return stored outcome. Replayed identical op -> same outcome;
   changed tool or payload -> IDEMPOTENCY_CONFLICT, no mutation.
```

SQLite, filesystem objects, Git, and indexes are NOT one transaction.
Staging cleanup handles objects that lose the transaction race; outbox replay
handles committed events whose projections lag or crash. This is why a write
can return success even if the Tantivy index has not yet caught up - and why
`wiki_index_rebuild` exists.

### Stable IDs

- **UUIDv7** lowercase canonical RFC 9562 text form is the server-generated
  stable identifier for owners, actors, server-registered clients, events,
  sources, renditions, evidence spans, entities, claims, proposals, jobs,
  and purge requests.
- **Raw object identity** is `sha256:<64 lowercase hex>` (content-derived).
- **`operation_id`** is client-supplied, opaque 1-128 byte UTF-8. Use UUIDv7
  or UUIDv4. The server never infers time or identity from it; this is the
  idempotency key.
- **`event_seq`** is a server-assigned 64-bit integer, monotonic within
  `owner_id`; the only semantic-order tie-breaker.
- External/provider IDs are aliases, never primary keys. IDs never contain
  slug, entity name, provider, filesystem path, email, or secret.

### Bitemporal semantics

Valid time uses half-open intervals `[valid_from, valid_to)`; both bounds
optional, empty/reversed invalid. A future-valid confirmed claim is
historical but not current until `valid_from`. Correcting a valid-time
boundary appends a correction or supersede event; the old event is never
edited.

### Retract vs irreversible purge

- **Retract / archive** (default deletion): appends a semantic event, removes
  the claim from current answers, preserves history and evidence, reversible
  by a later event.
- **Hard purge**: irreversible security operation. Requires `brain.purge`
  capability, owner re-authentication within the last 5 minutes, and a
  single-use confirmation nonce that expires after 60 seconds. The 7-stage
  saga is deny-first and idempotent: `requested` -> `registry_denied` ->
  `key_revoked` -> `live_deleted` -> `git_rewritten/index_rebuilt` ->
  `retention_pending` -> `completed`. After `registry_denied`, every
  read/decrypt/export/restore fails closed for the denied IDs even if later
  cleanup crashes.

This is why the rule "Irreversible operations require explicit user consent"
exists in `SKILL.md`.

### Why this matters

Silent writes are the number-one memory failure mode (BLUEPRINT ADR-006).
The ledger makes every semantic transition replayable and revocable; the
purge registry makes privacy destruction provable; the propose/confirm gate
makes every write visible as a diff before it becomes durable. This is why
the rule "Irreversible operations require explicit user consent" exists in
`SKILL.md`.

---

## Write Flow

Four flows. All follow the propose -> confirm shape from `SKILL.md`. Tool
names below are the ones wired into the running MCP server. BLUEPRINT 6 also
names aspirational tools (`memory_propose`, `memory_commit`,
`profile_propose_update`, `procedure_propose`, `procedure_promote`,
`procedure_demote`) that are design intent, not all wired yet. When the
BLUEPRINT and the live server disagree, the live server wins.

### Flow A: Profile write

Profile changes are the most privileged. Per BLUEPRINT 7, only the
interactive Desktop-style client may propose profile updates.

```text
1. profile_get  section=rules                          # read current
2. wiki_content_write uri=profile/hard-rules.md content=<new md+fm>
3. wiki_ingest     path=profile/hard-rules.md redact=false
4. wiki_content_read uri=profile/hard-rules.md         # verify
```

For supersession, set old page `status: superseded`, new page
`status: active` with `supersedes: <old-slug>`. The `supersedes` graph edge
(declared in `profile.json`) keeps the chain navigable.

### Flow B: Semantic claim

Two entry points depending on origin. Both leave the claim in `proposed`;
`brain_confirm` promotes; `brain_supersede` replaces.

User utterance:

```text
brain_capture
  operation_id=<uuidv7>
  utterance="Qdrant payload filters use keyword indexes, not SQL."
  subject="entity:qdrant"  predicate="filter_mechanism"
  value="keyword-payload"  domain="infra"
  claim_kind="user_assertion"
  -> proposal_operation_id

brain_confirm
  operation_id=<new uuidv7>
  proposal_operation_id=<from above>
```

AI-derived inference (always `status: proposed`, never auto-confirmed):

```text
brain_propose
  operation_id=<uuidv7>
  subject="concept:reciprocal-rank-fusion"  predicate="default_k"  value="60"
  domain="retrieval"  method="llm_extraction"  model="claude-sonnet-4"
  evidence_capture_operation_ids=<comma-separated chunk ids>
  -> proposal_operation_id

brain_confirm  operation_id=<new uuidv7>
               proposal_operation_id=<from above>
```

An inference with no evidence has `unsupported=true` and may only be proposed
or rejected - never confirmed. User acceptance creates a new user-assertion
or decision event rather than laundering an unsupported external fact
(ADR-0001 Decision 6).

Replacing a stale confirmed claim:

```text
brain_supersede
  operation_id=<uuidv7>
  proposal_operation_id=<new confirmed proposal op id>
  superseded_claim_operation_ids=<comma-separated existing confirm op ids>
```

Conflict policy: `latest-user-wins` applies only to `preference`, `profile`,
and `project decision` claims keyed by
`(owner_id, domain, subject_id, predicate, normalized_context)`. External
facts and observations never use latest-user-wins; conflicting credible
claims coexist as `disputed`.

### Flow C: Procedural write

Procedures are Markdown pages with `type: procedure`. They require a
`verification` block before promotion (mandatory per `procedure.json`).

```text
1. wiki_content_new  uri=procedural/deployment/deploy-brain-mcp  type=procedure
2. wiki_content_write uri=procedural/deployment/deploy-brain-mcp.md
     content = ---                        # minimal valid frontmatter:
       title: "Deploy brain-mcp"
       type: procedure
       status: draft                      # verified_count: 0, failure_count: 0
       risk_level: medium
       verification:                      # REQUIRED - pass/fail checks
         - "llm-wiki --version exits 0"
         - "MCP client can list tools"
         - "curl http://127.0.0.1:8765/health returns 200"
       rollback: [systemctl stop brain-mcp, git revert HEAD]
       ---
       ## Steps ...
3. wiki_ingest path=procedural/deployment/deploy-brain-mcp.md
4. After >=3 successful runs of the verification block, rewrite frontmatter
   to status: verified + verified_count: 3 + last_verified, then
   wiki_content_write + wiki_ingest again to commit the promotion.
```

BLUEPRINT's `procedure_propose` / `procedure_promote` / `procedure_demote`
are design intent; the live write path is `wiki_content_write` +
`wiki_ingest`, with verification evidence as the gate.

### Flow D: Ingest an external source

Used when an agent reads a URL, file, or text and wants to extract claims.

```text
1. brain_ingest_source                       # exactly one of url | file_path | text
     operation_id=<uuidv7>  url=<url>  max_chunk_bytes=4000
     -> quarantines one capture per paragraph-packed chunk
     -> returns chunk capture_operation_ids

2. brain_extract                             # one call per chunk (or batch)
     capture_operation_id=<chunk-N>  method="llm_extraction"
     model="claude-sonnet-4"  local_only=false
     -> proposes supported claims with evidence spans
     -> status is always "proposed"; never auto-confirmed

3. brain_confirm  (per claim the operator accepts)
```

`local_only=true` denies the AI provider call outright; use it when the chunk
must not leave the host.

---

## Recovery and Audit

### What is recoverable

| Layer | Recovery tool | Notes |
|---|---|---|
| Markdown content | `git revert`, `git checkout`, `wiki_history` | Restores authored text only. |
| Event ledger state | Ledger replay through any selected head | Deterministic. The authority. |
| Tantivy index | `wiki_index_rebuild` | Deletes and rebuilds from canonical Markdown. |
| Petgraph | Rebuilt from frontmatter on engine restart | Always derived. |
| Vector index | Re-embed from Markdown | BLUEPRINT DR drill target: <30 min at 1K pages. |
| Claim snapshot | Rebuild from ledger head minus denied IDs at purge epoch | Projector-only. |
| Generated wiki / Hugo site | `llm-wiki web install --wiki brain --force` | Always derived. |

### What is NOT recoverable

- **Purged bytes.** Hard purge destroys DEKs and rewrites Git history. After
  `registry_denied`, every read/decrypt fails closed; restore must first
  verify the newest purge registry from independent targets.
- **Claim state from Markdown alone.** Markdown carries authored document
  content, not the claim ledger.

### Audit tools

`audit_history` (path, limit), `wiki_history` (slug, optional `follow`),
`wiki_lint` (rules?), `brain_status`, `wiki_index_status`, `wiki_stats`.
The structured JSONL audit log records every MCP call; every git commit is
itself an audit entry. There is no unlogged write path.

---

## Performance Budget

From BLUEPRINT 11. If the source does not list a number, it is omitted.

| Operation | Target | Stretch | Hard limit |
|---|---|---|---|
| `profile_get` | <100 ms | <50 ms | 200 ms |
| `semantic_search` top-10 (1K pages) | <500 ms | <300 ms | 1000 ms |
| `semantic_search` top-10 (10K pages) | <800 ms | <500 ms | 1500 ms |
| `procedural_find` | <300 ms | <150 ms | 600 ms |
| `wiki_ingest` 1 page | <2 s | <1 s | 5 s |
| Full re-embed (1K pages) | <20 min | <10 min | 30 min |
| Consolidate dry-run (1K pages) | <5 min | <2 min | 10 min |

Operating rule: over stretch -> optimize. Over target -> investigate. Over
hard limit -> page.

BLUEPRINT 9 names Oracle Cloud Always Free on ARM Ampere
(VM.Standard.A1.Flex, 4 OCPU + 24 GB RAM + 200 GB block storage, Ubuntu 24.04
LTS ARM64) with systemd services as the reference production target
(no public ports; Tailscale or Cloudflare Tunnel only; daily git push;
weekly Qdrant snapshot to Object Storage). This is design intent, not a
claim about any specific running instance -- a given brain-mcp instance
may equally run as a local process or under Docker compose. See
`references/deployment-shapes.md` for the shapes this skill actually
operates against, and that instance's own `entities/brain-instance` page
for what is live right now.

---

## What It Is Not

Out of scope for v1 (BLUEPRINT 0.2, ADR-005, `docs/overview.md`):

- **Episodic chat recall** ("what did we say about X last week?"). The ledger
  is the authority for claim state, not conversation replay; may land in a v2
  append-only JSONL store.
- **Working memory or scratchpad.** That is a harness concern.
- **Real-time multi-user collaboration.** Single-operator design; one
  serialized writer per owner ledger.
- **Mobile or offline sync.** The wiki VM is the single source of truth.
- **Image or audio vectors.** Text only for v1; multi-modal ingest is v2.
- **An LLM, a RAG system, or a skill runtime.** The engine makes no AI calls
  and implements a Dynamic Knowledge Repository (DKR): knowledge is built at
  ingest time and compounds. Skills stored as `type: skill` pages are
  discoverable and readable; agents execute them.

---

## Further Reading

| Source | What it covers |
|---|---|
| `BLUEPRINT.md` (repo root) | Full design doc: stores, retrieval pipeline, deployment, ADRs 001-007, phased rollout. |
| `docs/adr/0001-semantic-authority-time-privacy.md` | vnext authority, identity, time, privacy, purge, authorization. |
| `docs/overview.md` | Engine-level overview: tools, type system, epistemic model, repository layout. |
| `docs/specifications/model/epistemic-model.md` | Why the type taxonomy exists. |
| `docs/specifications/model/type-system.md` | Type discovery, `x-wiki-types`, `x-index-aliases`, `x-graph-edges`, custom types. |
| `schemas/*.json` | The schemas: `profile`, `procedure`, `concept`, `semantic`, `paper`, `skill`, `doc`, `section`. |
| `references/type-system.md` | Per-type catalog with required fields, status enums, worked acid-test examples. |
| `references/tool-reference.md` | The 39-tool matrix. |
| `references/anti-patterns.md` | Risky patterns to avoid. |
| `references/deployment-shapes.md` | Which of the three deployment shapes a given host is running, and the per-shape commands. |
| `references/workflows/{operator,developer,deployer}.md` | Role playbooks. |

When this skill and the repo disagree, the repo wins.
