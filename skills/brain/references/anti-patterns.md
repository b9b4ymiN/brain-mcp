---
title: "Anti-Patterns"
summary: "Wrong vs right patterns for brain-mcp. Read this before any non-trivial write or destructive action."
read_when:
  - About to write to durable memory or claims
  - About to invoke a destructive tool
  - Reviewing why a previous action broke something
  - Onboarding a new agent or contributor to the skill
audience: any agent or human operating brain-mcp
last_updated: "2026-07-19"
---

# Brain MCP Anti-Patterns

This is the catalogue of ways to misuse brain-mcp that cause real harm:
silent writes, lost provenance, broken audits, privacy leaks, irreversible
deletions. Each entry has the **wrong** pattern, the **right** pattern,
the **why**, and the **recovery** path if you already did the wrong thing.

**Iron rule:** if any pattern here matches what you are about to do — stop
and re-plan. Surface the risk to the user when in doubt.

---

## Quick index

| # | Anti-pattern | Severity | Tier |
|---|---|---|---|
| 1 | Silent write to profile or claim | Critical | Write discipline |
| 2 | Confirming an AI-derived proposal without showing the user | Critical | Write discipline |
| 3 | Treating a procedural how-to as a concept page | High | Type system |
| 4 | Treating a concept as a procedure (forcing verification) | Medium | Type system |
| 5 | Collapsing source content into a concept (provenance loss) | High | Epistemic |
| 6 | Editing `site/content/` directly instead of `wiki/` | High | Web UI |
| 7 | Calling destructive tools without explicit user consent | Critical | Destructive |
| 8 | Ingesting a source without `redact: true` when secrets may exist | High | Security |
| 9 | Retrying an idempotency conflict with a changed payload | High | Idempotency |
| 10 | Exposing MCP HTTP on a public port without auth or tunnel | Critical | Security |
| 11 | Using `brain_propose` for a user utterance (or `brain_capture` for an inference) | Medium | Claim flow |
| 12 | Editing Markdown to change a claim that lives in the ledger | Critical | Authority |
| 13 | Forcing a tool call when `brain_status` shows degraded schema_version | High | Safety |
| 14 | Skipping session bootstrap (profile/rules not loaded) | Medium | Discipline |
| 15 | Writing to a wiki without first reading the current state | Medium | Discipline |
| 16 | Writing literal `[[double-bracket]]` text (e.g. TOML array-of-tables syntax) into a page body | Low | Content authoring |
| 17 | Free-form `domain`/`kind` strings during extraction (taxonomy drift) | High | Extraction |
| 18 | Packing two facts into one claim value | Medium | Extraction |
| 19 | Re-stating the same fact under a second predicate (duplicate claims) | Medium | Extraction |
| 20 | `current X` predicates + confidence 1.0 on extracted values | Medium | Extraction |
| 21 | Managing MCP-confirmed claims from the Console (client-scope trap) | High | Claim flow |

---

## 1. Silent write to profile or claim

### Wrong

```
# User: "from now on, always use 2-space indentation in TypeScript"
# Agent: (silently) wiki_content_write(... profile/soft-preferences.md ...)
# Agent: "Done."
```

Or:

```
brain_confirm(operation_id: "...", proposal_operation_id: "...")   # without showing user the diff
```

### Right

```
# 1. Read current state
profile_get(section: "style")            # or wiki_content_read on the file

# 2. Propose the change to the user in plain language
"Proposed change to soft-preferences.md:
 - add: 'TypeScript indentation: 2 spaces (hard rule)'
OK to commit? yes/no"

# 3. Only after explicit yes:
wiki_content_write(... updated content ...)
wiki_ingest(path: "profile/soft-preferences.md")
```

For claims:

```
brain_capture(operation_id, utterance, subject, predicate, value, domain)
# Show the user: "Captured proposal:
#   <subject> <predicate> <value>
#   domain: <domain>
#   status: proposed
# Confirm? yes/no"
# Only after explicit yes:
brain_confirm(...)
```

### Why

The #1 failure mode of memory systems is pollution from mistaken or
unreviewed writes. Profile rules govern every future session; a silent
edit becomes permanent policy. See BLUEPRINT §0.3 ("No silent writes")
and ADR-0001 (atomic write topology).

### Recovery

