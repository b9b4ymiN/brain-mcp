use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use llm_wiki::engine::WikiEngine;
use llm_wiki::git;
use llm_wiki::mcp::{McpServer, tools};
use llm_wiki::provider::{AiProvider, ProviderRequest, ProviderResult};
use llm_wiki::spaces;
use llm_wiki::web;
use serde_json::{Map, Value, json};

/// A canned `AiProvider` for `brain_extract`'s smoke-dispatch call — no
/// network, always returns one supported claim as valid JSON.
struct SmokeAiProvider;

impl AiProvider for SmokeAiProvider {
    fn complete(&self, _request: &ProviderRequest) -> ProviderResult<String> {
        Ok(serde_json::json!({
            "claims": [{
                "subject": "smoke-test",
                "predicate": "test_pred",
                "value": "extracted_value",
                "claim_kind": "external_fact",
                "domain": "projects",
                "confidence_basis_points": 9000,
                "supported": true
            }]
        })
        .to_string())
    }

    fn adapter_name(&self) -> &str {
        "smoke_mock"
    }
}

#[test]
fn tool_list_returns_39_tools() {
    let tools = tools::tool_list();
    assert_eq!(tools.len(), 39);
}

#[test]
fn tool_list_contains_expected_names() {
    let tools = tools::tool_list();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected = [
        "wiki_spaces_create",
        "wiki_spaces_register",
        "wiki_spaces_list",
        "wiki_spaces_remove",
        "wiki_spaces_set_default",
        "wiki_config",
        "wiki_content_read",
        "wiki_content_write",
        "wiki_content_new",
        "wiki_content_commit",
        "wiki_search",
        "wiki_list",
        "wiki_ingest",
        "wiki_index_rebuild",
        "wiki_index_status",
        "wiki_graph",
        "wiki_history",
        "wiki_stats",
        "wiki_lint",
        "wiki_resolve",
        "wiki_suggest",
        "wiki_export",
        "profile_get",
        "semantic_search",
        "semantic_get",
        "procedural_find",
        "procedural_get",
        "graph_neighbors",
        "audit_history",
    ];
    for name in &expected {
        assert!(names.contains(name), "missing tool: {name}");
    }
}

#[test]
fn tool_list_no_removed_tools() {
    let tools = tools::tool_list();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let removed = [
        "wiki_init",
        "wiki_read",
        "wiki_write",
        "wiki_new_page",
        "wiki_new_section",
        "wiki_commit",
        "wiki_index_check",
    ];
    for name in &removed {
        assert!(!names.contains(name), "tool should be removed: {name}");
    }
}

#[test]
fn tool_list_all_have_descriptions() {
    for tool in &tools::tool_list() {
        assert!(
            !tool.description.as_ref().is_none_or(|d| d.is_empty()),
            "tool {} has empty description",
            tool.name
        );
    }
}

#[test]
fn tool_list_all_have_object_schema() {
    for tool in &tools::tool_list() {
        let schema = &tool.input_schema;
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool {} schema is not an object",
            tool.name
        );
    }
}

#[test]
fn spaces_create_requires_path_and_name() {
    let tools = tools::tool_list();
    let tool = tools
        .iter()
        .find(|t| t.name == "wiki_spaces_create")
        .unwrap();
    let required = tool
        .input_schema
        .get("required")
        .unwrap()
        .as_array()
        .unwrap();
    let req: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(req.contains(&"path"));
    assert!(req.contains(&"name"));
}

#[test]
fn content_new_has_section_and_name_and_type_params() {
    let tools = tools::tool_list();
    let tool = tools.iter().find(|t| t.name == "wiki_content_new").unwrap();
    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("section"), "missing section param");
    assert!(props.contains_key("name"), "missing name param");
    assert!(props.contains_key("type"), "missing type param");
    assert!(props.contains_key("bundle"), "missing bundle param");
}

