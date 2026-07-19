# brain-mcp Type System

Audience: an agent or author deciding which `type` a new page or claim
should be. This is the decision layer. For the why behind the stores see
`architecture.md`; for per-tool arguments see `tool-reference.md`; for risky
patterns see `anti-patterns.md`.

Every field, status enum, and graph edge below is taken from the schema
files in `schemas/` of the brain-mcp repo. Where BLUEPRINT and schemas
disagree, the schemas win (they are what validates frontmatter on
`wiki_ingest`).

---

## Why Type, Not Folder

From `docs/specifications/model/epistemic-model.md`: a cooking wiki wants
`recipes/` and `techniques/`, not `concepts/`; if folder = epistemic role
then type and folder say the same thing (redundant) and the engine must
enforce folder-type coupling (rigid).

The `type` field is the epistemic axis. The physical layers
(`inbox/` -> `raw/` -> `wiki/`) are structural, not epistemic. The wiki
owner picks folders for human organization; the engine reads `type` for
validation, indexing, and graph edges. So `wiki_list --type concept`,
`wiki_search --type paper`, and `wiki_graph` (which reads edge fields
declared per type in `x-graph-edges`) all work regardless of folder.

Pick the type first; pick the folder for the humans who browse the tree.

---

## The Type -> Store -> Flow Decision Tree

Walk top-down. Stop at the first match.

```text
1. Operator constitution (identity/rules/style/stack/constraints)?
   -> type=profile  | Store: Profile | wiki_content_write -> wiki_ingest
      Only interactive Desktop-style clients may propose profile updates.

2. Executable workflow with a verification step?  (Apply the acid test below.)
   All three YES -> type=procedure  | Store: Procedural
      wiki_content_write (status: draft) -> wiki_ingest
      Promote to verified after >=3 successful runs of verification.

3. Synthesized knowledge ("what do we know about X?")?
   -> type=concept       | Semantic | schema: concept.json

4. Saved conclusion drawn at a specific time (frozen provenance)?
   -> type=query-result  | Semantic | schema: concept.json

5. Person, organization, product, or system?
   -> type=entity        | Semantic | schema: semantic.json

6. Durable project facts/constraints?
   -> type=project       | Semantic | schema: semantic.json

7. ADR-style architectural decision?
   -> type=decision      | Semantic | schema: semantic.json

8. External document someone else produced? Pick by nature of the material:
   paper | article | documentation | clipping | transcript | note
   | data | book-chapter | thread     (all schema: paper.json; Semantic)

9. Agent skill with workflow instructions?
   -> type=skill         | Extension | schema: skill.json (name/description)

10. Reference document (spec, guide, standard, policy)?
    -> type=doc          | Extension | schema: doc.json

11. Directory index grouping related pages?
    -> type=section      | Extension | schema: section.json
       Excluded from search by default.

12. None of the above -> default (base.json); frontmatter must have title+type.
```

### The acid test (BLUEPRINT 2.4)

Applied at step 2:

| # | Question | Procedural | Semantic |
|---|---|:---:|:---:|
| Q1 | Can a junior copy-paste and execute it as-is? | yes | no |
| Q2 | Is there a pass/fail verification step? | yes | no |
| Q3 | Is "did this succeed?" answerable as a boolean? | yes | no |

All three yes -> Procedural. Any no -> Semantic. Worked examples below.

---

## Type Catalog

Schema field columns mean: R = required by the schema's `required` array.
Optional fields list the most useful ones, not every property.

### Profile

| Aspect | Value |
|---|---|
| Type | `profile` |
| Schema file | `schemas/profile.json` |
| Required fields | `title`, `type` (const "profile"), `section`, `priority`, `status` |
| Optional fields | `supersedes`, `created`, `last_verified`, `tags` |
| When to use | Operator constitution loaded every session. |
| When NOT to use | Project facts, concepts, decisions - those go in semantic types. Project preferences go in `profile/soft-preferences.md` only if global operator preferences, not project-scoped. |
| Write flow | `wiki_content_write` -> `wiki_ingest`. Supersession: old `status: superseded`, new `status: active` + `supersedes: <old-slug>`. |