- Git revert the commit (`wiki_history` to find the SHA, then `git revert`).
- For claims, `brain_supersede` the confirmed claim with a corrected one,
  or ask the user whether to retract.
- Apologize to the user and surface the unwanted change explicitly.

---

## 2. Confirming an AI-derived proposal without showing the user

### Wrong

```
# Agent infers: "BTC price will recover next week" from a chart analysis
brain_propose(
  operation_id, subject: "BTC", predicate: "price_forecast",
  value: "recovery_next_week", domain: "crypto",
  method: "llm_extraction", model: "..."
)
brain_confirm(...)        # ❌ self-confirmed
```

### Right

```
brain_propose(...)
# Show user:
"AI-derived proposal (method=llm_extraction, model=...):
   BTC price_forecast = recovery_next_week
   Evidence: capture op ids [...]
This is an inference, not a fact. Confirm before it enters the ledger? yes/no"
# Only after explicit yes:
brain_confirm(...)
```

### Why

`brain_propose` exists precisely because AI output is **proposal material
only**, never confirmed memory without a policy event (ADR-0001 §1).
Self-confirmation collapses the propose/confirm gate and lets inference
become fact.

### Recovery

- If already confirmed: `brain_supersede` with a retraction or correction.
- Document the incident; treat the model that produced the bad inference
  as lower-trust for future proposals in that domain.

---

## 3. Treating a procedural how-to as a concept page

### Wrong

```yaml
---
title: "Deploy brain-mcp"
type: concept           # ❌
status: active
---
## Steps
1. SSH to VM
2. Run install.sh
3. ...
```

### Right

```yaml
---
title: "Deploy brain-mcp"
type: procedure          # ✅
status: draft
verified_count: 0
failure_count: 0
verification:
  - "llm-wiki --version exits 0"
  - "MCP client can list tools"
risk_level: medium
tags: [deployment, mcp]
---
## Steps
...
## Verification
...
## Rollback
...
```

### Why

The acid test (BLUEPRINT §2.4) is unambiguous:

| Question | Procedure | Concept |
|---|---|---|
| Junior copy-paste and execute? | Yes | No |
| Pass/fail verification step? | Yes | No |
| "Did this succeed?" boolean? | Yes | No |

Misclassifying means missing the required `verification` block, which
blocks promotion to `verified`, which blocks `procedural_find` recommending
the runbook. See `references/type-system.md`.

### Recovery

- Change `type` to `procedure`, add the verification block.
- Re-ingest: `wiki_ingest` to re-validate.

---

## 4. Treating a concept as a procedure

### Wrong

```yaml
---
title: "Reciprocal Rank Fusion"
type: procedure           # ❌
verification:
  - "math is correct"
---
```

### Right

```yaml
---
title: "Reciprocal Rank Fusion"
type: concept             # ✅
status: active
summary: "Fusion method that combines ranked lists without score normalization."
read_when:
  - "Designing hybrid retrieval"
tags: [retrieval, ranking, fusion]
---
```

### Why

A concept is synthesized knowledge; there is nothing to "execute" or
"verify as success". Forcing it into the procedure schema makes the page
unsearchable via `wiki_list --type concept` and pollutes procedure
results with non-executable entries.

### Recovery

- Switch `type` to `concept` (or `entity`/`source` as appropriate).
- Re-ingest.

---

## 5. Collapsing source content into a concept (provenance loss)

### Wrong

A paper says "RRF with k=60 is robust to score scale." The agent writes
a single `concept` page that contains the claim as if it were the wiki's
own knowledge, with no `source` page recording which paper said it.

### Right

Two pages:

```yaml
# wiki/sources/cormack-2009-rrf.md
---
title: "Reciprocal Rank Fusion (Cormack 2009)"
type: paper
status: active
summary: "Original RRF paper."
concepts:
  - concepts/reciprocal-rank-fusion
---

# wiki/concepts/reciprocal-rank-fusion.md
---
title: "Reciprocal Rank Fusion"
type: concept
status: active
sources:
  - sources/cormack-2009-rrf
---
```

### Why

The epistemic model requires three roles: what we know (concept), what
each source claims (source), what we concluded (query-result). Collapsing
them means you can no longer ask "which sources support this claim?"
when the concept is later contested. See
`references/architecture.md` § Epistemic Model.

### Recovery

- Split the page into `source` + `concept`.
- Link them via `sources` / `concepts` graph edges.
- Re-ingest both.

