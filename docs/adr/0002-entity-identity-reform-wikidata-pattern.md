# ADR-0002: Entity Identity Reform — Wikidata Pattern

- Status: **Accepted for Phase 0**
- Date: 2026-07-22
- Decision owners: repository owner and Brain Application Core
- Amends: ADR-0001 §Decision 6 (scope key)
- Baseline: post-reform store at schema_version 4

## Context

ADR-0001 §Decision 6 defines the scope key as
`(owner_id, domain, subject_id, predicate, normalized_context)` and notes that
`latest-user-wins` applies only to `preference`, `profile`, `project decision`
claims. The entity identity model inherited from Task 2.2 encoded `domain`
into the entity identity constraint (`UNIQUE(domain, canonical_subject)`),
which — under LLM free-text domain emission — fragmented each real-world
subject into one entity per domain variant. Verified on 2026-07-21: one
company (CATL) appeared as 6 entities; the home galaxy showed 10 phantom
nodes (34 entities for 24 real).

This violates the universally-observed production knowledge-graph identity
pattern (Wikidata QID, MusicBrainz MBID, OSM element ID, GitHub node ID,
OpenAI Temporal Agents UUID): identity is an opaque, immutable,
system-assigned identifier, and every categorization field (label, type,
domain, tags) is mutable metadata layered on top. No production KG has ever
used a categorization field as part of the identity constraint.

## Decision

Drop `domain` from entity identity. Specifically:

1. `entities` UNIQUE constraint becomes `UNIQUE(canonical_subject)` alone.
2. `entity_aliases` PRIMARY KEY becomes `PRIMARY KEY(alias, entity_id)`.
3. `resolve_or_create_entity_in_tx`, `resolve_entity_in_tx`, `insert_alias`,
   and their public wrappers drop the `domain` parameter.
4. `merge_entities` and `split_entities` no longer reject cross-domain
   operations (the guard is removed).
5. `claim_status.domain` is UNCHANGED — it remains a per-claim categorization
   tag, mirroring Wikidata's `instance of` statements, MusicBrainz genre,
   and OSM tags.
6. `ClaimDraft.domain` and `ProposalSummary.domain` become `Option<String>`.
7. The disk schema version bumps 3 → 4. The v3→v4 migration consolidates
   existing fragmented entities (most-claims-wins per canonical_subject),
   folds aliases onto the surviving entity, deletes the losers, and
   recreates both tables without `domain` in the key — all inside one
   atomic transaction.
8. Phase 1.6 conflict detection (`inbox_conflicts::detect_conflicts`)
   buckets by `(subject, predicate)` instead of `(domain, subject,
   predicate)`, so cross-domain conflicts now surface.

## Consequences

- One real-world subject = exactly one entity_id, regardless of how many
  domain variants extraction emits. CATL is now 1 entity, not 6.
- Phase 1.6 conflict detection now surfaces cross-domain conflicts — the
  pre-reform blind spot where "business/WACC" vs "Finance/WACC" was
  invisible is closed.
- The cross-domain merge/split guards are gone — any two entities can merge.
- `claim_timeline` accepts `domain` as an optional filter, not a required
  key. `None` returns every claim for the subject/predicate across domains.
- MCP `brain_capture` and `brain_propose` accept `domain` as an optional
  argument. Existing callers that send `domain` continue to work.
- The v3→v4 migration is reversible at the schema level (rollback restores
  the `domain` column) but cannot un-merge consolidated entities — deleted
  rows are gone. A re-run of forward is the documented way to re-assert
  consolidation.
- The schema-version gate uses transitive reachability
  (`schema_upgrade_reachable`) so a v2 store can be opened for upgrade even
  though the binary is at v4 (it goes 2→3→4 in two upgrade runs).

## Alternatives considered

- **Normalize LLM domain emission (taxonomy v2):** reduces the rate of
  fragmentation but does not fix the constraint. A single LLM-emit domain
  not in the canonical list still fragments. Rejected as a defense-in-depth
  complement, not a replacement.
- **Keep domain in identity, add a deduplication job:** a periodic job that
  merges fragmented entities. This is the Neo4j "resolve at query time"
  anti-pattern (cited in the design doc): fragmentation accumulates and is
  expensive to repair downstream. Rejected.
- **Drop `claim_status.domain` entirely:** removes a useful per-claim
  categorization tag and breaks existing conflict-detection display context.
  Rejected — domain-as-tag is the Wikidata pattern.

## References

- `docs/plans/entity-identity-reform-design-doc.md` (research + examples)
- `docs/plans/2026-07-21-entity-identity-reform-implementation-plan.md` (23-task TDD plan)
- ADR-0001 §Decision 6 (the scope key this amends)
- Wikidata:Identifiers, Help:Items, Help:Merge
- MusicBrainz Identifier, Style/Aliases
- OpenStreetMap Tags, Elements
- OpenAI Temporal Agents with Knowledge Graphs (Cookbook)
- Neo4j Agent Memory: Entity Resolution and Deduplication
