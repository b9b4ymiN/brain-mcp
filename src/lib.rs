//! Git-backed wiki engine. Full-text search, typed pages, concept graph,
//! MCP and ACP transports. The CLI is the primary interface; this crate also
//! exposes the engine internals for embedding or testing.

/// ACP (Agent Client Protocol) transport and session handling.
pub mod acp;
/// CLI argument structs and subcommand enums.
pub mod cli;
/// Global and per-wiki configuration types and loaders.
pub mod config;
/// Smart Console contract (Task 5.1) — review-workflow types, diff preview,
/// XSS-safe text rendering. The Console is a first-party app calling the same
/// API as MCP; it never writes SQLite/Git/index directly (§9).
pub mod console;
/// Consolidation + domain evals (Task 4.3) — duplicate/contradiction/stale
/// detection (review queue, never auto-apply), auto-approve policy (off by
/// default, per-type threshold), domain eval report, and model/prompt
/// regression report.
pub mod consolidation;
pub mod default_schemas;
/// Reproducible deployment contract (Task 6.1) — Docker Compose manifest,
/// health/readiness gates, secret references, platform targets.
pub mod deployment;
/// Central wiki engine — mounts spaces and manages indexes.
pub mod engine;
/// Evidence-linked extraction pipeline (Task 4.2) — source→spans→typed
/// proposals with provenance, prompt-injection resistance, and a
/// local-only/secret egress gate. The worker yields proposals only; it has
/// no commit capability (§4 rule 5: AI proposes, policy commits).
pub mod extraction;
/// Frontmatter parsing, scaffolding, and serialization helpers.
pub mod frontmatter;
/// Galaxy 3D graph contract (Task 5.2) — bounded subgraph data model, LOD
/// (semantic zoom), renderer-fallback chain. §9.2.
pub mod galaxy;
/// Git commit, history, and change-detection helpers.
pub mod git;
/// Concept graph construction, community detection, and renderers.
pub mod graph;
/// Tantivy index lifecycle manager for a single wiki space.
pub mod index_manager;
/// Tantivy schema builder and field classification.
pub mod index_schema;
/// File ingestion, validation, and optional redaction.
pub mod ingest;
/// Wikilink and cross-wiki link extraction and classification.
pub mod links;
/// Markdown page read/write, asset, and scaffolding helpers.
pub mod markdown;
/// MCP server and tool handlers.
pub mod mcp;
/// High-level operations called by CLI and server handlers.
pub mod ops;
/// Projection adapters — Tantivy/Petgraph/generated Markdown built from the
/// canonical semantic layers (event ledger + claim snapshots).
pub mod projection;
/// AI provider boundary — domain trait + config + outbound policy + error
/// coverage (Task 4.1). Provider-agnostic: no Z.ai-specific field in the
/// domain core. Real HTTP termination is a deployment adapter.
pub mod provider;
/// Full-text BM25 search and paginated list operations.
pub mod search;
/// Feature-flagged semantic event ledger and application-core boundary.
pub mod semantic;
/// HTTP and stdio server entry points.
pub mod server;
/// Slug validation, resolution, and URI parsing.
pub mod slug;
/// Builds SpaceTypeRegistry and IndexSchema from schema files.
pub mod space_builder;
/// Wiki space creation, registration, and management.
pub mod spaces;
/// Trust + operations views contract (Task 5.3) — trust flags, retrieval
/// trace, provenance answers, job/backup summaries, destructive-action
/// warnings. §5.3 + §10.
pub mod trust;
/// Per-wiki type registry — schema compilation and validation.
pub mod type_registry;
/// Filesystem watcher for auto-ingest on file save.
pub mod watch;
/// Embedded Hugo CMS web preview scaffold and runners.
pub mod web;