---

## 6. Editing `site/content/` directly instead of `wiki/`

### Wrong

The web UI shows a stale page. The agent edits
`brain/site/content/concepts/foo.md` to "fix" it.

### Right

```
# Always edit the source-of-truth:
wiki_content_write(uri: "concepts/foo", content: "...")
wiki_ingest(path: "concepts/foo")

# Then refresh the mirror:
llm-wiki web install --wiki brain --force
sudo systemctl restart brain-mcp
```

### Why

`site/content/` is a generated Hugo mirror produced by
`llm-wiki web install`. It converts section `index.md` files to Hugo
`_index.md` for child-page rendering. Edits to the mirror are silently
overwritten on the next refresh and bypass git history, schema
validation, and index updates.

### Recovery

- Move any unique edits back into `wiki/`.
- Run `llm-wiki web install --wiki brain --force` to regenerate the mirror.
- Audit `git log site/content/` for any unsynchronized edits.

---

## 7. Calling destructive tools without explicit user consent

### Wrong

```
wiki_schema(action: "remove", type: "paper", delete_pages: true)
# ❌ no consent, deletes schema AND all paper pages from disk
```

Or:

```
wiki_spaces_remove(name: "research", delete: true)
```

### Right

Always:

1. Read first to show blast radius (`wiki_schema action: "list"`,
   `wiki_list --type <type>`, `wiki_stats`).
2. Surface the exact consequence to the user: "This will remove the
   `paper` schema and delete N paper pages from disk. This is
   irreversible. Confirm? yes/no".
3. Only after explicit `yes`, call the destructive action.
4. Consider a non-destructive alternative first: set `status: deprecated`
   instead of removing; move pages to `raw/archive/` instead of deleting.

### Why

`wiki_schema` and `wiki_spaces_remove` are classified **Tier 4
destructive** in `annotations_for`. They remove data irrecoverably
(purge-registry notwithstanding for ledger claims; file removal is gone
unless backed up in git). See `references/tool-reference.md` § Tier 4.

### Recovery

- Restore from the most recent git commit (`git checkout HEAD -- <path>`)
  if the deletion was committed.
- Restore from backup if the deletion also removed git history.
- File a postmortem; tighten the confirmation flow that allowed it.

---

## 8. Ingesting a source without `redact: true` when secrets may exist

### Wrong

```
wiki_ingest(path: "inbox/raw-log.md")            # ❌ no redact
# raw-log.md contains an API key, an email, a Bearer token
```

### Right

```
wiki_ingest(path: "inbox/raw-log.md", redact: true)
# IngestReport.redacted lists what was scrubbed and where
```

Or, for `brain_ingest_source`:

```
brain_ingest_source(operation_id, text: ..., local_only: true)
# local_only: true denies the AI provider call entirely — use for
# sources that must never leave the host
```

### Why

Built-in redaction patterns scrub GitHub PATs, OpenAI keys, Anthropic
keys, AWS access keys, Bearer tokens, and emails before content hits git.
Without `redact: true`, a secret enters the canonical Markdown and
spreads to every backup, every clone, every derived index.

### Recovery (if a secret leaked)

- **Assume compromise.** Rotate the secret immediately at its source.
- Force-push the git history to remove the secret (coordinate with
  anyone who has cloned).
- Run `git filter-repo` or BFG on the backup repos.
- Audit `wiki_history`, `audit_history`, and the event ledger for
  copies; purge via the purge registry if needed (irreversible).
- File a security incident.

---

## 9. Retrying an idempotency conflict with a changed payload

### Wrong

```
brain_capture(operation_id: "op-123", ..., value: "v1")
# → returns IDEMPOTENCY_CONFLICT (same op id was already used with different args)

# Agent retries:
brain_capture(operation_id: "op-123", ..., value: "v2")     # ❌
```

### Right

```
# op id was used → either:
# (a) reuse the SAME payload (idempotent replay returns the same outcome)
brain_capture(operation_id: "op-123", ..., value: "v1")     # ✅ replay

# (b) or use a NEW operation_id for the new intent
brain_capture(operation_id: "op-124", ..., value: "v2")     # ✅ new op
```

### Why

