# Entity Identity Reform — Shipped (Code Complete)

> Date: 2026-07-22
> Status: **CODE COMPLETE — production migration (Task 22) pending operator action**
> Branch: `vnext/phase-0`
> Schema: 3 → 4

## What shipped

Dropped `domain` from the entity identity constraint. One real-world subject
now resolves to exactly one stable `entity_id`, regardless of how many domain
variants LLM extraction emits. This is the Wikidata identity pattern (opaque
ID + mutable categorization tags), the same pattern used by Wikidata,
MusicBrainz, OpenStreetMap, GitHub, and OpenAI Temporal Agents.

## Code changes (Tasks 1–21)

### Schema migration machinery (Tasks 1–5)
- `CURRENT_DISK_SCHEMA_VERSION: 3 → 4`.
- `schema_upgrade_path_exists` recognizes `(3, 4)`; transitive reachability
  (`schema_upgrade_reachable`) lets a v2 store open for upgrade to v4.
- v3→v4 forward migration: consolidates fragmented entities (most-active-claims
  wins per `canonical_subject`, entity_id tiebreak), folds aliases onto the
  survivor, drops the `domain` column from `entities` UNIQUE and
  `entity_aliases` PK via table-recreation. Alias collisions under the new
  `(alias, entity_id)` PK are resolved by keeping `canonical` over other kinds.
- v3→v4 reverse migration: restores the `domain` column (best-effort synthesis
  from `claim_status.domain`). Cannot un-merge consolidated entities.

### Entity functions (Tasks 6–12)
- `resolve_or_create_entity(_in_tx)`, `resolve_entity(_in_tx)`, `insert_alias`:
  drop `domain` parameter.
- `rename_entity`: collision check is subject-only.
- `merge_entities`, `split_entities`: cross-domain guards removed.
- `entity_by_id`: `EntityRecord` drops `domain` field.
- `backfill_entity_ids`: uses 2-arg resolution.

### Wire surface (Tasks 13–17)
- `claim_timeline(domain: Option<&str>, subject, predicate)` — domain is an
  optional filter; `None` returns claims across all domains.
- `ClaimDraft.domain`, `ProposalSummary.domain`: `Option<String>`.
- HTTP `GET /entity/timeline`: `domain` optional query param.
- MCP `brain_capture` + `brain_propose`: `domain` optional (manifest + handler).

### Conflict detection (Task 21)
- `inbox_conflicts::detect_conflicts` bucket key: `(domain, subject, predicate)`
  → `(subject, predicate)`. Cross-domain conflicts now surface.

### Tests (Tasks 18–20)
- All ~22 test files updated for new signatures (mechanical).
- Cross-domain tests inverted: merge/split now succeed.
- New file `tests/semantic_identity_reform_v1.rs` — 4 Wikidata-pattern
  regression tests.
- `tests/semantic_migration_v1.rs` — 6 new v3→v4 tests (path recognition,
  plan body, forward consolidation, forward constraint, reverse, fresh store).

## Verified numbers (test fixtures)

| Metric | Before (v3) | After (v4) |
|--------|-------------|------------|
| Two claims same subject, different domains | 2 entities | **1 entity** |
| Cross-domain merge | rejected | **succeeds** |
| Cross-domain split | rejected | **succeeds** |
| canonical_subject UNIQUE | composite (domain, subject) | **subject alone** |
| entities.domain column | present | **dropped** |
| entity_aliases.domain column | present | **dropped** |
| claim_status.domain column | present | **unchanged** (per-claim tag) |
| Conflict detection cross-domain | invisible | **surfaces** |

## Test results

- `cargo test --workspace`: **all pass, 0 failures** (1 pre-existing ignored).
- `cargo build --lib --bins`: clean, no warnings.

## Production migration (Task 22 — PENDING)

The live store at `./data/semantic-store/semantic.sqlite3` is at v3. The
binary now refuses to serve it until an operator runs:

```bash
cargo run --bin llm-wiki -- recovery upgrade
```

Before running against the live store, rehearse on a backup copy. See the
implementation plan Task 22 for the full procedure.

## Key bugs found and fixed during implementation

1. **Alias PK collision (Task 4):** when a loser's `canonical` alias is folded
   onto the target as `former_subject`, and the target already has a `canonical`
   alias for the same subject, the new `(alias, entity_id)` PK collides. Fixed
   with a window-function dedup that keeps `canonical` over other kinds.
2. **Rollback marker staleness (Task 5):** `execute_schema_upgrade` rewrites the
   marker FILE but not the in-memory `self.marker`. The rollback guard read the
   stale in-memory value. Fixed by reading the on-disk marker.
3. **Transitive schema reachability (Task 6):** a v2 store could not open for
   upgrade when the binary moved to v4 (the gate checked direct `2→4` path).
   Fixed with `schema_upgrade_reachable` (chain walk).

## References

- ADR-0002: `docs/adr/0002-entity-identity-reform-wikidata-pattern.md`
- Design doc: `docs/plans/entity-identity-reform-design-doc.md`
- Implementation plan: `docs/plans/2026-07-21-entity-identity-reform-implementation-plan.md`
