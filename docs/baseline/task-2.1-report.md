# Task 2.1 — Projection Adapters Report

Status: **PASS** — Independent Validator confirmed at HEAD `2258cc8` on branch `vnext/phase-0` (RED `d031110`, GREEN `715f780`, fix round 1 `a7a97b7`, fix round 2 `ac181f8`, docs `2258cc8`), after a first pass returned 2 MEDIUM findings (closed in fix round 2) and 5 LOW/INFO findings (4 deferred with recorded reasoning, 1 fixed as a free fail-safe). The validator independently re-ran every gate and re-read every changed file for the re-verification pass rather than trusting the reported numbers, and returned a final **PASS** with 0 CRITICAL/HIGH/MEDIUM open. Clear to close Task 2.1.

## Fix round 2 — Independent Validator findings closed

The first Independent Validator pass (against `f28ebeb`) returned **FINDINGS**: 0 CRITICAL/HIGH, 2 MEDIUM, 5 LOW/INFO. Closed at `ac181f8`:

- **[MEDIUM, fixed]** `cargo fmt --check` failed on `a7a97b7` — the isolation-test allowlist edit (fix round 1) was never reformatted afterward. `cargo fmt` applied; `cargo fmt --check` reverified clean.
- **[MEDIUM, fixed]** `rebuild_projection`'s "must be a projector-owned directory" contract was documentation-only — nothing stopped a future caller from pointing it at a real, human-authored `wiki_root` and having its content silently `remove_dir_all`'d. Added an `OWNERSHIP_MARKER` (`.projection-owned`) file dropped at the root of every directory the function manages; a pre-existing directory missing the marker is now refused with an error rather than deleted. New test `rebuild_refuses_to_delete_a_directory_it_does_not_own` proves it.
- **[LOW, fixed as a free fail-safe]** Projected claims weren't filtered by `privacy_label`. Currently a no-op in practice (`validate_claim_draft` already rejects any non-`LocalOnly` claim at capture time), but added an explicit filter now so the projector doesn't silently start leaking `PrivateExternalAllowed`/`Publishable` claims the moment a future release-authorization mechanism allows them to exist.
- **Not acted on** (documented, not regressions this task introduced): LOW-4 (the isolation guard test doesn't recurse into `src/` subdirectories — pre-existing structural limitation; independently confirmed nothing under `acp/`/`mcp/`/`ops/` currently references `semantic::`), LOW-5 (unrelated pre-existing untracked fixture files), LOW-6 (composite checksum's three metadata reads are non-atomic — inherited from the purge saga's own identical precedent pattern, not a new regression), LOW-7 (`Utc::now()` called fresh per `rebuild_projection` call in tests rather than pinned once — currently harmless, no time-bounded claim exercises it).

Re-verified after the fix: `cargo fmt --check` clean, `cargo clippy --all-targets --all-features` clean, `cargo test` (default) 646/646, `cargo test --all-features -j 2` 657/657 — 0 failures.

## Why this task exists

Task 2.1 closes the Phase 2 GOAL-vNext.md §13 DoD: Tantivy, Petgraph, and generated Markdown must be buildable from the canonical semantic layers (event ledger + claim snapshots in `src/semantic.rs`) alone, with a projection checkpoint/lag visible, and a delete-then-rebuild property (rebuilding after deleting all projection artifacts reproduces the same composite checksum). This is the first task in the roadmap where anything outside `src/semantic.rs` is authorized to read from it — Phase 1 deliberately kept the semantic module fully isolated from the legacy `brain-mcp` runtime (Tantivy/Petgraph/Markdown/MCP/server) while it was being built and hardened.

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `d93cde8` (Phase 1 Gate, Task 1.3 closed).

- **RED — `d031110`** ("test(projection): RED — Task 2.1 projection adapter contract"). `tests/projection_v1.rs` (new, 376 lines, 8 tests) written against the not-yet-existing `src/projection.rs`. Confirmed RED: `cargo check --test projection_v1` failed with exactly `unresolved import llm_wiki::projection` / `cannot find projection in llm_wiki` (3 errors, all the same missing-module cause) — no other type errors, meaning the test's usage of existing `semantic.rs`/`index_manager.rs`/`graph.rs`/`space_builder.rs` types already matched their real signatures.
- **GREEN — `715f780`** ("feat(projection): Task 2.1 — projection adapters from canonical layers"). `src/projection.rs` (new, 172 lines) + two additive reads on `src/semantic.rs` (`all_claims_current`, `schema_version`) + `src/lib.rs` module registration. 3 files changed, 281 insertions. All 8 new tests passed on the first run.
- **Fix round — `a7a97b7`** ("fix(projection): clippy sort_by_key; allowlist projection.rs in isolation test"). One clippy `unnecessary_sort_by` fix, and a one-entry allowlist addition to `tests/semantic_vertical_slice.rs`'s isolation guard (see "Deviation from the Task Brief" below).

Permitted paths per the Task Brief: `src/projection.rs` (new), `src/semantic.rs` (additive reads only, no transition-logic changes), `src/lib.rs` (module registration), `tests/projection_v1.rs` (new), `docs/baseline/task-2.1-report.md` (this file). The Task Brief also listed `tests/semantic_*.rs` as forbidden; that constraint was revisited mid-task (see below) with the user's explicit approval before `tests/semantic_vertical_slice.rs` was touched.

## Design

**Reuse, not reimplementation.** `src/index_manager.rs::SpaceIndexManager::rebuild()` and `src/graph.rs::build_graph()` already build a Tantivy index and a Petgraph graph from any `wiki_root` directory of frontmatter'd Markdown files — neither one is coupled to the legacy human-authored wiki path specifically, they just walk whatever directory they're given. The only genuinely new piece Task 2.1 needed was the claim-to-Markdown adapter layer that lets `SemanticStore`'s claims become that directory's contents. `src/projection.rs::rebuild_projection()` orchestrates: enumerate active claims as of a given `(ledger_head, world_time)`, delete-and-recreate a caller-owned `generated_wiki_root` directory, render one Markdown page per claim (frontmatter `type: entity`, the "semantic" schema family's generic subtype — mapping every `claim_kind` to a distinct page type is out of scope, nothing in the DoD requires that taxonomy), then call the two pre-existing `rebuild()`/`build_graph()` functions unmodified against that directory.

**Two additive reads on `SemanticStore`.** `all_claims_current(ledger_head, world_time)` is an unscoped sibling of the existing `claims_current` — same TOCTOU-safe single-transaction shape (scope resolution and plaintext materialization in one explicit read transaction, matching the pattern the Task 1.3 security review specifically validated), but without the `WHERE domain=?/subject=?/predicate=?` filter, since a full projection rebuild needs to enumerate every scope, not look one up. `schema_version()` exposes the store's already-existing `marker.schema_version` field, previously only readable from inside `semantic.rs` itself (the purge saga's private `composite_checksum()` reads it directly). Neither method touches any existing transition/mutation code path.

**Composite checksum.** GOAL-vNext.md's DoD names the identity `ledger_head + purge_epoch + schema_version` — the exact triple the hard-purge saga already uses for its own `composite_checksum()` (`src/semantic.rs:2616`, `sha256("{ledger_head}:{purge_epoch}:{schema_version}")`). `ProjectionCheckpoint::composite_checksum` reuses that same triple as a prefix, but extends it with a canonical-bytes fingerprint (`canonicalize_json`, the same public helper the store itself uses) over every projected claim, sorted by `claim_id` for determinism. A pure metadata-only checksum would trivially match on every rebuild regardless of whether the projector's own logic is deterministic or correct — the fingerprint of the actual claims that got rendered is what makes "delete then rebuild reproduces the same checksum" an actual test of `rebuild_projection`, not just of canonical-store stability. This is a documented, deliberate extension of the precedent formula, not a deviation from it — `test-projection-with-no-claims` and the delete/rebuild tests exercise both the metadata prefix and the content fingerprint.

**Checkpoint/lag visibility.** `checkpoint_lag(checkpoint, store)` compares a previously-computed checkpoint's `ledger_head` against the store's current `ledger_head()`, giving callers a staleness signal without forcing a full rebuild. Error visibility is via `rebuild_projection`'s `Result` — no separate persisted error-state table was added; nothing in the DoD requires state to survive past the caller receiving the `Err`.

## What the 8 new tests prove

- `rebuild_projects_active_claims_into_index_and_graph` / `rebuild_with_no_claims_produces_an_empty_projection` — the base case and the empty case both index/graph correctly.
- `delete_then_rebuild_reproduces_the_same_composite_checksum` — deletes both the generated-wiki directory and the Tantivy index directory entirely, rebuilds from the same canonical store, and asserts the composite checksum (and node count) are identical. This is the DoD's central property.
- `a_new_confirmed_claim_changes_the_composite_checksum_on_next_rebuild` — the converse: real ledger growth does change the checksum, so the determinism test above isn't vacuously true.
- `superseded_claim_is_excluded_and_only_the_new_claim_is_projected` / `retracted_claim_is_excluded_from_projection` — bitemporal correctness carries through: only currently-active claims are projected.
- `generated_projection_never_touches_the_human_authored_wiki_root` — writes a human-authored page to a sibling directory before rebuilding into a separate `generated_wiki_root`, asserts the human page's bytes are unchanged afterward.
- `checkpoint_lag_is_zero_immediately_after_rebuild_and_positive_after_a_new_claim` — lag visibility.

## Deviation from the Task Brief: isolation-test allowlist

The approved Task Brief listed `tests/semantic_*.rs` as forbidden paths. Running the full regression suite surfaced a real failure unrelated to any of the new code's correctness: `tests/semantic_vertical_slice.rs::semantic_module_is_isolated_and_legacy_runtime_does_not_call_writer` scans every file in `src/` (excluding `semantic.rs`/`lib.rs`) and asserts none contain the substring `"semantic::"` — a Phase 0/1 guard against the legacy runtime coupling to the semantic module before it was ready. `src/projection.rs` legitimately needs `semantic::{ClaimView, SemanticStore, canonicalize_json}`, which is exactly Task 2.1's stated purpose.

This was flagged to the user rather than silently fixed, given the forbidden-path conflict. The user pointed back to GOAL-vNext.md's own authority: Task 2.1's DoD does not forbid touching test files, and Task 1.3 sub-slice A set direct precedent for a narrow, justified fix inside this exact file (there: cfg-gating two crash tests to fix a pre-existing default-features defect; here: a one-entry allowlist addition). The fix adds `Some("projection.rs")` to the loop's skip condition, with a comment explaining the allowlist is intentionally one entry wide — every other legacy file (`server.rs`, `mcp/`, `engine.rs`, `cli.rs`, `ops/`, ...) remains forbidden from referencing `semantic::`. This is flagged here explicitly for the Independent Validator to check independently: confirm the allowlist is exactly one entry, and that no other file in `src/` references `semantic::`.

## Verification results

| Gate | Result |
|---|---|
| RED (`cargo check --test projection_v1`) | Confirmed failing: `unresolved import llm_wiki::projection` only |
| GREEN (`cargo test --test projection_v1`) | 8/8 passed, first run |
| `cargo fmt --check` | 2 files needed formatting (line wraps only); applied, re-verified clean |
| `cargo clippy --all-targets --all-features` | 1 warning (`unnecessary_sort_by`) → fixed → clean |
| `cargo test` (default features) | 645/645 passed pre-fix-round-2, 646/646 after (new guard test) — 0 failed, 21 binaries |
| `cargo test --all-features` (`-j 2`, worked around a pagefile-exhaustion build failure at full parallelism) | 656/656 passed pre-fix-round-2, 657/657 after — 0 failed, 37 binaries |
| `cargo llvm-cov --all-features` | `projection.rs`: 100.00% lines (77/77), 100.00% functions (4/4), 91.22% regions (135/148). `semantic.rs`: 89.75% lines (consistent with the 89.74% baseline at Task 1.3's close — the two new additive methods did not regress it) |

## Notes and carried risks

- Toolchain: this machine's `rustup` default host resolved to `x86_64-pc-windows-gnu` for this session (missing `dlltool.exe`, breaks `getrandom`'s build script), diverging from this project's established MSVC baseline. Worked around per-command with `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc`; `rust-toolchain.toml` itself was not touched. Not a code issue, but worth the user's attention if CI or another session hits the same default-host drift.
- `cargo test --all-features` at full parallelism failed with a Windows `ERROR_PAGEFILE_INSUFFICIENT` (os error 1455) trying to mmap an already-built rlib while linking 37 test binaries concurrently — an environment/resource constraint, not a code defect. Worked around with `-j 2`. Flagging in case CI or future full-suite runs on this machine hit it again.
- Claim→page-type taxonomy (`type: entity` for every claim regardless of `claim_kind`) is a documented simplification, matching the project's existing pattern of explicitly scoping such decisions out (e.g. Task 1.2's scope-key-without-context-dimension). Revisit if/when Task 2.2's ownership model or a Console view needs richer typing.
- Carried forward, unchanged by this task: the Task 1.3 security review's HIGH finding (capability enforcement is per-method not per-handle) remains scoped to Phase 3/4 auth-boundary wiring, not touched here since `rebuild_projection` only calls read-only `SemanticStore` methods.
