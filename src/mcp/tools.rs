use std::sync::Arc;

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use super::McpServer;
use super::handlers;
use super::helpers::{ToolResult, err_text};

// ── Schema helpers ────────────────────────────────────────────────────────────

fn schema(props: Value, required: &[&str]) -> Arc<Map<String, Value>> {
    let req: Vec<Value> = required
        .iter()
        .map(|s| Value::String(s.to_string()))
        .collect();
    let obj = json!({
        "type": "object",
        "properties": props,
        "required": req,
    });
    Arc::new(obj.as_object().unwrap().clone())
}

fn str_prop(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn opt_str(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn opt_bool(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

fn opt_int(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}

// ── Tool annotations (Task 3.1) ───────────────────────────────────────────────
//
// GOAL-vNext §7.2 makes MCP annotations mandatory: every tool carries
// `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint`
// so clients can reason about read-only vs destructive calls without
// probing. Rather than thread a fourth arg through every `Tool::new` call
// (which would touch all 29 declarations), we attach annotations in a
// single post-processing pass keyed on the tool name.

fn read_only() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn write_additive() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false)
}

fn write_idempotent() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(false)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn write_destructive() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(false)
        .destructive(true)
        .idempotent(false)
        .open_world(false)
}

/// Map a tool name to its §7.2 annotation profile. Unknown tools default to
/// the most conservative non-read-only profile (write-additive) so a future
/// tool is never accidentally hinted as read-only.
fn annotations_for(name: &str) -> ToolAnnotations {
    match name {
        // Read-only tools — no environment mutation.
        "wiki_search" | "wiki_list" | "wiki_content_read" | "wiki_history" | "wiki_stats"
        | "wiki_graph" | "wiki_resolve" | "wiki_lint" | "wiki_suggest" | "profile_get"
        | "semantic_search" | "semantic_get" | "procedural_find" | "procedural_get"
        | "graph_neighbors" | "audit_history" | "wiki_index_status" | "brain_status"
        | "brain_search" | "brain_get" => read_only(),
        // brain_* mutations (Phase C C2 / Phase D D2/D3)
        "brain_capture"
        | "brain_confirm"
        | "brain_supersede"
        | "brain_propose"
        | "brain_ingest_source"
        | "brain_extract" => write_additive(),
        // Destructive — removes data. `wiki_schema` is a multi-action tool
        // whose `action: remove` path can delete a schema file AND page files
        // from disk (`delete_pages: true`), so it is classified by its most
        // dangerous action rather than its read paths (list/show/validate).
        // Splitting it into separate read/write tools is deferred; until then
        // the worst-case-safe destructive hint prevents a client from
        // auto-approving it based on a read-only hint.
        "wiki_spaces_remove" | "wiki_schema" => write_destructive(),
        // Idempotent mutations — safe to retry with the same args.
        "wiki_index_rebuild" | "wiki_config" | "wiki_spaces_set_default" => write_idempotent(),
        // Additive/side-effecting mutations — not destructive, not idempotent.
        // create/register/list are space-management; write/new/commit/ingest
        // touch content; export writes a file.
        "wiki_spaces_create"
        | "wiki_spaces_register"
        | "wiki_spaces_list"
        | "wiki_content_write"
        | "wiki_content_new"
        | "wiki_content_commit"
        | "wiki_ingest"
        | "wiki_export" => write_additive(),
        _ => write_additive(),
    }
}

// ── Tool definitions ─────────────────────────────────────────────────────────