Canonical frontmatter:

```yaml
---
title: "Hard Rules"
type: profile
section: rules         # rules | identity | style | stack | constraints
priority: hard         # hard | soft
status: active         # active | superseded
created: 2026-05-23
last_verified: 2026-07-19
tags: [commit-policy, language]
---
```

### Semantic - knowledge (schema: concept.json)

| Aspect | Value |
|---|---|
| Types served | `concept`, `query-result` |
| Schema file | `schemas/concept.json` |
| Store | Semantic |
| Required fields | `title`, `type`, `read_when` |
| Optional fields | `summary`, `status`, `last_updated`, `tags`, `owner`, `superseded_by`, `tldr`, `sources`, `concepts`, `confidence`, `claims` |
| `status` enum | `active`, `draft`, `stub`, `generated` |
| `confidence` | 0.0 to 1.0 (default 0.5) |
| `claims[].text` required | Yes (when using the structured `claims` array) |
| When to use `concept` | The wiki's current synthesized understanding of a topic. One concept per page. Continuously enriched across sources. |
| When to use `query-result` | A conclusion saved at a specific time for a specific question, with its source slugs. |
| When NOT to use | A specific source's claims -> use a source type. An executable workflow -> use `procedure`. |

Canonical `concept`:

```yaml
---
title: "Reciprocal Rank Fusion"
type: concept
summary: "Fuse multiple ranked lists into one using 1/(k+rank) scoring."
read_when: ["Building a hybrid retrieval pipeline",
            "Fusing BM25 and dense vector rankings"]
status: active
confidence: 0.9
tags: [retrieval, ranking, fusion]
sources: [sources/cormack-2009-rrf]
concepts: [concepts/hybrid-search]
---
```

Canonical `query-result`:

```yaml
---
title: "Does MoE scale efficiently on ARM?"
type: query-result
summary: "Yes for inference latency, with caveats on memory."
read_when: ["Choosing a model architecture for ARM inference"]
status: active
confidence: 0.7
sources: [sources/moe-scaling-paper, sources/arm-inference-bench]
concepts: [concepts/mixture-of-experts]
---
```

### Semantic - entities, projects, decisions (schema: semantic.json)