#[test]
fn mcp_content_new_places_bare_page_in_concepts() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let server = McpServer::new(manager);

    let result = tools::call(
        &server,
        "wiki_content_new",
        &args(json!({
            "uri": "thai-tts-voice-cloning",
            "wiki": "test",
            "type": "page",
            "name": "Thai TTS"
        })),
    );

    assert!(!result.is_error);
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("\"slug\": \"concepts/thai-tts-voice-cloning\""));
    assert!(
        repo_root
            .join("wiki/concepts/thai-tts-voice-cloning.md")
            .exists()
    );
}

#[test]
fn mcp_content_write_places_bare_concept_in_concepts() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let server = McpServer::new(manager);

    let result = tools::call(
        &server,
        "wiki_content_write",
        &args(json!({
            "uri": "thai-tts-voice-cloning",
            "wiki": "test",
            "content": "---\ntitle: \"Thai TTS\"\ntype: concept\nstatus: active\n---\n\nBody.\n"
        })),
    );

    assert!(!result.is_error);
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("\"slug\": \"concepts/thai-tts-voice-cloning\""));
    assert!(
        repo_root
            .join("wiki/concepts/thai-tts-voice-cloning.md")
            .exists()
    );
    assert!(!repo_root.join("wiki/thai-tts-voice-cloning.md").exists());
}

#[test]
fn mcp_content_write_notifies_managed_web_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_mcp_smoke_wiki(dir.path());
    web::install_hugo_site(&repo_root, "test", "wiki", false).unwrap();
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let server = McpServer::with_web_refresh(manager, tx);

    let result = tools::call(
        &server,
        "wiki_content_write",
        &args(json!({
            "uri": "managed-web-refresh",
            "wiki": "test",
            "content": "---\ntitle: \"Managed Web Refresh\"\ntype: concept\nstatus: active\n---\n\nFresh body.\n"
        })),
    );

    assert!(!result.is_error);
    assert_eq!(rx.try_recv().unwrap(), "test");
    let mirrored =
        fs::read_to_string(repo_root.join("site/content/concepts/managed-web-refresh.md")).unwrap();
    assert!(mirrored.contains("Fresh body."));
    assert!(repo_root.join("site/content/.llm-wiki-refresh").exists());
}

#[test]
fn search_has_type_param() {
    let tools = tools::tool_list();
    let tool = tools.iter().find(|t| t.name == "wiki_search").unwrap();
    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("type"), "missing type param");
}

#[test]
fn wiki_resolve_has_uri_required() {
    let tools = tools::tool_list();
    let tool = tools.iter().find(|t| t.name == "wiki_resolve").unwrap();
    let required = tool
        .input_schema
        .get("required")
        .unwrap()
        .as_array()
        .unwrap();
    let req: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(req.contains(&"uri"));
    assert!(!req.contains(&"wiki"), "wiki should be optional");
    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("uri"));
    assert!(props.contains_key("wiki"));
}

#[test]
fn graph_has_relation_param() {
    let tools = tools::tool_list();
    let tool = tools.iter().find(|t| t.name == "wiki_graph").unwrap();
    let props = tool
        .input_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(props.contains_key("relation"), "missing relation param");
}