/// Return the complete list of MCP tool definitions for registration.
pub fn tool_list() -> Vec<Tool> {
    let tools = vec![
        Tool::new(
            "wiki_spaces_create",
            "Initialize a new wiki repository",
            schema(
                json!({
                    "path": str_prop("Path to create the wiki at"),
                    "name": str_prop("Wiki name — used in wiki:// URIs"),
                    "description": opt_str("Optional one-line description"),
                    "force": opt_bool("Update space entry if name already exists"),
                    "set_default": opt_bool("Set as default wiki"),
                    "wiki_root": opt_str("Content directory relative to repo root (default: \"wiki\")"),
                }),
                &["path", "name"],
            ),
        ),
        Tool::new(
            "wiki_spaces_register",
            "Register an existing wiki repository without creating files",
            schema(
                json!({
                    "path": str_prop("Absolute path to the existing wiki repository"),
                    "name": str_prop("Wiki name — used in wiki:// URIs"),
                    "description": opt_str("Optional one-line description"),
                    "wiki_root": opt_str("Content directory (overrides wiki.toml; must already exist)"),
                }),
                &["path", "name"],
            ),
        ),
        Tool::new(
            "wiki_spaces_list",
            "List all registered wiki spaces",
            schema(
                json!({
                    "name": opt_str("Wiki name (omit for all)"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_spaces_remove",
            "Remove a wiki space",
            schema(
                json!({
                    "name": str_prop("Wiki name to remove"),
                    "delete": opt_bool("Also delete the wiki directory from disk"),
                }),
                &["name"],
            ),
        ),
        Tool::new(
            "wiki_spaces_set_default",
            "Set the default wiki space",
            schema(
                json!({
                    "name": str_prop("Wiki name to set as default"),
                }),
                &["name"],
            ),
        ),
        Tool::new(
            "wiki_config",
            "Get or set configuration values",
            schema(
                json!({
                    "action": str_prop("Action: get, set, or list"),
                    "key": opt_str("Config key (for get/set)"),
                    "value": opt_str("Config value (for set)"),
                    "global": opt_bool("Write to global config"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["action"],
            ),
        ),
        Tool::new(
            "wiki_content_read",
            "Read full content of a page by slug or URI",
            schema(
                json!({
                    "uri": str_prop("Slug or wiki:// URI"),
                    "no_frontmatter": opt_bool("Strip frontmatter from output"),
                    "list_assets": opt_bool("List co-located assets instead of content"),
                    "backlinks": opt_bool("Include incoming links — pages that link to this page"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["uri"],
            ),
        ),
        Tool::new(
            "wiki_content_write",
            "Write content to a page in the wiki tree; bare slugs are canonicalized from frontmatter type into the Blueprint layout",
            schema(
                json!({
                    "uri": str_prop("Slug or wiki:// URI. Prefer explicit paths such as concepts/topic; bare slugs are placed by frontmatter type."),
                    "content": str_prop("File content"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["uri", "content"],
            ),
        ),
        Tool::new(
            "wiki_content_new",
            "Create a page or section with scaffolded frontmatter; bare page slugs are placed in the Blueprint layout",
            schema(
                json!({
                    "uri": str_prop("Slug or wiki:// URI. Prefer explicit paths such as concepts/topic; bare page slugs default to concepts/."),
                    "section": opt_bool("Create a section instead of a page"),
                    "bundle": opt_bool("Create as bundle (folder + index.md)"),
                    "name": opt_str("Page title (default: derived from slug)"),
                    "type": opt_str("Page type (default: page)"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["uri"],
            ),
        ),
        Tool::new(
            "wiki_content_commit",
            "Commit pending changes to git; bare legacy slugs are rehomed to the Blueprint layout before commit when their frontmatter type is known",
            schema(
                json!({
                    "slugs": opt_str("Comma-separated page slugs to commit (omit for all)"),
                    "message": opt_str("Commit message"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_search",
            "Full-text BM25 search, returns ranked results",
            schema(
                json!({
                    "query": str_prop("Search query"),
                    "type": opt_str("Filter by frontmatter type"),
                    "no_excerpt": opt_bool("Omit excerpts — refs only"),
                    "include_sections": opt_bool("Include section index pages"),
                    "top_k": opt_int("Max results"),
                    "wiki": opt_str("Target wiki name"),
                    "cross_wiki": opt_bool("Search across all wikis"),
                    "format": opt_str("Output format: json | llms (default: json)"),
                }),
                &["query"],
            ),
        ),
        Tool::new(
            "wiki_list",
            "Paginated page listing with filters",
            schema(
                json!({
                    "type": opt_str("Filter by frontmatter type"),
                    "status": opt_str("Filter by frontmatter status"),
                    "page": opt_int("Page number, 1-based"),
                    "page_size": opt_int("Results per page"),
                    "wiki": opt_str("Target wiki name"),
                    "format": opt_str("Output format: json | llms (default: json)"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_ingest",
            "Validate, commit, and index files in the wiki tree",
            schema(
                json!({
                    "path": str_prop("File or folder path, relative to wiki root"),
                    "dry_run": opt_bool("Show what would be created without creating"),
                    "redact": opt_bool("Run redaction pass on file bodies before validation (opt-in; lossy — original values are replaced)"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["path"],
            ),
        ),
        Tool::new(
            "wiki_index_rebuild",
            "Rebuild the tantivy search index",
            schema(
                json!({
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_index_status",
            "Inspect index health",
            schema(
                json!({
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_graph",
            "Generate concept graph, returns GraphReport",
            schema(
                json!({
                    "format": opt_str("Output format: mermaid | dot | llms (default: mermaid)"),
                    "root": opt_str("Subgraph from this node (slug)"),
                    "depth": opt_int("Hop limit from root"),
                    "type": opt_str("Comma-separated page types to include"),
                    "relation": opt_str("Filter edges by relation label"),
                    "output": opt_str("File path for output (default: stdout/return)"),
                    "cross_wiki": opt_bool("Merge all mounted wikis into a unified graph"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_export",
            "Export the full wiki to a file (llms.txt, llms-full, or json)",
            schema(
                json!({
                    "wiki": str_prop("Target wiki name"),
                    "path": opt_str("Output path (relative to wiki root or absolute; default: llms.txt)"),
                    "format": opt_str("Export format: llms-txt | llms-full | json (default: llms-txt)"),
                    "status": opt_str("Page status filter: active | all (default: active, excludes archived)"),
                }),
                &["wiki"],
            ),
        ),
        Tool::new(
            "wiki_history",
            "Git commit history for a page",
            schema(
                json!({
                    "slug": str_prop("Slug or wiki:// URI"),
                    "limit": opt_int("Max entries to return"),
                    "follow": opt_bool("Track renames (default: from config)"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["slug"],
            ),
        ),
        Tool::new(
            "wiki_stats",
            "Wiki health dashboard — page counts, graph metrics, staleness, structural topology (diameter, radius, center)",
            schema(
                json!({
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_suggest",
            "Suggest related pages to link",
            schema(
                json!({
                    "slug": str_prop("Slug or wiki:// URI"),
                    "limit": opt_int("Max suggestions"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["slug"],
            ),
        ),
        Tool::new(
            "wiki_lint",
            "Run deterministic lint rules on the wiki index",
            schema(
                json!({
                    "rules": opt_str("Comma-separated rule names: orphan, broken-link, broken-cross-wiki-link, missing-fields, stale, unknown-type, articulation-point, bridge, periphery (omit for all)"),
                    "severity": opt_str("Filter output: error | warning (omit for all)"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "wiki_resolve",
            "Resolve a slug or wiki:// URI to its local filesystem path. Use before writing content directly to disk.",
            schema(
                json!({
                    "uri": str_prop("Slug or wiki:// URI"),
                    "wiki": opt_str("Target wiki name (optional, uses default)"),
                }),
                &["uri"],
            ),
        ),
        Tool::new(
            "wiki_schema",
            "Inspect and manage type schemas",
            schema(
                json!({
                    "action": str_prop("Action: list, show, add, remove, validate"),
                    "type": opt_str("Type name (for show/add/remove/validate)"),
                    "template": opt_bool("Return frontmatter template instead of schema (for show)"),
                    "schema_path": opt_str("Path to schema file (for add)"),
                    "delete": opt_bool("Also delete schema file (for remove)"),
                    "delete_pages": opt_bool("Also delete page files from disk (for remove)"),
                    "dry_run": opt_bool("Show what would be done (for remove)"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["action"],
            ),
        ),
        Tool::new(
            "profile_get",
            "Read active operator profile pages, optionally scoped to one section",
            schema(
                json!({
                    "section": opt_str("Profile section: rules | identity | style | stack | constraints"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &[],
            ),
        ),
        Tool::new(
            "semantic_search",
            "Blueprint read-tier alias for BM25 semantic wiki search. Vector/rerank is not enabled yet.",
            schema(
                json!({
                    "query": str_prop("Search query"),
                    "top_k": opt_int("Max results"),
                    "type": opt_str("Optional semantic type filter: concept | entity | source | project | decision"),
                    "wiki": opt_str("Target wiki name"),
                    "format": opt_str("Output format: json | llms (default: json)"),
                }),
                &["query"],
            ),
        ),
        Tool::new(
            "semantic_get",
            "Read a semantic page by page_id or URI",
            schema(
                json!({
                    "page_id": str_prop("Page slug or wiki:// URI"),
                    "with_backlinks": opt_bool("Include incoming links"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["page_id"],
            ),
        ),
        Tool::new(
            "procedural_find",
            "Find procedure runbooks matching an intent",
            schema(
                json!({
                    "intent": str_prop("Natural-language procedure intent"),
                    "context": opt_str("Additional context to append to the search query"),
                    "top_k": opt_int("Max results"),
                    "wiki": opt_str("Target wiki name"),
                    "format": opt_str("Output format: json | llms (default: json)"),
                }),
                &["intent"],
            ),
        ),
        Tool::new(
            "procedural_get",
            "Read a procedure runbook by proc_id or URI",
            schema(
                json!({
                    "proc_id": str_prop("Procedure slug or wiki:// URI"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["proc_id"],
            ),
        ),
        Tool::new(
            "graph_neighbors",
            "Read related pages around a root page",
            schema(
                json!({
                    "page_id": str_prop("Root page slug or URI"),
                    "depth": opt_int("Hop depth"),
                    "edge_types": opt_str("Relation label filter"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["page_id"],
            ),
        ),
        Tool::new(
            "audit_history",
            "Read git audit history for a page",
            schema(
                json!({
                    "path": str_prop("Page slug or wiki:// URI"),
                    "limit": opt_int("Max entries"),
                    "wiki": opt_str("Target wiki name"),
                }),
                &["path"],
            ),
        ),
        // ── brain_ingest_source (Task D3) ──────────────────────────────────
        Tool::new(
            "brain_ingest_source",
            "Quarantine a URL/file/text source, chunked one capture per paragraph-packed chunk (SSRF-guarded for URLs)",
            schema(
                json!({
                    "operation_id": str_prop("Base operation id (for idempotency) — each chunk gets '{operation_id}-chunk-N'"),
                    "text": opt_str("Raw text to ingest (exactly one of text/url/file_path required)"),
                    "url": opt_str("URL to fetch and ingest (http/https only, SSRF-guarded)"),
                    "file_path": opt_str("File path (relative to the wiki root, or absolute inside it) to ingest"),
                    "max_chunk_bytes": opt_int("Max bytes per chunk before packing the next paragraph into a new chunk (default 4000)"),
                    "wiki": opt_str("Target wiki name (used to resolve file_path)"),
                }),
                &["operation_id"],
            ),
        ),
        Tool::new(
            "brain_extract",
            "Extract claims from a quarantined chunk via the real AI provider, validate evidence against actual rendition bytes, and propose the supported ones",
            schema(
                json!({
                    "capture_operation_id": str_prop("The chunk capture operation id (from brain_ingest_source) to extract from"),
                    "method": str_prop("Extraction method label (e.g. llm_extraction)"),
                    "model": opt_str("Model name/id used for the inference"),
                    "prompt_version": opt_str("Prompt version identifier (default: the built-in extraction prompt version)"),
                    "local_only": opt_bool("Never send this chunk to the AI provider — deny the call outright (default false)"),
                }),
                &["capture_operation_id", "method"],
            ),
        ),
        // ── brain_* semantic tools (Phase C Task C1) ──────────────────────
        Tool::new(
            "brain_status",
            "Brain health: ledger head, claim count, schema version",
            schema(json!({}), &[]),
        ),
        Tool::new(
            "brain_search",
            "Search confirmed claims in the semantic brain",
            schema(
                json!({
                    "query": str_prop("Search query (subject or predicate substring)"),
                    "domain": opt_str("Filter by domain (e.g. stocks, projects)"),
                    "top_k": opt_int("Max results (default 10)"),
                }),
                &["query"],
            ),
        ),
        Tool::new(
            "brain_get",
            "Read claims for a subject from the semantic brain",
            schema(
                json!({
                    "subject": str_prop("Subject to look up"),
                    "domain": opt_str("Filter by domain"),
                }),
                &["subject"],
            ),
        ),
        // brain_* mutation tools (Phase C Task C2)
        Tool::new(
            "brain_capture",
            "Capture a user utterance as a proposed claim in the semantic brain",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id (for idempotency)"),
                    "utterance": str_prop("The user's exact utterance text"),
                    "subject": str_prop("Claim subject"),
                    "predicate": str_prop("Claim predicate"),
                    "value": str_prop("Claim value"),
                    "domain": opt_str("Optional domain tag (e.g. stocks, projects). Entity Identity Reform: domain is a categorization tag, not part of entity identity."),
                    "claim_kind": opt_str("Claim kind (default: user_assertion)"),
                }),
                &[
                    "operation_id",
                    "utterance",
                    "subject",
                    "predicate",
                    "value",
                ],
            ),
        ),
        Tool::new(
            "brain_confirm",
            "Confirm a proposed claim in the semantic brain",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id"),
                    "proposal_operation_id": str_prop("The capture operation id to confirm"),
                }),
                &["operation_id", "proposal_operation_id"],
            ),
        ),
        Tool::new(
            "brain_supersede",
            "Supersede an existing confirmed claim with a new one",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id"),
                    "proposal_operation_id": str_prop("The propose operation id for the new claim"),
                    "superseded_claim_operation_ids": str_prop("Comma-separated confirm operation ids to supersede"),
                }),
                &[
                    "operation_id",
                    "proposal_operation_id",
                    "superseded_claim_operation_ids",
                ],
            ),
        ),
        Tool::new(
            "brain_propose",
            "Propose a claim derived by inference (AI worker write path) — always status 'proposed', never auto-confirmed",
            schema(
                json!({
                    "operation_id": str_prop("Unique operation id (for idempotency)"),
                    "subject": str_prop("Claim subject"),
                    "predicate": str_prop("Claim predicate"),
                    "value": str_prop("Claim value"),
                    "domain": opt_str("Optional domain tag (e.g. stocks, projects). Entity Identity Reform: domain is a categorization tag, not part of entity identity."),
                    "method": str_prop("Inference method (e.g. llm_extraction)"),
                    "model": opt_str("Model name/id used for the inference"),
                    "prompt_version": opt_str("Prompt version identifier"),
                    "claim_kind": opt_str("Claim kind (default: inference)"),
                    "evidence_capture_operation_ids": opt_str("Comma-separated capture operation ids backing this inference (omit for an unsupported/no-evidence proposal)"),
                }),
                &[
                    "operation_id",
                    "subject",
                    "predicate",
                    "value",
                    "method",
                ],
            ),
        ),
    ];
    // Task 3.1: attach the §7.2 annotation profile to every tool in a single
    // pass. This keeps the declarations above readable (3-arg Tool::new) and
    // centralizes the read-only/destructive/idempotent policy in one match.
    tools
        .into_iter()
        .map(|tool| {
            let name = tool.name.clone();
            tool.with_annotations(annotations_for(&name))
        })
        .collect()
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch a tool call by name to the appropriate handler, catching panics.
pub fn call(server: &McpServer, name: &str, args: &Map<String, Value>) -> ToolResult {
    let _span = tracing::info_span!("tool_call", tool = name).entered();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match name {
        "wiki_spaces_create" => handlers::handle_spaces_create(server, args),
        "wiki_spaces_register" => handlers::handle_spaces_register(server, args),
        "wiki_spaces_list" => handlers::handle_spaces_list(server, args),
        "wiki_spaces_remove" => handlers::handle_spaces_remove(server, args),
        "wiki_spaces_set_default" => handlers::handle_spaces_set_default(server, args),
        "wiki_config" => handlers::handle_config(server, args),
        "wiki_content_read" => handlers::handle_content_read(server, args),
        "wiki_content_write" => handlers::handle_content_write(server, args),
        "wiki_content_new" => handlers::handle_content_new(server, args),
        "wiki_content_commit" => handlers::handle_content_commit(server, args),
        "wiki_search" => handlers::handle_search(server, args),
        "wiki_list" => handlers::handle_list(server, args),
        "wiki_ingest" => handlers::handle_ingest(server, args),
        "wiki_index_rebuild" => handlers::handle_index_rebuild(server, args),
        "wiki_index_status" => handlers::handle_index_status(server, args),
        "wiki_graph" => handlers::handle_graph(server, args),
        "wiki_history" => handlers::handle_history(server, args),
        "wiki_stats" => handlers::handle_stats(server, args),
        "wiki_lint" => handlers::handle_lint(server, args),
        "wiki_resolve" => handlers::handle_resolve(server, args),
        "wiki_suggest" => handlers::handle_suggest(server, args),
        "wiki_schema" => handlers::handle_schema(server, args),
        "wiki_export" => handlers::handle_export(server, args),
        "profile_get" => handlers::handle_profile_get(server, args),
        "semantic_search" => handlers::handle_semantic_search(server, args),
        "semantic_get" => handlers::handle_semantic_get(server, args),
        "procedural_find" => handlers::handle_procedural_find(server, args),
        "procedural_get" => handlers::handle_procedural_get(server, args),
        "graph_neighbors" => handlers::handle_graph_neighbors(server, args),
        "audit_history" => handlers::handle_audit_history(server, args),
        // brain_* semantic tools (Phase C)
        "brain_status" => handlers::handle_brain_status(server, args),
        "brain_search" => handlers::handle_brain_search(server, args),
        "brain_get" => handlers::handle_brain_get(server, args),
        "brain_capture" => handlers::handle_brain_capture(server, args),
        "brain_confirm" => handlers::handle_brain_confirm(server, args),
        "brain_supersede" => handlers::handle_brain_supersede(server, args),
        "brain_propose" => handlers::handle_brain_propose(server, args),
        "brain_ingest_source" => handlers::handle_brain_ingest_source(server, args),
        "brain_extract" => handlers::handle_brain_extract(server, args),
        _ => Err(format!("unknown tool: {name}")),
    }));
    match result {
        Ok(Ok((content, notify_uris))) => {
            let notify_resources_changed = matches!(
                name,
                "wiki_spaces_create"
                    | "wiki_spaces_register"
                    | "wiki_spaces_remove"
                    | "wiki_spaces_set_default"
            );
            tracing::debug!(tool = name, "tool call ok");
            ToolResult {
                content,
                is_error: false,
                notify_uris,
                notify_resources_changed,
                structured_content: None,
            }
        }
        Ok(Err(msg)) => {
            tracing::warn!(tool = name, error = %msg, "tool call failed");
            ToolResult {
                content: err_text(msg),
                is_error: true,
                notify_uris: vec![],
                notify_resources_changed: false,
                structured_content: None,
            }
        }
        Err(_) => {
            tracing::error!(tool = name, "tool handler panicked");
            ToolResult {
                content: err_text("internal error: tool panicked".into()),
                is_error: true,
                notify_uris: vec![],
                notify_resources_changed: false,
                structured_content: None,
            }
        }
    }
}
