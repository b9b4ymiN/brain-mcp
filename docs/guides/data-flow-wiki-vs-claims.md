# brain-mcp Data Flow — Wiki vs Semantic

Audience: any agent or human confused about **where data lands** and **which tool to pick**. This is the practical companion to the brain skill's `references/architecture.md` (WHY) and `references/tool-reference.md` (HOW) — it answers **WHEN** to use each path.

> **Mirror note:** this file is kept in sync with `~/.agents/skills/brain/references/data-flow.md`. The skill copy is what MCP clients (Claude Code, Codex, etc.) read at session start; this repo copy is the canonical source for reviewers and developers.

If you've ever wondered:
- "I ingested a Markdown page — why is Inbox still empty?"
- "Why doesn't Console Search find the page I just wrote?"
- "What's the difference between `wiki_ingest` and `brain_ingest_source`?"

…read this first.

---

## TL;DR — Two Stores, Not One

brain-mcp holds data in **two separate stores** that do **not** sync automatically:

| Store | What lives here | Format | Who writes (typically) | Trust level |
|---|---|---|---|---|
| **Wiki Tree** (Markdown + git) | Documents: runbooks, notes, research, knowledge base | `.md` files with frontmatter | Operator (you) — authored | High (you wrote it) |
| **Semantic Store** (sqlite + encrypted blobs) | Facts: structured `{subject, predicate, value}` assertions | JSON objects in event ledger | LLM extraction, capture, propose — needs review | Variable (LLM can be wrong) |

**Iron rule:** writing to one store does **not** populate the other. `wiki_ingest` does not create claims. `brain_capture` does not create pages. Pick the right path for the job.

---

## Mental Model

```
   "I want to remember this"
            │
   ┌────────┴────────┐
   │                 │
   ▼                 ▼
 as DOCUMENT     as FACT
 (human-readable, (structured,
  long-form)      queryable)
   │                 │
   ▼                 ▼
 Wiki Tree       Semantic Store
 wiki_ingest     brain_capture
 wiki_content_   brain_propose
   write         brain_ingest_source
                 brain_extract
   │                 │
   ▼                 ▼
 Tantivy         SQLite FTS5
 (BM25 over       (over claims:
  page text)       subject,
                   predicate,
                   value)
   │                 │
   ▼                 ▼
 wiki_search     brain_search
 wiki_get        brain_get
 wiki_list       audit_history
```

**Critical asymmetry:** the Console's Search page calls `search_claims` (FTS5 over the semantic store), NOT tantivy over pages. Searching "thai" in Console won't find `procedural/research/thai-stock-factsheet.md` because that's a page, not a claim. To search page content, use the MCP `wiki_search` tool.

---

## Side-by-Side Comparison

| Dimension | Wiki Page | Semantic Claim |
|---|---|---|
| **What it is** | A Markdown document | A single structured assertion |
| **Example** | `procedural/research/thai-stock-factsheet.md` (a 200-line runbook) | `{subject: "CATL", predicate: "revenue", value: "CNY 424B"}` |
| **Storage** | `/data/wikis/<wiki>/wiki/<slug>.md` + git history | `semantic.sqlite3` + encrypted object store (event ledger) |
| **Index** | Tantivy (BM25 full-text over title + body + tags) | SQLite FTS5 (over subject + predicate + value text) |
| **Search hits** | "thai-stock-factsheet" body, title, tags | `subject="CATL"`, `predicate="revenue"`, `value="CNY 424B"` |
| **Mutation flow** | Write file → git commit → tantivy re-index | Propose → review in Inbox → confirm → claim lives |
| **Delete** | `git rm` + commit | Retract (soft delete + audit trail in ledger) |
| **Versioning** | Git (diff, merge, branch, history) | Append-only event ledger + supersede chain |
| **Provenance** | Git author + commit message | `provenance_kind` (user_assertion / inference / external_fact) + capture operation id + model |
| **Trust** | Operator-curated — trusted on write | Must pass Inbox review before becoming authoritative |
| **Best for** | Documentation, runbooks, research notes, knowledge base | Structured facts you want to query, aggregate, draw as graph |

---

## When to Use Which Path

### Use **Wiki** (Markdown) when:

- You're writing a **document** someone will read end-to-end (runbook, analysis, guide)
- You're keeping **research notes** or **decisions** as narrative
- You want **git-style history** (diff, branch, blame)
- The content is **long-form prose**, not a single fact
- You're fine with the operator (you) being the trust authority
- **Tools:** `wiki_ingest`, `wiki_content_write`, `wiki_content_commit`

### Use **Semantic** (claims) when:

- You want **structured facts** you can query (`brain_search "CATL"`, Entity page, Galaxy)
- You're extracting knowledge **from an external source** (URL, PDF, big text) and want LLM to do the work
- You need **provenance** — "this claim came from this source, captured at this time, by this model"
- You want a **review gate** — capture now, decide later (Inbox)
- You want to draw **entity relationships** as a graph
- **Tools:** `brain_capture`, `brain_propose`, `brain_ingest_source` + `brain_extract`

### Common confusion: "ingest" appears in both

| Tool | Path | What it actually does |
|---|---|---|
| `wiki_ingest` | Wiki | Validate + commit + index Markdown files in a directory |
| `brain_ingest_source` | Semantic | Fetch URL/file/text → chunk → store as **capture** in object store (intermediate, not a claim yet) |

They are NOT the same operation despite the shared word. `brain_ingest_source` produces raw chunks; you then run `brain_extract` on a chunk to get proposed claims.

---

## Real-World Scenarios