`operation_id` is the idempotency key. Per ADR-0001, the server stores
the canonical request hash and outcome per `(owner_id, client_id,
operation_id)`. Replaying with a different payload returns
`IDEMPOTENCY_CONFLICT` and performs no mutation — this is the
protection. Retrying with a changed payload hoping for success is both
futile and a sign of a bug in the calling code.

### Recovery

- Use a fresh `operation_id` for the new intent.
- Audit why the conflict happened (caller bug, retry logic, etc.).

---

## 10. Exposing MCP HTTP on a public port without auth or tunnel

### Wrong

```
llm-wiki serve --http :47778 --web-bind 0.0.0.0
# firewall opens 47778 to 0.0.0.0/0
```

### Right

```
# Option A: bind to Tailscale interface only
llm-wiki serve --http :47778 --web-bind 100.x.y.z

# Option B: bind localhost, expose via reverse proxy with TLS + auth
llm-wiki serve --http 127.0.0.1:47778
# Caddy/Nginx in front with TLS + bearer auth

# Option C: Cloudflare Tunnel
cloudflared tunnel --url http://localhost:47778
```

### Why

The MCP server stores the operator's entire knowledge base — profile,
decisions, sources, claims. A public endpoint is a complete data leak.
Even with `http_allowed_hosts` set, an unauthenticated HTTP listener
can be enumerated and abused.

This is not hypothetical: at least one shipped brain-mcp instance's
`/mcp` HTTP path grants a full-capability owner principal to **any TCP
caller** — there is no per-caller auth on that path (`AuthPolicy` is
enforced for non-owner principals, but the HTTP transport never derives
a caller principal at all). It is safe only because it publishes
loopback-only. Widening the bind/publish on that kind of instance
without adding MCP-layer auth first is an immediate unauthenticated
full write/purge exposure — check the instance's own
`entities/brain-instance` page (or its go-live decision record) before
assuming this has been handled.

### Recovery (if exposed)

- Close the firewall rule immediately.
- Rotate any credentials that may have been stored in the wiki.
- Audit `journalctl -u brain-mcp` for unexpected client IPs.
- Treat the exposure as a security incident.

---

## 11. Using `brain_propose` for a user utterance (or `brain_capture` for an inference)

### Wrong

```
# User said: "I prefer Vim over Emacs"
brain_propose(            # ❌ wrong tool
  operation_id, subject: "user", predicate: "editor_preference",
  value: "vim", domain: "preferences", method: "llm_extraction"
)
```

### Right

```
brain_capture(            # ✅ captures the utterance itself
  operation_id, utterance: "I prefer Vim over Emacs",
  subject: "user", predicate: "editor_preference",
  value: "vim", domain: "preferences",
  claim_kind: "user_assertion"
)
```

### Why

- `brain_capture` is for **user utterances** — `claim_kind: user_assertion`,
  evidence is the utterance text itself.
- `brain_propose` is for **AI-derived inferences** — `claim_kind: inference`,
  evidence is capture op ids backing the inference.

Mixing them corrupts provenance: an inference will be tagged as a user
assertion (high-trust) when it should be tagged as inference
(lower-trust, retractable).

### Recovery

- Supersede the mis-typed claim with the correct kind.
- Audit other claims by the same caller for the same mistake.

---

## 12. Editing Markdown to change a claim that lives in the ledger

### Wrong

```
# Agent wants to retract a confirmed claim about BTC price_forecast
# ❌ Edits wiki/.../btc.md directly to remove the claim
```

### Right

```
# 1. The claim lives in the event ledger; supersede it
brain_propose(operation_id: "op-new", ..., value: "retracted")
brain_supersede(
  operation_id: "op-supersede",
  proposal_operation_id: "op-new",
  superseded_claim_operation_ids: "op-original-confirm"
)
# 2. If a purge is required (irreversible), use the purge registry
```

### Why

Per ADR-0001, the event ledger is the **sole semantic authority**. A
readable claim is `project(replay(ledger)) - purge_registry.denied_ids`.
Editing Markdown does not change the ledger; the claim reappears on the
next projection rebuild.

### Recovery

- Restore the Markdown from git (the edit was wrong).
- Use `brain_supersede` or the purge registry as appropriate.

---

## 13. Forcing a tool call when `brain_status` shows degraded schema_version

### Wrong

```
brain_status → schema_version: expected=2, actual=1, status: degraded
# Agent ignores and calls brain_capture(...)
```

### Right