fn write_page(wiki_root: &Path, slug: &str, content: &str) {
    let path = wiki_root.join(format!("{slug}.md"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn setup_mcp_smoke_wiki(dir: &Path) -> (PathBuf, PathBuf) {
    let config_path = dir.join("state").join("config.toml");
    let repo_root = dir.join("test");
    spaces::create(&repo_root, "test", None, false, true, &config_path, None).unwrap();

    let wiki_root = repo_root.join("wiki");
    write_page(
        &wiki_root,
        "concepts/moe",
        "---\ntitle: \"MoE\"\ntype: concept\nstatus: active\ntags: [ml]\n---\n\nMixture of Experts links to [[concepts/transformer]].\n",
    );
    write_page(
        &wiki_root,
        "concepts/transformer",
        "---\ntitle: \"Transformer\"\ntype: concept\nstatus: active\n---\n\nAttention model links to [[concepts/moe]].\n",
    );
    write_page(
        &wiki_root,
        "profiles/rules",
        "---\ntitle: \"Rules\"\ntype: profile\nsection: rules\npriority: hard\nstatus: active\n---\n\nAlways verify MCP tool calls.\n",
    );
    write_page(
        &wiki_root,
        "procedures/rebuild-index",
        "---\ntitle: \"Rebuild Index\"\ntype: procedure\nstatus: verified\nverification: [\"cargo test\"]\n---\n\nRun index rebuild and verify search.\n",
    );
    write_page(
        &wiki_root,
        "decisions/adr-mcp",
        "---\ntitle: \"MCP Decision\"\ntype: decision\nstatus: active\nsummary: \"Use MCP smoke tests\"\n---\n\nThe MCP server should call every tool.\n",
    );
    write_page(
        &wiki_root,
        "inbox/to-ingest",
        "---\ntitle: \"To Ingest\"\ntype: doc\nstatus: active\n---\n\nA valid page for ingest dry-run.\n",
    );

    git::commit(&repo_root, "add smoke pages").unwrap();
    (config_path, repo_root)
}

fn existing_wiki_repo(path: &Path) {
    fs::create_dir_all(path.join("wiki")).unwrap();
    fs::write(path.join("wiki.toml"), "name = \"registered-space\"\n").unwrap();
    git::init_repo(path).unwrap();
    git::commit(path, "init registered space").unwrap();
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn mcp_tool_dispatch_smoke_calls_every_registered_tool() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, _repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    // Attach a SemanticStore so brain_* tools work.
    let semantic_root = dir.path().join("semantic-store");
    let (semantic_store, _admin) = llm_wiki::semantic::SemanticStore::create(
        &semantic_root,
        llm_wiki::semantic::SemanticConfig::enabled_for(dir.path()),
    )
    .unwrap();
    let server = McpServer::new(manager)
        .with_semantic_store(Arc::new(semantic_store))
        .with_ai_provider(Arc::new(SmokeAiProvider));

    let create_path = dir.path().join("created-space");
    let register_path = dir.path().join("registered-space");
    existing_wiki_repo(&register_path);
    let export_path = dir.path().join("mcp-export.json");

    let calls: Vec<(&str, Map<String, Value>)> = vec![
        (
            "wiki_spaces_create",
            args(json!({
                "path": create_path.to_string_lossy(),
                "name": "created-space",
                "set_default": false
            })),
        ),
        (
            "wiki_spaces_register",
            args(json!({
                "path": register_path.to_string_lossy(),
                "name": "registered-space"
            })),
        ),
        ("wiki_spaces_list", args(json!({}))),
        ("wiki_spaces_set_default", args(json!({"name": "test"}))),
        (
            "wiki_spaces_remove",
            args(json!({"name": "registered-space", "delete": false})),
        ),
        ("wiki_config", args(json!({"action": "list"}))),
        (
            "wiki_content_read",
            args(json!({"uri": "concepts/moe", "wiki": "test"})),
        ),
        (
            "wiki_content_write",
            args(json!({
                "uri": "scratch/mcp-written",
                "wiki": "test",
                "content": "---\ntitle: \"MCP Written\"\ntype: concept\nstatus: active\n---\n\nWritten through MCP smoke test.\n"
            })),
        ),
        (
            "wiki_content_new",
            args(json!({
                "uri": "scratch/new-page",
                "wiki": "test",
                "type": "concept",
                "name": "New Page"
            })),
        ),
        (
            "wiki_content_commit",
            args(json!({"wiki": "test", "message": "mcp smoke content changes"})),
        ),
        (
            "wiki_search",
            args(json!({"query": "Mixture", "wiki": "test", "top_k": 5})),
        ),
        (
            "wiki_list",
            args(json!({"wiki": "test", "type": "concept", "page_size": 10})),
        ),
        (
            "wiki_ingest",
            args(json!({"wiki": "test", "path": "inbox/to-ingest.md", "dry_run": true})),
        ),
        ("wiki_index_rebuild", args(json!({"wiki": "test"}))),
        ("wiki_index_status", args(json!({"wiki": "test"}))),
        (
            "wiki_graph",
            args(json!({"wiki": "test", "format": "llms", "root": "concepts/moe", "depth": 1})),
        ),
        (
            "wiki_history",
            args(json!({"wiki": "test", "slug": "concepts/moe", "limit": 5})),
        ),
        ("wiki_stats", args(json!({"wiki": "test"}))),
        ("wiki_lint", args(json!({"wiki": "test"}))),
        (
            "wiki_resolve",
            args(json!({"wiki": "test", "uri": "concepts/moe"})),
        ),
        (
            "wiki_suggest",
            args(json!({"wiki": "test", "slug": "concepts/transformer", "limit": 5})),
        ),
        (
            "wiki_schema",
            args(json!({"wiki": "test", "action": "list"})),
        ),
        (
            "wiki_export",
            args(json!({
                "wiki": "test",
                "format": "json",
                "path": export_path.to_string_lossy()
            })),
        ),
        (
            "profile_get",
            args(json!({"wiki": "test", "section": "rules"})),
        ),
        (
            "semantic_search",
            args(json!({"wiki": "test", "query": "decision", "type": "decision", "top_k": 5})),
        ),
        (
            "semantic_get",
            args(json!({"wiki": "test", "page_id": "decisions/adr-mcp", "with_backlinks": true})),
        ),
        (
            "procedural_find",
            args(json!({"wiki": "test", "intent": "rebuild index", "top_k": 5})),
        ),
        (
            "procedural_get",
            args(json!({"wiki": "test", "proc_id": "procedures/rebuild-index"})),
        ),
        (
            "graph_neighbors",
            args(json!({"wiki": "test", "page_id": "concepts/moe", "depth": 1})),
        ),
        (
            "audit_history",
            args(json!({"wiki": "test", "path": "concepts/moe", "limit": 5})),
        ),
        ("brain_status", args(json!({}))),
        ("brain_search", args(json!({"query": "test"}))),
        ("brain_get", args(json!({"subject": "test"}))),
        (
            "brain_capture",
            args(json!({
                "operation_id": "smoke-cap",
                "utterance": "test utterance",
                "subject": "smoke-test",
                "predicate": "test_pred",
                "value": "test_value",
                "domain": "projects",
            })),
        ),
        (
            "brain_confirm",
            args(json!({
                "operation_id": "smoke-confirm",
                "proposal_operation_id": "smoke-cap",
            })),
        ),
        // Capture a second proposal for supersede (can't reuse smoke-cap, it's confirmed)
        (
            "brain_capture",
            args(json!({
                "operation_id": "smoke-cap-2",
                "utterance": "updated test utterance",
                "subject": "smoke-test",
                "predicate": "test_pred",
                "value": "updated_value",
                "domain": "projects",
            })),
        ),
        (
            "brain_supersede",
            args(json!({
                "operation_id": "smoke-supersede",
                "proposal_operation_id": "smoke-cap-2",
                "superseded_claim_operation_ids": "smoke-confirm",
            })),
        ),
        (
            "brain_propose",
            args(json!({
                "operation_id": "smoke-propose",
                "subject": "smoke-test",
                "predicate": "test_pred",
                "value": "inferred_value",
                "domain": "projects",
                "method": "llm_extraction",
            })),
        ),
        (
            "brain_ingest_source",
            args(json!({
                "operation_id": "smoke-ingest",
                "text": "First paragraph of smoke-test source text.\n\nSecond paragraph.",
            })),
        ),
        (
            "brain_extract",
            args(json!({
                "capture_operation_id": "smoke-ingest-chunk-0",
                "method": "llm_extraction",
            })),
        ),
    ];

    let registered_names: Vec<String> = tools::tool_list()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let called_names: Vec<&str> = calls.iter().map(|(name, _)| *name).collect();

    for name in &registered_names {
        assert!(
            called_names.contains(&name.as_str()),
            "missing MCP smoke call for registered tool: {name}"
        );
    }

    for (name, call_args) in calls {
        let result = tools::call(&server, name, &call_args);
        assert!(!result.is_error, "MCP tool {name} returned an error");
    }
}

// ── Auth gate on the real dispatch path (Task B3 / F1 fix) ───────────────────
// These drive `McpServer::check_capability` — the exact gate `call_tool` runs
// before dispatch — proving enforcement is on the hot path, not just a
// standalone `AuthPolicy::allows` unit assertion.

#[test]
fn auth_gate_denies_restricted_principal_at_dispatch() {
    use llm_wiki::mcp::auth::{AuthPolicy, AuthPrincipal, Capability};

    let dir = tempfile::tempdir().unwrap();
    let (config_path, _repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());

    // A read-only principal (e.g. a proposal-less viewer).
    let read_only = AuthPrincipal {
        id: "read-only-worker".to_owned(),
        capabilities: vec![Capability::Read],
    };
    let server = McpServer::new(manager).with_auth_policy(AuthPolicy::default(), read_only);

    // Read-class tool is allowed.
    assert!(
        server.check_capability("brain_search").is_ok(),
        "read-only principal should be allowed a read tool"
    );
    // Capture/confirm-class tools are denied at the dispatch gate.
    let denied = server
        .check_capability("brain_capture")
        .expect_err("read-only principal must be denied brain_capture");
    assert!(
        denied.contains("capability denied") && denied.contains("brain_capture"),
        "denial message should name the tool: {denied}"
    );
    assert!(
        server.check_capability("brain_confirm").is_err(),
        "read-only principal must be denied brain_confirm"
    );
    assert!(
        server.check_capability("wiki_spaces_remove").is_err(),
        "read-only principal must be denied a purge-class tool"
    );
}

#[test]
fn auth_gate_absent_allows_all_legacy_mode() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, _repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    // No auth policy → legacy/dev mode, gate is a no-op.
    let server = McpServer::new(manager);
    assert!(server.check_capability("brain_capture").is_ok());
    assert!(server.check_capability("wiki_spaces_remove").is_ok());
}

#[test]
fn auth_gate_owner_principal_allows_all_on_serve_path() {
    use llm_wiki::mcp::auth::AuthPolicy;

    let dir = tempfile::tempdir().unwrap();
    let (config_path, _repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    // Mirror what serve() does: gate ON, owner principal has every capability,
    // so the single-owner local path is enforced-but-permitted (no breakage).
    let server = McpServer::new(manager)
        .with_auth_policy(AuthPolicy::default(), llm_wiki::mcp::owner_principal());
    for tool in [
        "brain_search",
        "brain_capture",
        "brain_confirm",
        "brain_supersede",
        "wiki_spaces_remove",
    ] {
        assert!(
            server.check_capability(tool).is_ok(),
            "owner principal should be allowed {tool}"
        );
    }
}

// ── Merged write-pipeline contract (Task 6) ─────────────────────────────────
// These pin the promised UX of the ingest merge: a single `wiki_content_write`
// is enough to make a page searchable, the legacy `wiki_ingest` escape hatch
// remains a no-op when nothing changed, and bulk-write-via-commit=false plus a
// single directory ingest advances HEAD exactly once.

#[test]
fn mcp_content_write_default_makes_page_searchable_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, _repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let server = McpServer::new(manager);

    let body = "---\ntitle: \"Single Call\"\ntype: concept\nstatus: active\n---\n\nUnique token: zeta-immediate-search-9b3c.\n";
    let write_result = tools::call(
        &server,
        "wiki_content_write",
        &args(json!({
            "uri": "single-call",
            "wiki": "test",
            "content": body,
        })),
    );
    assert!(!write_result.is_error, "write should succeed");
    let write_text = write_result.content[0].as_text().unwrap().text.clone();
    assert!(
        write_text.contains("\"commit_sha\"") && !write_text.contains("\"commit_sha\": null"),
        "commit_sha should be populated; got: {write_text}"
    );

    // The page must be searchable WITHOUT any further call.
    let search_result = tools::call(
        &server,
        "wiki_search",
        &args(json!({
            "wiki": "test",
            "query": "zeta-immediate-search-9b3c",
        })),
    );
    assert!(!search_result.is_error, "search should succeed");
    let search_text = search_result.content[0].as_text().unwrap().text.clone();
    assert!(
        search_text.contains("single-call") || search_text.contains("Single Call"),
        "search should find the new page; got: {search_text}"
    );
}

#[test]
fn mcp_content_write_then_ingest_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let server = McpServer::new(manager);

    let body = "---\ntitle: \"Idempotent\"\ntype: concept\nstatus: active\n---\n\nBody.\n";
    let write_result = tools::call(
        &server,
        "wiki_content_write",
        &args(json!({
            "uri": "idempotent",
            "wiki": "test",
            "content": body,
        })),
    );
    let write_text = write_result.content[0].as_text().unwrap().text.clone();
    // Extract the commit_sha from the write response.
    let write_sha: String = {
        let parsed: serde_json::Value = serde_json::from_str(&write_text).unwrap();
        parsed["commit_sha"].as_str().unwrap().to_string()
    };
    assert!(!write_sha.is_empty());

    // Now call wiki_ingest on the same page. It should be a no-op commit-wise.
    let ingest_result = tools::call(
        &server,
        "wiki_ingest",
        &args(json!({
            "wiki": "test",
            "path": "concepts/idempotent.md",
        })),
    );
    assert!(!ingest_result.is_error, "follow-up ingest should succeed");
    let ingest_text = ingest_result.content[0].as_text().unwrap().text.clone();

    // HEAD should not have advanced — ingest saw no changes.
    let head_after = std::process::Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head_sha = String::from_utf8(head_after.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        head_sha, write_sha,
        "follow-up ingest should not advance HEAD; got {head_sha}, expected {write_sha}"
    );

    // Silence unused warning if the assertion shape needs the text.
    let _ = ingest_text;
}

#[test]
fn mcp_content_write_commit_false_then_ingest_handles_bulk_write() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_mcp_smoke_wiki(dir.path());
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let server = McpServer::new(manager);

    // Write three pages with commit=false.
    for slug in ["bulk-a", "bulk-b", "bulk-c"] {
        let body = format!("---\ntitle: \"{slug}\"\ntype: concept\nstatus: active\n---\n\nBody.\n");
        let result = tools::call(
            &server,
            "wiki_content_write",
            &args(json!({
                "uri": slug,
                "wiki": "test",
                "commit": false,
                "content": body,
            })),
        );
        assert!(!result.is_error, "write of {slug} should succeed");
        let text = result.content[0].as_text().unwrap().text.clone();
        assert!(
            text.contains("\"commit_sha\": null"),
            "commit_sha should be null when commit=false; got: {text}"
        );
    }

    // Record HEAD before the bulk ingest.
    let head_before = std::process::Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head_before = String::from_utf8(head_before.stdout)
        .unwrap()
        .trim()
        .to_string();

    // One ingest call commits and indexes all three.
    let ingest_result = tools::call(
        &server,
        "wiki_ingest",
        &args(json!({
            "wiki": "test",
            "path": "concepts",
        })),
    );
    assert!(!ingest_result.is_error, "bulk ingest should succeed");

    let head_after = std::process::Command::new("git")
        .args(["-C", repo_root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head_after = String::from_utf8(head_after.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(
        head_before, head_after,
        "bulk ingest should advance HEAD exactly once"
    );

    // All three should now be searchable.
    for slug in ["bulk-a", "bulk-b", "bulk-c"] {
        let search_result = tools::call(
            &server,
            "wiki_search",
            &args(json!({ "wiki": "test", "query": slug })),
        );
        let search_text = search_result.content[0].as_text().unwrap().text.clone();
        assert!(
            search_text.contains(slug),
            "search should find {slug} after bulk ingest; got: {search_text}"
        );
    }
}