### Scenario 1: "I read CATL's annual report and want to remember the key numbers"

You have two options:

**Option A — write a research note (Wiki path)**

```text
wiki_content_write  uri="sources/catl-annual-report-2025"
                    content="# CATL 2025 Annual Report\n\nRevenue: CNY 424B..."
wiki_ingest         path="sources/catl-annual-report-2025.md"
```

Result: a Markdown page lives in the wiki. Findable via `wiki_search "CATL revenue"`. Does NOT appear in Inbox (you wrote it, no review needed).

**Option B — extract structured facts (Semantic path)**

```text
1. brain_ingest_source  url="https://catl.com/annual-report-2025.pdf"
                        → returns capture_operation_id

2. brain_extract        capture_operation_id=<id from step 1>
                        → LLM extracts N proposed claims

3. (review N proposals in Inbox → Approve the ones you trust)

4. brain_search         "CATL revenue"
                        → finds the confirmed claim
```

Result: structured claims live in semantic store. Findable via `brain_search`, appear on Entity page, feed the Galaxy graph. Each claim carries provenance (capture id, source URL, model).

**Both can coexist** — many operators write a narrative note AND extract claims from the same source.

### Scenario 2: "I added a new page — why is Inbox empty?"

Because you used the Wiki path. Inbox shows proposed **claims**, not pages. Adding a Markdown page does not propose any claim.

To populate Inbox you need to:

- **Manually:** `brain_capture subject="..." predicate="..." value="..."` → 1 proposal in Inbox
- **Automatically:** `brain_ingest_source` + `brain_extract` → N proposals in Inbox

### Scenario 3: "Why doesn't Console Search find my page?"

The Console's Search page calls `/api/v1/search` which is backed by `search_claims` (FTS5 over the semantic store). It searches claims, not pages.

To search page content:

- **MCP path:** use `wiki_search` (BM25 over tantivy — fast, hits body + title + tags)
- **Console path:** not currently exposed. This is a known gap (the Console expansion's "Bug 2").

### Scenario 4: "I want both — page AND structured facts"

Do both, in either order:

```text
# Write the page
wiki_content_write uri="sources/catl-q2-2025" content="..."
wiki_ingest        path="sources/catl-q2-2025.md"

# Extract claims from the same source URL
brain_ingest_source url="https://catl.com/q2-2025.pdf"
brain_extract       capture_operation_id=<id>
# (review proposals in Inbox)
```

The page and the claims live in different stores. Cross-references between them happen via:

- `[[wikilinks]]` inside the page (used by `wiki_graph`)
- `subject` of a claim matching a slug or entity name (Galaxy draws these as edges)

There is no automatic "page → claim" or "claim → page" sync today.

---

## Console Page → Source Store

Which Console page reads from which store:

| Console page | Primary source | Notes |
|---|---|---|
| **Home** (galaxy) | Semantic (graph from claims) | + Inbox preview |
| **Today** | Both (composes) | Inbox count + Status (wiki) + Activity (wiki git) |
| **Search** | ⚠ **Semantic (claims FTS5)** | Does NOT search wiki pages — known gap |
| **Inbox** | Semantic (proposals) | Status = "proposed" |
| **Entity** | Semantic (confirmed claims) | Grouped by subject |
| **Activity** | Wiki (git log) | Page changes only |
| **Status** | Wiki (tantivy stats) | Page count, staleness, index health |
| **Config** | Wiki (config + index) | Operator config, not claims |
| **Operations** | Semantic (trust/jobs/clients) | Trust flags over claims |

---

## Decision Tree

```
"I have information to add"
       │
       ▼
   Is it a long document someone will read?
       │
   ├── YES ──▶ wiki_ingest / wiki_content_write   [Wiki path]
       │
   └── NO (it's a fact)
              │
              ▼
       Do I trust myself to write it correctly?
              │
       ├── YES ──▶ brain_capture                  [Semantic, manual]
       │           (1 proposal → Inbox → Approve)
       │
       └── NO (extract from source via LLM)
                  │
                  ▼
            brain_ingest_source                   [Semantic, automated]
                  │
                  ▼
            brain_extract
                  │
                  ▼
            (N proposals → Inbox → Approve each)
```

---

## Anti-Patterns

- ❌ `brain_ingest_source` and expecting pages in the wiki. That tool writes **captures** (intermediate chunks), not Markdown files.
- ❌ `wiki_ingest` and expecting Inbox to populate. Wiki ingest writes pages, not claims.
- ❌ Editing `.md` files directly to "fix" a claim. Claims live in the semantic store; use `brain_supersede`.
- ❌ Using Console Search to find a page by content. Console Search hits claims FTS5; use `wiki_search` MCP tool instead.

More in `references/anti-patterns.md`.

---

## Reference

- **brain skill (`~/.agents/skills/brain/`)** — the skill bundles deeper references that mirror this guide:
  - `references/architecture.md` — WHY: 3 stores, event-ledger authority, epistemic model
  - `references/tool-reference.md` — HOW: 40-tool matrix with args, tiers, examples
  - `references/type-system.md` — which type a new page or claim should be
  - `references/anti-patterns.md` — risky patterns to avoid
  - `references/data-flow.md` — this file (kept in sync with the repo copy)
- **In this repo:**
  - `src/mcp/handlers.rs` — handler entry points cited inline above
  - `src/api.rs` — Console HTTP routes (`/search`, `/inbox`, `/status`, etc.)
  - `src/semantic.rs` — semantic store, proposal lifecycle, `list_pending_proposals`
  - `src/index_manager.rs` — tantivy index rebuild + state.toml persistence