```
# STOP. Do not write.
# 1. Read references/architecture.md § vnext Authority Model
# 2. Surface the situation to the user:
"brain_status shows schema_version mismatch (expected=2, actual=1).
 Writes are blocked until the schema upgrade runner completes.
 Recommend running the upgrade before any mutation."
# 3. Wait for the upgrade; only then resume writes.
```

### Why

A degraded schema_version means the projector cannot correctly replay
the ledger. Writes during this window may produce projections that are
incompatible with the new schema, causing data loss during the upgrade.

### Recovery

- Do not write.
- Run the schema upgrade runner (see `docs/` for the upgrade procedure).
- Verify `brain_status` returns healthy before resuming.

---

## 14. Skipping session bootstrap (profile/rules not loaded)

### Wrong

```
# Agent receives "save this concept: ..." and immediately:
wiki_content_write(...)
```

### Right

```
# Mandatory first calls:
wiki_spaces_list()
profile_get()
brain_status()
# Then proceed with the user's task
```

### Why

Without profile loaded, the agent does not know the operator's hard
rules (commit message style, redaction preferences, language, etc.) and
may violate them silently. Without `brain_status`, a degraded ledger
goes unnoticed.

### Recovery

- Stop, run the bootstrap, re-evaluate the action under the loaded
  rules.

---

## 15. Writing to a wiki without first reading the current state

### Wrong

```
# User: "update the MoE concept page with this new finding"
wiki_content_write(uri: "concepts/moe", content: "<entirely new content>")
# ❌ overwrites prior knowledge without merging
```

### Right

```
# 1. Read current state:
wiki_content_read(uri: "concepts/moe")
# 2. Compute the diff (what's new, what stays, what's superseded)
# 3. Write the merged content
# 4. If a prior claim is now wrong: supersede it via brain_supersede
#    rather than silently rewriting history
```

### Why

Overwriting without reading loses the synthesis that made the concept
valuable in the first place. The DKR pattern (BLUEPRINT §0, §2.2)
requires accumulation, not replacement.

### Recovery

- `wiki_history` + `audit_history` to recover prior content.
- Reconcile the lost content manually.

---

## 16. Writing literal `[[double-bracket]]` text into a page body

### Wrong

```markdown
Add a new entry:

```toml
[[wikis]]
name = "brain"
```
```

`wiki_ingest` reports `broken link in body_links: wikis` — the page now
fails `wiki_lint`.

### Right

Describe the syntax in words, or break up the bracket pair so the raw
substring `[[...]]` never appears verbatim in the body — e.g. "a TOML
array-of-tables header named `wikis`" instead of showing the literal
`[[wikis]]`. If the exact syntax must be shown, put it in a *different*
document that isn't itself a wiki page (a skill reference file, the repo's
own docs) and link to that from the wiki page.

### Why

The `[[slug]]` wikilink scanner matches on the raw Markdown source text,
not the rendered AST — it does **not** respect fenced code blocks or
inline code spans. Any literal `[[...]]` substring, including TOML's
array-of-tables header syntax, HTML/JS array-of-arrays, or a citation
style using double brackets, is read as a wikilink attempt and flagged
broken if no page with that slug exists. This is a real engine behavior
(confirmed 2026-07-19 while authoring a Docker troubleshooting page that
needed to show `[[wikis]]` TOML syntax), not a hypothetical.

### Recovery

- Rewrite the offending line to avoid the literal `[[...]]` substring.
- Re-run `wiki_ingest` then `wiki_lint` to confirm the error clears.

---

## Pattern: when in doubt, escalate

If you encounter a situation not covered here:

1. **Stop** the action.
2. **Read** `references/architecture.md` and `references/type-system.md`.
3. **Surface** the ambiguity to the user with concrete options.
4. **Document** the new anti-pattern here once resolved.

The cost of asking is bounded. The cost of a silent irreversible write
is unbounded.

---

## 17. Free-form `domain`/`kind` strings during extraction (taxonomy drift)

### Wrong

Letting the extraction prompt invent taxonomy per run: one chunk emits
`domain: financial`, another `domain: Finance`; kinds drift across
`financial` / `financial_metric` / `metric` for the same category of fact.

### Right

