# Task 0.3 Coupling Decision

Decision: **EXTEND**

Date: 2026-07-14  
Baseline: `9521f87e88fd57a2f1a254cd6474e883426814d5`  
Branch: `vnext/phase-0`

## Decision

Retain the existing Rust crate and transport/wiki/index components. Add the semantic authority as an isolated Application Core module and route future semantic commands to that interface without dual-writing. Task 0.3 does not wire the module into MCP, HTTP, ACP, Markdown, Git, Tantivy, or Petgraph.

All common gates required by the neutral decision rule pass. The same-crate coupling gate also passes, so `REPLATFORM_SEMANTIC_CORE` is not justified.

## Executable coupling evidence

- CodeGraph was refreshed after implementation: 140 files, 2,372 nodes, and 6,657 edges. The only callers of private `mutate` are `SemanticStore::capture`, `propose`, and `confirm` in `src/semantic.rs`.
- CodeGraph flow is `SemanticApplicationCore -> SemanticStore::{capture,propose,confirm} -> mutate -> mutate_once`. No legacy runtime symbol appears in that flow.
- The architecture integration test rejects imports/calls to `ops`, `markdown`, `git`, `index_manager`, `mcp`, `server`, Tantivy, and Petgraph from `src/semantic.rs`.
- The same test scans existing top-level runtime files and proves none calls `semantic::`. `src/lib.rs` only exports the module.
- Existing `content_write` remains unchanged and still resolves a wiki URI then calls `markdown::write_page`. There is no semantic dual write.
- `SemanticApplicationCore` is the narrow replacement seam future transports can call. Its implementation delegates only to the isolated semantic authority.

## Reused and new boundaries

| Area | Decision |
|---|---|
| Rust crate and existing CLI/MCP/ACP/Axum boundary | Reuse unchanged |
| Markdown/Git authored wiki | Reuse unchanged; not semantic authority |
| Tantivy/Petgraph and generated views | Reuse later as outbox-driven replaceable projections |
| Semantic transitions | New `SemanticApplicationCore` and `SemanticStore` |
| Authority/idempotency/outbox | SQLite WAL, `synchronous=FULL`, `BEGIN IMMEDIATE` |
| Evidence/claim objects | SHA-256 addressed immutable files using RFC 8785 JCS for semantic JSON |
| Read model | Atomically replaced, checksummed projection snapshot; rebuildable from ledger |
| Rollback | Opaque store-bound admin capability; isolated fixture root only |

## Migration and rollback evidence

No production or registered wiki data was read, migrated, or dual-written. Tests create caller-supplied temporary roots only. The disabled default returns before canonicalization or directory creation.

Rollback validates the canonical allowed parent, store UUID, deletion nonce, marker, database identity, symlink/reparse and `.git` hazards. It takes non-blocking exclusive maintenance ownership, rejects live handles/transactions, takes a SQLite maintenance transaction, and then removes only the bound spike root. Tests prove the legacy fixture bytes remain unchanged and repeat cleanup is safe.

Repository rollback is limited to Task 0.3 permitted-path commits and returns the branch to `9521f87e...`.

## Known limitations and follow-up

- This is a local, single-host SQLite spike. WAL is not a multi-host/network-filesystem lease. Phase 1 must keep a single semantic writer topology or add a durable external coordinator before scale-out.
- The trusted context is store-generated and local; production identity/OIDC/capability integration remains gated by ADR-0001 and later phases.
- The object store is plaintext in this spike. Encryption, purge registry, key destruction, and backup denial are later mandatory security phases; no remote API exposes these objects now.
- Projection is a deliberately minimal local snapshot proving ordering/checkpoint/atomic replacement, not the future Markdown/Tantivy/Petgraph projector.
- Existing RustSec findings and unauthenticated HTTP exposure are unchanged and remain release blockers; Task 0.3 adds no remote surface.

## Decision trigger for reconsideration

Reconsider `REPLATFORM_SEMANTIC_CORE` only if a later executable integration proves that routing transports through `SemanticApplicationCore` requires a second authority or unavoidable coupling to the legacy writer. Tooling, performance, coverage, or dependency failures are not replatform evidence.