| Aspect | Value |
|---|---|
| Types served | `entity`, `source`, `project`, `decision` |
| Schema file | `schemas/semantic.json` |
| Required fields | `title`, `type` (one of the four enum values) |
| Optional fields | `summary`, `status`, `confidence`, `tags`, `sources`, `related`, `supersedes`, `last_lint`, `content_hash`, `embedding_version` |
| `status` enum | `active`, `draft`, `stale`, `contested` |
| `confidence` | 0.0 to 1.0 (default 0.5) |
| When to use `entity` | A person, organization, product, or system (e.g., Qdrant, Anthropic). |
| When to use `source` | A semantic source summary that is not one of the nine paper.json source types (rare; prefer the paper.json source type that matches the material). |
| When to use `project` | Durable project facts and constraints (e.g., brain-mcp's deployment target). |
| When to use `decision` | An ADR-style record. |
| When NOT to use | Synthesized knowledge across many sources -> use `concept`. |

Canonical `entity` / `decision` (same schema):

```yaml
# entity
---
title: "Qdrant"
type: entity
summary: "Rust vector database with payload filter, HNSW, and scalar quantization."
status: active
confidence: 0.9
tags: [vector-db, rust, infrastructure]
sources: [sources/qdrant-docs]
related: [concepts/hybrid-search]
content_hash: sha256:abcd1234...
embedding_version: bge-m3-v1
---

# decision (same schema, type swapped)
---
title: "ADR-001: Choose Qdrant over pgvector"
type: decision
summary: "Payload filter + ARM build + HNSW tuning justify a separate service."
status: active
confidence: 0.9
sources: [sources/qdrant-bench, sources/pgvector-bench]
related: [projects/brain-mcp]
---
```

### Semantic - source types (schema: paper.json)

All nine types share one schema and the same status enum. Pick by the
**nature of the source material**, not its topic. A blog post about academic
research is `article`, not `paper`.

| Aspect | Value |
|---|---|
| Types served | `paper`, `article`, `documentation`, `clipping`, `transcript`, `note`, `data`, `book-chapter`, `thread` |
| Schema file | `schemas/paper.json` |
| Store | Semantic (source provenance) |
| Required fields | `title`, `type` |
| Optional fields | `summary`, `status`, `last_updated`, `tags`, `owner`, `superseded_by`, `read_when`, `tldr`, `sources`, `concepts`, `confidence`, `claims` |
| `status` enum | `active`, `draft`, `stub`, `generated` |
| Graph edges | `sources` -> `cites`; `concepts` -> `informs`; `superseded_by` -> `superseded-by` |

Type selection guide:

| Type | Use for | Do NOT use for |
|---|---|---|
| `paper` | Academic papers, preprints | Blog posts about research |
| `article` | Blog posts, news, essays | Academic publications |
| `documentation` | Product docs, API references | Tutorials as blog posts |
| `clipping` | Browser clips, bookmarks | Long-form reading you summarized |
| `transcript` | Meeting transcripts, podcasts | Notes taken during a meeting |
| `note` | Informal drafts, quick captures | Anything you would publish |
| `data` | CSV, JSON, datasets | Prose that contains numbers |
| `book-chapter` | Book excerpts | Articles |
| `thread` | Forum threads, social media | Single blog post |

Canonical `paper`:

```yaml
---
title: "Reciprocal Rank Fusion (Cormack et al., 2009)"
type: paper
summary: "Original RRF paper showing k=60 is a robust default."
status: active
confidence: 0.95
tags: [retrieval, rrf, ranking]
concepts: [concepts/reciprocal-rank-fusion]
tldr: "RRF with k=60 outperforms learned fusion on TREC."
claims:
  - {text: "RRF score for d is sum of 1/(k+rank_i(d)).", confidence: 0.99, section: "Section 3"}
---
```

### Procedural

| Aspect | Value |
|---|---|
| Type | `procedure` |
| Schema file | `schemas/procedure.json` |
| Store | Procedural |
| Required fields | `title`, `type` (const "procedure"), `status`, `verification` |
| Optional fields | `verified_count`, `last_verified`, `last_failed`, `failure_count`, `inputs`, `outputs`, `preconditions`, `postconditions`, `rollback`, `tags`, `related_procedures`, `estimated_duration`, `risk_level` |
| `status` enum | `verified`, `draft`, `deprecated`, `needs_review` |
| `risk_level` enum | `low`, `medium`, `high` |
| `verification` | Required array of pass/fail strings. Mandatory before promotion. |
| When to use | Executable workflows with a clear pass/fail. |
| When NOT to use | Conceptual explanations ("what is deployment?") - use `concept`. |

Promotion gate (BLUEPRINT 2.3): `draft` -> `verified` needs the verification
block to pass >= 3 times. Demotion: `failure_count` >= 2 in 30 days triggers
`deprecated`. After a failed run, set `status: needs_review`, bump
`failure_count`, set `last_failed`.

Canonical `procedure`:

```yaml
---
title: "Deploy brain-mcp to Oracle VM"
type: procedure
status: draft                 # verified | draft | deprecated | needs_review
verified_count: 0
failure_count: 0
inputs:
  - {name: vm_ip, type: ipv4}
  - {name: ssh_key_path, type: filepath}
outputs: [mcp_endpoint_url]
preconditions:  ["VM running Ubuntu 24.04 ARM", "SSH access established"]
postconditions: ["MCP server responds on :8765"]
verification:                       # REQUIRED - pass/fail checks
  - "curl http://{vm_ip}:8765/health returns 200"
  - "mcp_list_tools returns >= 10 tools"
  - "write test profile -> read back -> bytewise equal"
rollback: ["systemctl stop brain-mcp", "git revert HEAD on the wiki repo"]
related_procedures: [procedural/deployment/setup-qdrant]
estimated_duration: 15min
risk_level: medium
tags: [deployment, mcp, oracle]
---

## Steps
1. ...

## Verification
- [ ] curl ...
- [ ] mcp_list_tools ...

## Failure Modes
- If step 3 fails with "permission denied" -> ...

## Rollback
1. ...
```

### Extensions - skill, doc, section

| Type | Schema | Required | Optional highlights | `status` enum | Use |
|---|---|---|---|---|---|
| `skill` | `skill.json` | `name`, `description`, `type` (const "skill") | `when_to_use`, `argument-hint`, `paths`, `disable-model-invocation`, `user-invocable`, `allowed-tools`, `context`, `agent`, `model`, `effort`, `shell`, `hooks`, `status`, `tags`, `owner`, `superseded_by`, `document_refs`, `compatibility`, `license`, `metadata` | `active`, `draft`, `stub`, `generated` | Agent skill with workflow instructions. Uses `name`/`description` (aliased to `title`/`summary`). |
| `doc` | `doc.json` | `title`, `type` | `summary`, `status`, `last_updated`, `tags`, `owner`, `superseded_by`, `read_when`, `sources` | `active`, `draft`, `stub`, `generated` | Reference document: specifications, guides, standards, policies. |
| `section` | `section.json` | `title`, `type` | `summary`, `status`, `last_updated`, `tags`, `owner`, `superseded_by` | `active`, `draft`, `stub`, `generated` | Directory index grouping related pages. Excluded from search results by default. |

Canonical `skill`:

```yaml
---
name: ingest-source
description: Use when ingesting an external source into the wiki as typed claims.
type: skill
when_to_use: "After reading a URL, paper, or doc the operator wants stored."
allowed-tools: [brain_ingest_source, brain_extract, brain_confirm]
user-invocable: true
disable-model-invocation: false
effort: medium
status: active
tags: [ingest, capture]
document_refs: [docs/ingest-reference]
---
```

Canonical `doc`:

```yaml
---
title: "brain-mcp Operator Guide"
type: doc
summary: "Day-to-day memory and knowledge tasks."
status: active
read_when: ["Operating brain-mcp as the day-to-day user"]
tags: [guide, operator]
sources: [sources/brain-mcp-blueprint]
---
```

Canonical `section` (lives at `some/dir/index.md`):

```yaml
---
title: "Retrieval Concepts"
type: section
summary: "Concepts covering hybrid retrieval, fusion, and reranking."
status: active
---
```

---

## Status Lifecycle per Type

Each schema pins its own `status` enum. Use exactly these values.

| Type(s) | Schema | `status` enum | Notes |
|---|---|---|---|
| `profile` | `profile.json` | `active`, `superseded` | Supersession via `supersedes` field. No time-based decay. |
| `concept`, `query-result` | `concept.json` | `active`, `draft`, `stub`, `generated` | `stub` = placeholder; `generated` = AI-produced draft pending review. |
| `entity`, `source`, `project`, `decision` | `semantic.json` | `active`, `draft`, `stale`, `contested` | `stale`: all sources older than 12 months AND no inbound link in 6 months. `contested`: contradiction detected, human review required, never auto-resolved. |
| `paper`, `article`, `documentation`, `clipping`, `transcript`, `note`, `data`, `book-chapter`, `thread` | `paper.json` | `active`, `draft`, `stub`, `generated` | Same lifecycle as concept/query-result. |
| `procedure` | `procedure.json` | `verified`, `draft`, `deprecated`, `needs_review` | Promotion requires verification block pass >= 3 times. Demotion triggers: `failure_count` >= 2 in 30 days. |
| `skill` | `skill.json` | `active`, `draft`, `stub`, `generated` | Deprecate by setting `superseded_by`. |
| `doc` | `doc.json` | `active`, `draft`, `stub`, `generated` | Deprecate by setting `superseded_by`. |
| `section` | `section.json` | `active`, `draft`, `stub`, `generated` | Sections are excluded from search results by default. |

Schema-vs-BLUEPRINT note: BLUEPRINT 2.2 lists the semantic sub-types
(`concept`/`entity`/`source`/`project`/`decision`) as sharing one
`active/draft/stale/contested` enum, but the shipped schemas split them.
`concept` and `query-result` use `active/draft/stub/generated` via
`concept.json`; `entity`/`source`/`project`/`decision` use
`active/draft/stale/contested` via `semantic.json`. The schemas are the
source of truth. If a page uses `stale` or `contested`, it must be one of
the `semantic.json` types.

---

## Acid Test Worked Examples

Five short scenarios. Each shows the wrong-type trap and the correction.

| # | Utterance | Wrong | Right |
|---|---|---|---|
| 1 | "Remember that Qdrant supports payload filters." | `procedure` (nothing to execute); `paper` (not citing a paper) | **`concept`** with `sources: [sources/qdrant-docs]`, or **`entity`** if the page is specifically about Qdrant. |
| 2 | "Deploy brain-mcp step by step." | `concept`; `paper` | **`procedure`** with `verification` (`curl /health`, tool-count, write-read-back); `draft` until verified 3 times. |
| 3 | "Cormack 2009 says RRF with k=60 is robust." | `concept` (this is one source's claim) | **`paper`** with `claims: [{text, section}]` and `concepts: [concepts/reciprocal-rank-fusion]`; the concept page cites this paper via `sources:`. |
| 4 | "What did we conclude about ARM inference efficiency?" | `concept` (a concept is the live view, not a frozen conclusion) | **`query-result`** with `sources:` and `concepts:` populated so the conclusion is auditable. |
| 5 | "Anthropic builds Claude." | `concept` (an actor, not a synthesized principle) | **`entity`** with `related: [concepts/claude, concepts/mcp]`. |

---

## Field Aliasing and Graph Edges

### Field aliasing - `x-index-aliases`

Different types use different field names for the same role. The engine maps
them to canonical index fields at ingest time. The file on disk is never
rewritten. From `skill.json`:

```json
"x-index-aliases": { "name": "title", "description": "summary", "when_to_use": "read_when" }
```

A `type: skill` page with `name: ingest-source` is indexed under the
canonical `title` field; `wiki_search --type skill "ingest"` works the same
as a concept search on `title`. If both alias source and canonical are
present, the canonical wins. Unaliased fields are indexed as generic text.

### Typed graph edges - `x-graph-edges`

Each schema declares outgoing edges that become labeled links in the
Petgraph projection; `wiki_graph` and `graph_neighbors` filter on them.
From `concept.json`:

```json
"x-graph-edges": {
  "sources":       { "relation": "fed-by",        "direction": "outgoing",
                     "target_types": ["paper","article","documentation","clipping",
                                      "transcript","note","data","book-chapter","thread"] },
  "concepts":      { "relation": "depends-on",    "direction": "outgoing", "target_types": ["concept"] },
  "superseded_by": { "relation": "superseded-by", "direction": "outgoing" }
}
```

Edge summary by type:

| Type(s) | Edge field | Relation |
|---|---|---|
| `profile` | `supersedes` | `supersedes` (target: `profile`) |
| `concept`, `query-result` | `sources` / `concepts` / `superseded_by` | `fed-by` / `depends-on` / `superseded-by` |
| `entity`, `source`, `project`, `decision` | `sources` / `related` / `supersedes` | `supported-by` / `related` / `supersedes` |
| Source types (paper.json) | `sources` / `concepts` / `superseded_by` | `cites` / `informs` / `superseded-by` |
| `procedure` | `related_procedures` | `related-procedure` (target: `procedure`) |
| `skill` | `document_refs` / `superseded_by` | `documented-by` (target: `doc`) / `superseded-by` |
| `doc` | `sources` / `superseded_by` | `informed-by` / `superseded-by` |
| `section` | (none) | (none) |

Body `[[wiki-links]]` always get the generic `links-to` relation regardless of
type.

---

## Custom Types

The engine does not need to know what your type means. It validates against
the schema and indexes using the alias mapping.

**Option A: drop a schema in `schemas/`** (preferred):

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "x-wiki-types": { "meeting-notes": "Meeting notes with attendees and action items" },
  "type": "object",
  "required": ["title", "type"],
  "properties": {
    "title":        { "type": "string" },
    "type":         { "type": "string" },
    "attendees":    { "type": "array", "items": { "type": "string" } },
    "action_items": { "type": "array", "items": { "type": "string" } }
  },
  "additionalProperties": true
}
```

Save as `schemas/meeting-notes.json`. The engine scans `schemas/*.json` and
auto-registers the type via `x-wiki-types`.

**Option B: `[types.*]` override in `wiki.toml`** - use to remap an existing
type to a different schema file:

```toml
[types.meeting-notes]
schema = "schemas/meeting-notes.json"
description = "Meeting notes with attendees and action items"
```

**Resolution order:** scan `schemas/*.json` -> read `[types.*]` from
`wiki.toml` -> `wiki.toml` wins -> merged registry.

**Base schema invariant.** The `default` type (from `base.json`) is the
fallback for every unknown or missing `type`. The engine enforces:

1. `default` always exists; if no schema declares it, the embedded
   `base.json` is used.
2. A custom `base.json` must declare `default` in `x-wiki-types`.
3. A custom `base.json` must require at least `title` and `type` (superset
   OK; cannot drop either).
4. Violations make `SpaceTypeRegistry::build()` return an error.

Pages without a `type` field default to `type: page` and validate against
`[types.default]`.

---

## Common Mistakes

High-frequency wrong-type traps. See `anti-patterns.md` for the full risk
catalog before destructive actions.

| # | Mistake | Fix |
|---|---|---|
| 1 | "Deploy brain-mcp" stored as `concept`. | It is a `procedure`. Acid test all yes. Move to `procedural/`, add `verification`. |
| 2 | `paper` page paraphrasing what "we believe" about RRF. | That is a `concept`. `paper` records what Cormack 2009 *claims*; `concept` synthesizes and cites via `sources:`. |
| 3 | "Does MoE scale on ARM?" baked into a `concept` as timeless. | Split: `concept: mixture-of-experts` (synthesis) + `query-result` (frozen conclusion with provenance). |
| 4 | "Qdrant" page of type `concept`. | Use `entity` for systems; reserve `concept` for synthesized knowledge that may outlive any vendor. |
| 5 | Blog post about a paper filed as `paper`. | Classify by the source material's nature, not topic. Blog post about research is `article`. |
| 6 | `status: verified`, `verified_count: 0`. | Promotion needs the verification block to pass >= 3 times. Leave `draft` or set `needs_review` after a failure. |
| 7 | `some/dir/page.md` exists but `some/dir/index.md` does not. | Add `index.md` with `type: section` + one-line `summary`, or move the page under a folder that has one. |

---

## Reference Index

| Reference | When to read |
|---|---|
| `references/architecture.md` | Why: three stores, event-ledger authority, epistemic model. |
| `references/tool-reference.md` | How: the 39-tool matrix. |
| `references/anti-patterns.md` | Before any destructive or risky operation. |
| `references/deployment-shapes.md` | Which deployment shape a given host is running. |
| `references/workflows/{operator,developer,deployer}.md` | Role playbooks. |

When this skill and the repo disagree, the repo wins.
