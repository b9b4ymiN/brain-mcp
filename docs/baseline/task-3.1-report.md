# Task 3.1 — Stable MCP tool contracts Report

Status: **PASS** — Independent Validator confirmed (after fix-round) on branch `vnext/phase-0` (RED `1ae31e0`, GREEN `7595873`, fix-round this commit). The first pass returned 1 MEDIUM finding (`wiki_schema` misclassified as read-only despite its destructive `action: remove` path) — closed by reclassifying `wiki_schema` to `write_destructive` and adding it to the destructive-tools test. The validator's LOW observations (pagination test coverage, no handler yet calls `ok_structured`, `wiki_content_write` overwrite judgment) are acknowledged deferrals to Task 3.2. The third DoD bullet (mutations carry operation_id/authority/audit link) is formally carried to Task 3.2 because the `brain_*` mutation surface that carries those properties does not exist yet.

## Why this task exists

Task 3.1 is the contract-layer half of GOAL-vNext §13 Phase 3 Task 3.1 + §7.2: every tool must declare input/output schema, annotations (`readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`), structured content + text fallback, bounded output, and actionable errors. Prior to this task, all 29 `wiki_*`/`semantic_*`/`procedural_*` tools declared only a 3-arg `Tool::new(name, description, schema)` with zero annotations, opaque text-blob outputs, and string-based errors. This task adds the annotation layer, the structured-content plumbing, and the actionable error shaping — without yet wiring the semantic store into the MCP layer (that is Task 3.2, where the `brain_*` surface and operation_id/authority/audit live).

## Scope and TDD evidence

Base: clean `vnext/phase-0` at `811fd16` (Phase 2 Gate closed).

- **RED — `1ae31e0`** ("test(mcp-contracts): Task 3.1 RED checkpoint"). `tests/mcp_tool_contracts_v1.rs` (new, 11 tests) written against the not-yet-annotated tool declarations and the not-yet-existing `structured_content`/`ok_structured`/`err_structured`.
- **GREEN — `7595873`** ("feat(mcp-contracts): Task 3.1 GREEN"). `src/mcp/tools.rs` (annotation profiles + single-pass attachment), `src/mcp/helpers.rs` (`ToolResult.structured_content` + `ok_structured`/`err_structured` + dedup), `src/mcp/mod.rs` (propagate `structured_content` onto `CallToolResult`). All 11 tests passed.
- **Fix-round — this commit.** Closed the Validator's MEDIUM: `wiki_schema` reclassified from `read_only` to `write_destructive` (its `action: remove` with `delete_pages: true` deletes files from disk); added to `destructive_tools_declare_destructive_hint`; removed from `read_only_tools_declare_read_only_hint`.

## DoD verification

### Bullet 1 — schemas/annotations/errors pass schema tests (MET, pagination deferred)
- `every_tool_has_annotations`, `every_tool_input_schema_is_a_valid_object_schema`, `required_parameters_are_declared_in_properties` — all pass.
- All 30 declared tools carry a non-None `annotations`.
- **Pagination**: cursor-based pagination is deferred to Task 3.2 (semantic wiring). The DoD word "pagination" is only indirectly covered by `wiki_list`'s `page`/`page_size`. Acknowledged LOW deferral.

### Bullet 2 — read tools bounded + structured + text fallback (MET as mechanism; handler wiring deferred)
- `ToolResult.structured_content: Option<Value>` exists; `ok_structured` sets both text + structured; `err_structured` sets both with `{code, message}` + `[CODE]` text; `mod.rs::call_tool` propagates.
- **Deferral (LOW)**: no read-tool handler currently calls `ok_structured` — they still use `ok_text`. The capability is proven; the actual wiring is Task 3.2 (semantic read tier). Reasonable for a contract-layer task.

### Bullet 3 — mutations carry operation_id/authority/audit (DEFERRED to Task 3.2)
- The `brain_*` mutation surface (`brain_capture`/`brain_propose`/`brain_confirm`/`brain_supersede`/`brain_purge`) that carries `operation_id`/authority/audit does not exist yet — the current surface is still `wiki_*`. Demanding operation_id now would be premature. Formally carried to Task 3.2.

## Annotation profiles (§7.2)

Single-pass attachment keyed on tool name, centralized in one `match` so the policy is auditable in one place:

- **read_only** (18 tools): search/list/content_read/history/stats/graph/resolve/lint/suggest/index_status + the blueprint read-tier aliases (profile_get, semantic_search/get, procedural_find/get, graph_neighbors, audit_history).
- **write_destructive** (2 tools): `wiki_spaces_remove` (deletes a directory), `wiki_schema` (multi-action; `action: remove` with `delete_pages: true` deletes page files — classified by worst case, not read paths).
- **write_idempotent** (3 tools): `wiki_index_rebuild`, `wiki_config`, `wiki_spaces_set_default`.
- **write_additive** (7 tools): spaces create/register/list, content write/new/commit, ingest, export.
- **default** (`_`): `write_additive` — a future tool is never accidentally hinted read-only.

## Independent Validator findings (all closed or acknowledged)

- **[MEDIUM, fixed]** `wiki_schema` was `read_only` despite its destructive `remove` path → reclassified to `write_destructive` + test coverage added.
- **[LOW, acknowledged]** Pagination test coverage — deferred to Task 3.2.
- **[LOW, acknowledged]** No handler calls `ok_structured` yet — capability proven, wiring in 3.2.
- **[LOW, acknowledged]** `wiki_content_write` overwrite classified additive — defensible (git-backed, recoverable); future tightening option.
- **[LOW, acknowledged]** `wiki_export` writes a file but is additive — correct (derived artifact, not destructive to wiki data).

## Gates (re-verified independently by Validator)

- `cargo test -j 2 --test mcp_tool_contracts_v1`: 11/11 pass.
- `cargo test -j 2` (default): 0 failed across ~41 binaries.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- Python integration: engine 63 / mcp 76 / acp 26+2skip (== baseline).
- Eval v1: 126/126 cases pass. `byte_lock_passed: false` pre-existing (Task 3.1 touched none of the 9 byte-locked files).
- `cargo audit`: 1 vuln + 3 warnings (== baseline, no new dependencies).
- Isolation: `semantic_vertical_slice` 14/14 pass; `grep -rn "semantic::" src/mcp/ src/server.rs` returns nothing.

## Security review (Independent Validator, all clear)

- **TM-012 (Critical):** no new `semantic::` import in `src/mcp/` — the MCP layer remains cleanly decoupled; semantic wiring is Task 3.2.
- **Annotation safety:** no mutating tool is hinted read-only (after the `wiki_schema` fix). The worst-case classification principle (multi-action tool classified by its most-dangerous action) prevents a client from auto-approving a destructive call based on a read-only hint.

## Carried risks

- **Deferred to Task 3.2**: `brain_*` tool surface, cursor pagination, handler-level `ok_structured` wiring, operation_id/authority/audit link on mutations.
- **Inherited, unchanged**: HIGH per-handle capability enforcement (Phase 3/4); schema break v1→v2 (pre-production); `cargo audit` 1 vuln + 3 warnings (Task 0.1); eval `byte_lock_passed: false` (pre-existing).
- **Toolchain (inherited):** `-j 2` + `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc`.

## Phase 3 status

Task 3.1 (stable contracts) ✅ PASS. **Task 3.2 (stdio + Streamable HTTP, + `brain_*` semantic wiring)** is the next permitted implementation task, pending its own Task Brief review. The Phase 3 Gate (interoperability + auth/security suites pass + contract versioned) remains open until Tasks 3.2 and 3.3 close.
