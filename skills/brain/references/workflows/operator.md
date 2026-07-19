# Operator Playbook

Audience: the **day-to-day user** of brain-mcp — the operator, or an
agent acting for them. The most-used playbook: read the constitution,
search knowledge, capture claims, ingest sources, write durable pages,
crystallize sessions.

Each scenario shows the exact MCP call sequence with realistic
arguments, the verify step, and watch-outs. Argument names match
`references/tool-reference.md` verbatim; on disagreement, the tool
reference wins. For the why read `references/architecture.md`; for
type choice `references/type-system.md`; for risky patterns
`references/anti-patterns.md`.

Three habits apply to every scenario:

1. **No silent writes.** Every durable mutation goes through propose →
   confirm. Even if the MCP client would auto-approve, surface the diff
   and wait for explicit "yes" before the confirm step.
2. **Read before write.** Call the matching read (`brain_get`,
   `profile_get`, `wiki_content_read`) **before** the propose step.
3. **`operation_id` is your idempotency key.** Fresh UUIDv7/UUIDv4 per
   logical operation. Same id + same payload replays safely; same id +
   different payload returns `IDEMPOTENCY_CONFLICT`, no mutation.

---

## Intent → Scenario Map

| You want to… | Scenario |
|---|---|
| Start a working session safely | [1](#1-session-bootstrap) |
| Read or refresh the operator constitution | [2](#2-read-the-operator-constitution) |
| Find knowledge by keyword, type, or claim | [3](#3-search-the-knowledge-base) |
| Read a specific page, claim, or procedure | [4](#4-read-a-specific-page-or-claim) |
| Remember something the user just said | [5](#5-capture-a-user-utterance-as-a-claim) |
| Record something the agent inferred | [6](#6-propose-an-ai-derived-claim) |
| Replace an outdated fact | [7](#7-supersede-a-stale-claim) |
| Turn a URL, file, or text into knowledge | [8](#8-ingest-an-external-source) |
| Write a durable Markdown page | [9](#9-write-a-durable-markdown-page) |
| Close a working session cleanly | [10](#10-crystallize-a-session) |
| Walk the graph or audit a page's history | [11](#11-audit-and-explore-the-graph) |
| Export the wiki for an LLM or CI | [12](#12-export-knowledge) |
| Refuse a risky request | [Anti-Patterns to Refuse](#anti-patterns-to-refuse) |

---

## 1. Session Bootstrap

**When to use.** At the start of every session, before any other call.
Skip only when the user has explicitly narrowed the task to a single,
well-scoped read that touches neither memory nor profile.

```
# 1. Confirm the brain wiki is mounted and is the default.
wiki_spaces_list()
#  -> look for an entry marked "*" (default). brain should be present.

# 2. Load the operator constitution into context.
profile_get()
#  -> identity, hard rules, soft preferences, style, stack, constraints.
#     <100ms target, cached ~5 minutes.

# 3. Confirm the semantic brain is healthy.
brain_status()
#  -> ledger head, claim count, schema version. If schema version is
#     degraded, STOP — do not write. Read architecture.md, escalate.

# Optional, only if later reads look stale:
wiki_index_status()
#  -> {stale, built}. If stale: true, run wiki_index_rebuild (tier 3).
```

**Verify.** `wiki_spaces_list` shows `brain` as default; `profile_get`
returns non-empty `rules` and `identity`; `brain_status` reports a
non-zero claim count on a used brain.

**Watch out.**
- `wiki_spaces_list` is classified write-additive by `annotations_for`
  even though it mutates nothing (conservative default for the
  space-management family; flagged in `tool-reference.md`). Treat it as
  read-only in practice.
- `brain_status` degraded schema is a stop signal. Never write against
  an unhealthy ledger.
- If no default wiki exists, do not silently create one — that is a
  deployer task (see `references/workflows/deployer.md`).

---

## 2. Read the Operator Constitution

**When to use.** Bootstrap; before any profile-affecting write; when a
rule feels out of date and you want to confirm what it actually says.

```
# Read one section at a time:
profile_get(section: "rules")        # hard + soft rules
profile_get(section: "identity")
profile_get(section: "style")
profile_get(section: "stack")
profile_get(section: "constraints")

# Or the whole constitution (no section arg):
profile_get()
```

**How to refresh a stale rule.** Profile writes are the most privileged
(BLUEPRINT §7); Desktop-client-only; always propose → confirm.

```
# 1. Read current state.
profile_get(section: "rules")

# 2. Stage the new section (set status: active; for replacement,
#    add supersedes: <old-slug>).
wiki_content_write(
  uri: "profile/hard-rules.md",
  content: "---\ntitle: \"Hard Rules\"\ntype: profile\nsection: rules\npriority: hard\nstatus: active\ncreated: 2026-05-23\nlast_verified: 2026-07-19\n---\n\n- No emoji in commit messages.\n- ..."
)

# 3. Show the user the diff BEFORE ingesting. Wait for explicit "yes".

# 4. Validate + index + commit.
wiki_ingest(path: "profile/hard-rules.md")

# 5. Verify.
profile_get(section: "rules")
```

**Verify.** `profile_get` returns the new text exactly as ingested.

**Watch out.**
- **NEVER silently update profile.** Even if the user said "the rule
  about X is old", show the new text and wait for confirmation.
- Profile is not in the search index (ADR-007). `wiki_search` will not
  find profile content; always read via `profile_get`.
- Profile changes are Desktop-only. If running as Codex/skill/cron,
  surface the proposed diff and escalate to Desktop.

---

## 3. Search the Knowledge Base

**When to use.** Whenever you need to find existing knowledge before
capturing new. Three concurrent strategies cover every shape.

```
# A. Keyword discovery over all page types (BM25). The workhorse.
wiki_search(query: "hybrid retrieval fusion", top_k: 10)
#  -> matches titles, summaries, tags, read_when, tldr, body.

# B. Discovery scoped to semantic types (concept/entity/source/
#    project/decision). Today a BM25 alias with the type filter;
#    becomes hybrid (vector + BM25 + rerank) when the vector tier
#    is wired. Use for intent clarity.
semantic_search(query: "scaling laws", type: "concept", top_k: 10)

# C. Claim lookup in the semantic brain. CONFIRMED claims only.
brain_search(query: "qdrant", domain: "infra", top_k: 10)
```

**When each wins.**

| Question shape | Tool | Why |
|---|---|---|
| "Find pages mentioning Y" / "How do we handle X?" | `wiki_search` | BM25 across all page types. |
| "What concept do we have about Z?" | `semantic_search` | Strict type filter; future-proofs into hybrid retrieval. |
| "What did we decide about Y?" / "What's the value of X?" | `brain_search` | Ledger-backed confirmed claims, not document text. |

Rule of thumb: documents → `wiki_search`; concepts →
`semantic_search`; facts/claims → `brain_search`.

**Verify.** Cross-check two strategies when the answer matters. If
`wiki_search` finds a `concept` page that should have associated claims,
`brain_search` for the same subject should return them. Empty
`brain_search` for a topic with rich concept pages means claims have
not been crystallized yet (see [Scenario 10](#10-crystallize-a-session)).

**Watch out.**
- `brain_search` sees confirmed claims only. A captured-but-unconfirmed
  claim is invisible. Use `brain_get` for a known subject.
- The `type` facet on `wiki_search` is **unfiltered** even when the
  `type` arg is set (gotcha in `tool-reference.md`). Use
  `semantic_search` for strict filtering.
- Pass `format: "llms"` when feeding results back into context — it
  drops score/excerpt noise.

---

## 4. Read a Specific Page or Claim

**When to use.** You know what you want — a page by slug or URI, a
claim by subject, or a procedure by id — and want the body, not a
search ranking.

```
# 4a. Markdown page by slug or wiki:// URI (bare slug, short URI,
#     and full wiki:// URI all accepted).
wiki_content_read(uri: "concepts/moe")
wiki_content_read(uri: "wiki://research/concepts/moe")

# Graph-aware read: include pages that link INTO this one.
# (Response shape becomes JSON { content, backlinks: [...] }.)
wiki_content_read(uri: "concepts/moe", backlinks: true)

# Body only, no frontmatter:
wiki_content_read(uri: "concepts/moe", no_frontmatter: true)

# 4b. Semantic-tier read (Blueprint alias; functionally equivalent
#     today; use for intent clarity on concept/entity/source/
#     project/decision pages).
semantic_get(page_id: "concepts/moe", with_backlinks: true)

# 4c. Claim lookup by subject (ledger-derived claim set).
brain_get(subject: "AAPL")
brain_get(subject: "concept:reciprocal-rank-fusion", domain: "retrieval")

# 4d. Procedure read (runbook with verification block).
procedural_get(proc_id: "procedural/deployment/deploy-brain-mcp")
```

**Verify.** For pages: full body incl. frontmatter (unless
`no_frontmatter: true`). For claims: the full claim set for the
subject; empty means wrong subject string (try `brain_search` first)
or no confirmed claims yet.

**Watch out.**
- `backlinks: true` changes the response shape from text to JSON.
  Parse accordingly.
- `brain_get` is subject-keyed. Wrong subject → empty result. Use
  `brain_search` with a substring when unsure.
- Superseded pages render with a notice pointing to the replacement.
  Follow it.
- Bundle vs. flat slug: `wiki://research/concepts/moe` resolves to
  `index.md` if the page is a bundle; append `/asset.png` for a
  co-located asset.

---

## 5. Capture a User Utterance as a Claim

**When to use.** The user said something worth remembering verbatim —
a preference, decision, fact. Provenance is "the user said it". Most
common write scenario.

> **NEVER skip the confirm step for user utterances.** A captured
> claim is `proposed` until the user explicitly confirms. Showing the
> proposal in human-readable form and waiting is mandatory.

```
# 5a. Read current state so the capture is a diff, not a blind add.
brain_get(subject: "TSLA", domain: "stocks")

# 5b. Capture. ALL six args are required.
brain_capture(
  operation_id: "u7-capture-0192f...",         # fresh UUIDv7
  utterance: "Tesla reports Q3 2025 EPS of $0.62.",
  subject: "TSLA",
  predicate: "eps_q3_2025",
  value: "0.62",
  domain: "stocks"
  # claim_kind defaults to "user_assertion"
)
#  -> returns { proposal_operation_id: "u7-capture-0192f...",
#               status: "proposed" }

# 5c. Render the proposal to the user BEFORE confirming:
#
#     Proposed claim (from your statement):
#       subject:   TSLA
#       predicate: eps_q3_2025
#       value:     0.62
#       domain:    stocks
#       utterance: "Tesla reports Q3 2025 EPS of $0.62."
#     Confirm? (yes / no / edit)

# 5d. ONLY after explicit approval:
brain_confirm(
  operation_id: "u7-confirm-0193a...",          # fresh UUIDv7
  proposal_operation_id: "u7-capture-0192f..."  # from 5b
)

# 5e. Verify by reading back.
brain_get(subject: "TSLA", domain: "stocks")
```

**Verify.** `brain_get` shows the new claim in the subject's claim
set. `brain_search(query: "TSLA", domain: "stocks")` surfaces it too.

**Watch out.**
- All six args of `brain_capture` are required: `operation_id`,
  `utterance`, `subject`, `predicate`, `value`, `domain`.
- `operation_id` on `brain_confirm` is the **confirm** op's id (fresh),
  not the capture's. `proposal_operation_id` is the one returned from
  `brain_capture`.
- Conflict policy: `latest-user-wins` applies only to `preference`,
  `profile`, and `project decision` claims; external facts never use
  it — conflicting credible claims coexist as `disputed`.
- For agent inferences, use `brain_propose` (Scenario 6), not capture.

---

## 6. Propose an AI-Derived Claim

**When to use.** You (the agent) inferred something by reading a
source, deduction, or summarization. Provenance is "the agent inferred
it". Stricter than capture: AI-derived claims are **always**
`proposed`, never auto-confirmed, and must cite evidence when they
claim support.

```
# 6a. (Recommended) read current state.
brain_get(subject: "concept:reciprocal-rank-fusion", domain: "retrieval")

# 6b. Propose. method is required. Cite evidence chunk ids if any.
brain_propose(
  operation_id: "u7-prop-0194f...",
  subject: "concept:reciprocal-rank-fusion",
  predicate: "default_k",
  value: "60",
  domain: "retrieval",
  method: "llm_extraction",
  model: "claude-sonnet-4",
  evidence_capture_operation_ids: "u7-src-...-chunk-0,u7-src-...-chunk-1"
  # claim_kind defaults to "inference"
)
#  -> returns { proposal_operation_id, status: "proposed" }
#     STATUS IS ALWAYS "proposed".

# 6c. Render to the user with provenance:
#
#     AI-derived proposal:
#       subject:   concept:reciprocal-rank-fusion
#       predicate: default_k
#       value:     60
#       method:    llm_extraction (claude-sonnet-4)
#       evidence:  u7-src-...-chunk-0, u7-src-...-chunk-1
#     Confirm? (yes / no / edit)

# 6d. ONLY after explicit approval:
brain_confirm(
  operation_id: "u7-confirm-0195a...",
  proposal_operation_id: "u7-prop-0194f..."
)
```

**Verify.** `brain_get` returns the confirmed claim with provenance
(method, model, evidence ids).

**Watch out.**
- `brain_propose` always leaves status `proposed`. The user must
  explicitly confirm.
- Unsupported proposals without evidence are refused by the extraction
  policy. If you have no evidence, capture it as a user assertion via
  `brain_capture` instead (user "adopted" the inference by saying it).
- Evidence spans are hash-checked. Do not hand-craft
  `evidence_capture_operation_ids`; only pass ids returned from
  `brain_ingest_source`.
- `brain_capture` vs `brain_propose` — distinction is provenance:
  capture = user said it, propose = agent inferred it. Both leave
  status `proposed`; both need `brain_confirm`.

---

## 7. Supersede a Stale Claim

**When to use.** A confirmed claim is now outdated. Record the new
value and link it back so history is preserved.

```
# 7a. Read current claim(s) for the subject. Note the confirm op id(s)
#     you will supersede.
brain_get(subject: "TSLA", domain: "stocks")
#  -> eps_q3_2025 = 0.58   (confirm op id: u7-confirm-deadbeef...)

# 7b. Propose the NEW claim first (must exist as a proposal before
#     brain_supersede can reference it).
brain_propose(
  operation_id: "u7-prop-0196f...",
  subject: "TSLA",
  predicate: "eps_q3_2025",
  value: "0.62",
  domain: "stocks",
  method: "llm_extraction",
  model: "claude-sonnet-4",
  evidence_capture_operation_ids: "u7-src-...-chunk-0"
)

# 7c. Render before/after to the user:
#
#     Supersede proposal:
#       OLD: TSLA.eps_q3_2025 = 0.58  (op u7-confirm-deadbeef...)
#       NEW: TSLA.eps_q3_2025 = 0.62  (op u7-prop-0196f...)
#     Confirm supersede? (yes / no / edit)

# 7d. ONLY after explicit approval. superseded_claim_operation_ids is
#     a COMMA-SEPARATED STRING of confirm op ids.
brain_supersede(
  operation_id: "u7-super-0197a...",
  proposal_operation_id: "u7-prop-0196f...",
  superseded_claim_operation_ids: "u7-confirm-deadbeef..."
)

# 7e. Verify.
brain_get(subject: "TSLA", domain: "stocks")
```

**Verify.** `brain_get` shows the new value as current. The old claim
is not deleted — supersession is an append-only ledger event that
marks it superseded. `audit_history` still shows the old value with its
supersede transition.

**Watch out.**
- `superseded_claim_operation_ids` is a comma-separated **string**.
  Pass multiple confirm op ids to retire several claims at once.
- The new claim must already be `proposed`. `brain_supersede`
  references a `proposal_operation_id` from a prior `brain_propose` or
  `brain_capture`.
- Supersede preserves history. For irreversible destruction (privacy
  purge) — Tier-4, explicit re-authentication required — read
  `references/anti-patterns.md` and `references/architecture.md`.

---

## 8. Ingest an External Source

**When to use.** You read a URL, file, or pasted text and want to turn
it into typed claims. Two-stage: quarantine into chunks, then extract
claims per chunk.

```
# 8a. Quarantine. EXACTLY ONE of url / text / file_path.
brain_ingest_source(
  operation_id: "u7-src-0198f...",
  url: "https://example.com/blog/hybrid-search",
  max_chunk_bytes: 4000       # default
)
#  -- OR -- text: "<inline text>"  -- OR --
#  file_path: "inbox/imported-note.md"  (relative to wiki root or
#                                        absolute inside it)
#  -> returns chunk capture_operation_ids:
#     ["u7-src-0198f...-chunk-0", "u7-src-0198f...-chunk-1", ...]

# 8b. Extract from each chunk that should yield claims. method is
#     required; model recommended; local_only is the sensitive-source
#     switch.
brain_extract(
  capture_operation_id: "u7-src-0198f...-chunk-0",
  method: "llm_extraction",
  model: "claude-sonnet-4",
  local_only: false
)
#  -> returns proposed claims with sha256-verified evidence spans.
#     Status is ALWAYS "proposed"; never auto-confirmed.

# 8c. For SENSITIVE sources (must not leave the host), set local_only.
brain_extract(
  capture_operation_id: "u7-src-0198f...-chunk-2",
  method: "llm_extraction",
  local_only: true
)
#  -> denies the AI provider call outright. Returns nothing
#     extracted. See references/anti-patterns.md.

# 8d. Show each extracted proposal to the user. Confirm only those
#     they accept.
brain_confirm(
  operation_id: "u7-confirm-0199a...",
  proposal_operation_id: "<from a brain_extract result>"
)
```

**Verify.** After confirming, `brain_search(query: "<topic>")`
returns the new claims. Original chunks stay in the object store
(SHA-256 addressed) so evidence spans stay verifiable.

**Watch out.**
- URLs are SSRF-guarded (http/https only). File paths resolve relative
  to the wiki root. Text is inline.
- Quarantine produces one capture per paragraph-packed chunk. Iterate
  extraction over the chunks that matter.
- Evidence spans are mechanically verified. A quote that does not
  hash-match the chunk bytes returns `QuoteHashMismatch` and the
  proposal is rejected — prompt-injection resistance. Never hand-craft
  evidence.
- Extraction is one chunk at a time — no batch.

---

## 9. Write a Durable Markdown Page

**When to use.** Author or update a typed wiki page — concept, entity,
source, project, decision, procedure, doc, or section. The page is the
canonical Markdown layer; it goes into git.

> Type decides flow. Pick the type **first** using the decision tree
> in `references/type-system.md`. The worked example uses `concept`;
> for an executable runbook with a verification block, use `procedure`.

```
# 9a. Pick the type first (type-system.md decision tree).

# 9b. Scaffold (returns the canonical path).
wiki_content_new(
  uri: "concepts/reciprocal-rank-fusion",
  name: "Reciprocal Rank Fusion",
  type: "concept"
)
#  -> returns { uri, slug, path, wiki_root, bundle }.
#     You can write directly to `path` (recommended in
#     docs/guides/writing-content.md) or use wiki_content_write.

# 9c. Stage the full content. For updates, read first via
#     wiki_content_read so the write is a real diff.
wiki_content_write(
  uri: "concepts/reciprocal-rank-fusion",
  content: "---\n" +
            "title: \"Reciprocal Rank Fusion\"\n" +
            "type: concept\n" +
            "summary: \"Fuse ranked lists using 1/(k+rank) scoring.\"\n" +
            "read_when:\n" +
            "  - \"Building a hybrid retrieval pipeline\"\n" +
            "status: active\n" +
            "confidence: 0.9\n" +
            "tags: [retrieval, ranking, fusion]\n" +
            "sources:\n" +
            "  - sources/cormack-2009-rrf\n" +
            "concepts:\n" +
            "  - concepts/hybrid-search\n" +
            "---\n\n" +
            "# Reciprocal Rank Fusion\n\n" +
            "RRF combines ranked lists: score(d) = sum_i 1/(k + rank_i(d)).\n"
)
#  -> writes the file. Does NOT validate, index, or commit.

# 9d. Show the user the proposed content (or diff). Wait for approval.

# 9e. Validate + index + commit. Pass redact: true if the body may
#     contain secrets (lossy — see docs/guides/redaction.md).
wiki_ingest(
  path: "concepts/reciprocal-rank-fusion.md",
  redact: false
)
#  -> returns { pages_validated, warnings, commit, redacted }.

# 9f. If auto_commit was off, or you want one atomic commit for
#     several staged pages:
wiki_content_commit(
  slugs: "concepts/reciprocal-rank-fusion",
  message: "add reciprocal-rank-fusion concept"
)

# 9g. Verify.
wiki_content_read(uri: "concepts/reciprocal-rank-fusion")
```

**Verify.** `wiki_content_read` returns the page exactly as ingested.
Run `wiki_lint(rules: "broken-link,orphan")` after a multi-page session
to catch dead references.

**Watch out.**
- Type decides flow. A how-to with a verification block is a
  `procedure`, not a `concept` (acid test in `type-system.md`). A
  procedure needs `status: draft` until its `verification` block has
  passed at least 3 times.
- `wiki_content_write` alone does not commit or index. Always pair
  with `wiki_ingest` (or follow with `wiki_content_commit`).
- Bare slugs are canonicalized from frontmatter type into the Blueprint
  layout at write time.
- `redact: true` is lossy. Review the `redacted` array to see what
  was scrubbed.
- For updates, always `wiki_content_read` first so your write is a
  real diff.
- Sections are pages too. `wiki_content_new(uri: "...", section: true)`
  creates `dir/index.md` with `type: section`. Sections are excluded
  from search by default.

---

## 10. Crystallize a Session

**When to use.** At the end of a working session, before the user
leaves. Decide what was learned and durably record each piece through
the correct flow.

```
# 10a. Inventory what was learned. Bucket each item by type and show
#      the user one consolidated plan:
#
#      Crystallization plan:
#        [claim] TSLA.eps_q3_2025 = 0.62            -> Scenario 5
#        [claim] concept:rrf.default_k = 60         -> Scenario 6
#        [super] TSLA.eps_q3_2025 0.58 -> 0.62      -> Scenario 7
#        [page]  concepts/reciprocal-rank-fusion    -> Scenario 9
#        [page]  procedural/.../deploy-brain-mcp    -> Scenario 9
#      Confirm each? (yes / no / edit / skip)

# 10b. For each approved item, run the matching scenario (5-9).
#      Do NOT batch-confirm without showing each diff.

# 10c. After all writes land, commit pending markdown pages in one
#      atomic commit if you staged several:
wiki_content_commit(
  slugs: "concepts/reciprocal-rank-fusion,procedural/deployment/deploy-brain-mcp",
  message: "crystallize session: RRF concept + deploy procedure"
)

# 10d. Confirm the wiki grew correctly.
wiki_stats()
#  -> page counts by type/status, graph topology, orphans.

# 10e. Optional health pass before the session ends.
wiki_lint(rules: "orphan,broken-link,stale")
```

**Verify.** Every approved claim appears in `brain_search` /
`brain_get`. Every approved page appears in `wiki_content_read`.
`wiki_stats` shows counts consistent with what was written.

**Watch out.**
- One confirmation per item. Never batch-confirm proposals you have
  not shown the user individually.
- Procedures start at `draft`. Do not pre-mark `verified`.
- Run `wiki_lint` before the session ends — new pages often introduce
  broken `[[wiki-links]]` or orphans; catching them now is cheap.
- Crystallization is not episodic recall. The ledger records claim
  transitions, not chat history — "what did we say last week" is out
  of scope for v1 (`references/architecture.md` "What It Is Not").

---

## 11. Audit and Explore the Graph

**When to use.** Understand what depends on a page before changing
it; time-travel a page's history; find missing links and orphans;
render the wiki structure for review.

```
# 11a. Single-hop neighborhood (cheap, focused).
graph_neighbors(page_id: "concepts/moe", depth: 1)
#  optional: edge_types: "depends-on,fed-by"

# 11b. Full subgraph (render-ready or LLM-readable).
wiki_graph(
  root: "concepts/moe",
  depth: 2,
  format: "mermaid"      # or "dot", or "llms" for a natural-language digest
)
#  -> use format: "llms" for a paragraph summary (clusters, hubs,
#     isolated nodes) rather than a diagram you have to render.

# 11c. Audit trail: git + ledger combined.
audit_history(path: "concepts/moe", limit: 20)
#  -> includes claim events AND git shas. Use when the page has
#     associated claim events and you need the full trail.

# 11d. Pure git log (commits, dates, authors).
wiki_history(slug: "concepts/moe", limit: 20, follow: true)

# 11e. Find missing links to enrich a page.
wiki_suggest(slug: "concepts/moe", limit: 5)
#  -> suggested related pages. Cheap; safe to call before writing.
```

**Verify.** The two history tools (`audit_history`, `wiki_history`)
should agree on the git-commit portion of the trail. Divergence means
the ledger has events that have not yet projected into git (or vice
versa) — investigate before relying on either.

**Watch out.**
- `graph_neighbors` is for one hop; `wiki_graph` is for full subgraphs.
  Do not use `wiki_graph` for a single-hop question.
- `audit_history` reads the audit ledger (claim events + git shas);
  `wiki_history` reads pure git log. Distinct tools, distinct data.
- `wiki_graph` without `cross_wiki` renders cross-wiki links as
  external placeholder nodes. Pass `cross_wiki: true` to merge all
  mounted wikis.
- Large wikis skip some metrics in `wiki_stats` (`communities: null`
  below `graph.min_nodes_for_communities` default 30; diameter/radius
  skipped above `graph.max_nodes_for_diameter` default 2000).

---

## 12. Export Knowledge

**When to use.** You need a **file** of the wiki for an external
consumer: an LLM context window, a CI pipeline, offline analysis, or
the `llms.txt` publishing ecosystem.

```
# 12a. llms.txt ecosystem — summary listing (default format).
wiki_export(wiki: "brain")
#  -> writes <wiki-root>/llms.txt. Use for LLM clients that follow
#     the llms.txt convention (Cursor, Perplexity, etc.).

# 12b. llms-full — same shape, inlines full page bodies.
wiki_export(wiki: "brain", format: "llms-full")
#  -> long-context consumption; one file, every body inlined.

# 12c. JSON — for tooling, CI, batch scripts.
wiki_export(wiki: "brain", format: "json")

# 12d. Include archived/superseded pages.
wiki_export(wiki: "brain", format: "llms-full", status: "all")

# 12e. Custom output path.
wiki_export(wiki: "brain", format: "json", path: "exports/brain-2026-07-19.json")
```

**When to use each format.**

| Format | For | Why |
|---|---|---|
| `llms-txt` (default) | LLM context, `llms.txt` publishing | Compact summary listing; community convention. |
| `llms-full` | Long-context LLM analysis | Inlines every body so a long-context model reasons over the whole wiki in one prompt. |
| `json` | Tooling, CI, batch scripts | Machine-parseable; downstream code reads metadata + body per page. |

**Verify.** `wiki_export` returns a report
(`{path, pages_written, bytes, format}`) — it does not stream content
back. Read the file at the returned path to verify.

**Watch out.**
- `wiki_export` writes to disk; it is not a session tool. Default path
  is `<wiki-root>/llms.txt`; pass `path:` to override.
- `status: "active"` (default) excludes archived. Pass `status: "all"`
  to include superseded/archived.
- Distinct from `format: "llms"` on search/list/graph (in-session
  compact output — see `docs/guides/llms-format.md`).

---

## Anti-Patterns to Refuse

When the user (or another agent) asks for any of the following, refuse
and explain why. The full risk catalog lives in
`references/anti-patterns.md`.

### "Just save this directly"

> "Skip the propose/confirm dance, just write it."

**Refuse.** Every durable write to memory must go through propose →
confirm (SKILL rule 1). Skipping is the number-one memory failure
mode (BLUEPRINT ADR-006). Offer `brain_capture` (utterance) or
`brain_propose` (inference), show the diff, confirm normally. The only
exception is an explicit "save this now" from the user — that counts
as confirmation in advance; still show the diff after writing.

### Ingesting a source without redaction when it might contain secrets

> "Just pull this URL into the wiki as-is."

**Refuse if the source may contain secrets.** For external content
(web clips, transcripts, pasted notes, API docs with example
credentials), either:

1. `brain_ingest_source` then `brain_extract` with `local_only: true`
   if content must not leave the host; OR
2. If writing as a wiki page via `wiki_ingest`, pass `redact: true` so
   built-in patterns (GitHub PATs, OpenAI/Anthropic/AWS keys, bearer
   tokens, emails) are scrubbed before commit.

Redaction is **lossy**. Review the `redacted` array. See
`docs/guides/redaction.md` and `references/anti-patterns.md`.

### Treating a procedural how-to as a concept page (or vice versa)

> "Make a concept page called 'Deploy brain-mcp'."

**Refuse if the content is executable.** Run the acid test from
`references/type-system.md`:

| # | Question | Procedural | Semantic |
|---|---|:---:|:---:|
| Q1 | Can a junior copy-paste and execute it as-is? | yes | no |
| Q2 | Is there a pass/fail verification step? | yes | no |
| Q3 | Is "did this succeed?" answerable as a boolean? | yes | no |

All three yes → `procedure` (needs `verification` block; `draft` until
verified 3 times). Any no → `concept`. Filing a how-to as a concept
loses the verification gate; filing a synthesis as a procedure loses
the source-citation graph.

### Confirming an AI-derived proposal without showing the user

> "You proposed three claims from that paper, just confirm them all."

**Refuse.** AI-derived claims (`brain_propose` / `brain_extract`
output) are **always** `proposed`. The user must explicitly approve
each one before `brain_confirm`. Show each proposal with provenance
(method, model, evidence ids) and wait for per-item approval.
Batch-confirming unseen proposals defeats the audit trail and launders
unsupported inferences into durable memory.

### Other patterns to escalate

Anything in `references/anti-patterns.md` marked destructive — profile
changes from a non-Desktop client, hard purge of claims, `wiki_schema
action: remove` with `delete_pages: true`, `wiki_spaces_remove` with
`delete: true` — requires explicit user consent with the blast radius
explained first (SKILL rule 5).

---

## Quick Reference: Read → Write → Verify

Every scenario in this playbook follows the same shape:

```
1. READ current state
     profile_get / wiki_content_read / brain_get / brain_search
2. PROPOSE the change
     brain_capture (user utterance) | brain_propose (AI-derived)
     wiki_content_write (page body, not yet committed)
3. SHOW the user a human-readable diff
     subject / predicate / value / provenance, or
     page frontmatter + body, before/after
4. CONFIRM only after explicit "yes"
     brain_confirm / brain_supersede (claims)
     wiki_ingest / wiki_content_commit (pages)
5. VERIFY by reading back
     brain_get / brain_search (claims)
     wiki_content_read / wiki_lint / wiki_stats (pages)
```

If you ever find yourself about to skip step 3 or 4, stop and re-read
`references/anti-patterns.md`. The propose/confirm gate is the entire
reason brain-mcp exists.

---

## Reference Index

| Reference | When to read |
|---|---|
| `references/architecture.md` | Why: three stores, event-ledger authority, epistemic model, recovery model. |
| `references/tool-reference.md` | How: the 39-tool matrix with arguments, tiers, examples, gotchas. |
| `references/type-system.md` | Which type a new page or claim should be; the acid test; worked examples. |
| `references/anti-patterns.md` | Before any destructive or risky operation; the full risk catalog. |
| `references/workflows/operator.md` | This file. Day-to-day memory and knowledge tasks. |
| `references/workflows/developer.md` | Contributing to brain-mcp source. |
| `references/workflows/deployer.md` | Deployment, systemd, MCP client config, backups. |

When this skill and the repo disagree, the repo wins.