Pin a closed vocabulary in the extraction prompt/config and reuse the
EXACT strings already in the store (check with `brain_get`/`brain_search`
first). Current canon: domains lowercase (`business`, `financial`,
`project`, `personal`); kinds from one list (`financial_metric`,
`valuation_metric`, `valuation_ratio`, `market_share`, `operational`,
`location`, `ranking`).

### Why

The entity model is **(domain, canonical_subject)** — a domain typo mints a
brand-new entity for the same subject, and `entity/merge` **refuses
cross-domain merges by design** (`cannot merge across domains`). A stray
`Finance` entity cannot be merged back into `financial`; the only cleanup
is retract + re-capture. Seen live 2026-07-19: CATL split across 3
entities from one ingest run.

### Recovery

Supersede/retract the claims under the stray domain from the same client
that confirmed them, re-capture under the canonical domain, then leave the
empty stray entity (harmless) or purge per policy.

---

## 18. Packing two facts into one claim value

### Wrong

`predicate: "battery cost", value: "$60 vs $69/kWh"` — CATL's cost AND the
industry average fused into one string.

### Right

One atomic fact per claim: `battery cost = "$60/kWh"` for subject CATL;
the comparator is a separate claim on its own subject (or stays as prose
in the entity page).

### Why

Packed values can't be queried, compared, or superseded independently —
when one half changes, the whole claim goes stale and the ledger diff is
meaningless.

### Recovery

`brain_capture` the atomic fact with the SAME (domain, subject, predicate)
scope, then `brain_supersede` the packed claim. Note supersede is
**scope-locked** — it cannot rename a predicate, so reuse the existing
predicate string.

---

## 19. Re-stating the same fact under a second predicate (duplicate claims)

### Wrong

Chunked extraction confirms `"ROIC vs WACC spread" = "11pp"` (chunk 0) and
`"ROIC - WACC" = "+11.1 pp"` (chunk 1) — same fact, two predicates, two
values, even two domains.

### Right

Before confirming a batch, `brain_search` the subject and dedupe against
existing predicates. One fact → one predicate, canonical spelling.

### Why

Duplicates diverge (11pp vs +11.1pp), double-count in the galaxy/graph,
and CANNOT be collapsed later in one step: supersede is scope-locked to a
single (domain, subject, predicate), so cross-predicate dupes need a
per-scope supersede plus a retract of the loser.

### Recovery

Supersede the claim whose scope matches the canonical predicate with the
precise value; retract the other from the client that confirmed it.

---

## 20. `current X` predicates + confidence 1.0 on extracted values

### Wrong

`predicate: "current stock price", value: "CNY 361", confidence: 1.0`
extracted from a report the source page itself grades 0.75.

### Right

Time-bound facts either carry the as-of in the claim (predicate or value,
e.g. `stock price (as of 2026-07-08)`) or accept a standing duty to
`brain_supersede` on every refresh. Extracted-claim confidence must be
**≤ the source's own confidence** — 1.0 is reserved for facts the operator
asserted directly.

### Why

"current" rots silently; a stale claim with confidence 1.0 is
indistinguishable from a fresh one to every downstream consumer —
accuracy-first breaks exactly where it matters.

### Recovery

Supersede with a dated value and honest confidence; keep the old claim as
history (that is what the bitemporal ledger is for).

---

## 21. Managing MCP-confirmed claims from the Console (client-scope trap)

### Wrong

Extraction confirms claims via the MCP client (`__bootstrap__`), then the
operator tries `POST /api/v1/claim/{confirm-op}/retract` from the Console
session and gets `not_found`, concluding the claim is gone or the API is
broken.

### Right

Operation ids resolve scoped to **(owner_id, client_id)** — manage a claim
from the SAME client that confirmed it: MCP-confirmed → retract/supersede
via MCP tools; Console-confirmed → Console routes. Find a claim's confirm
operation id via `/entity/timeline` (proposal side) or the ledger.

### Why

`stored_outcome` filters `WHERE owner_id AND client_id AND operation_id`;
a cross-client lookup is a scope miss, not a missing claim. As of
2026-07-19 there is **no MCP retract tool** — supersede is the only
MCP-side lever, and it is scope-locked (see #18/#19). Cross-client retract
of an extraction claim currently has no path; treat that as a known
product gap, not operator error.

### Recovery

Re-issue the operation from the correct client. If no tool exists on that
client (MCP retract), supersede within scope or surface the gap to the
user instead of improvising ledger edits.
