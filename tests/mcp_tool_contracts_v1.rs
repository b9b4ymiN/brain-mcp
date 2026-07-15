//! Task 3.1 — Stable MCP tool contracts (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 3.1 DoD + §7.2 annotations requirement:
//! - tool schemas/output/annotations/errors/pagination ผ่าน schema tests
//! - read tools bounded และคืน structured + text fallback
//! - mutations มี operation_id, authority และ audit link
//!
//! This is a contract-layer test: it inspects the tool declarations and the
//! result shaping, NOT the wiki engine. It proves the contracts are stable
//! and self-consistent.

use llm_wiki::mcp::helpers::{ToolResult, WikiError, err_code};
use llm_wiki::mcp::tools::tool_list;

// =============================================================================
// DoD: annotations — every tool carries the right hints (§7.2)
// =============================================================================

/// Every declared tool has a non-None `annotations`. The MCP spec treats
/// annotations as hints, but GOAL-vNext §7.2 makes them mandatory for this
/// server so clients can reason about read-only vs destructive calls without
/// probing.
#[test]
fn every_tool_has_annotations() {
    let tools = tool_list();
    assert!(!tools.is_empty(), "tool list must not be empty");
    for tool in &tools {
        assert!(
            tool.annotations.is_some(),
            "tool {} is missing annotations",
            tool.name
        );
    }
}

/// Read-only tools declare `read_only_hint == true`. A tool that only reads
/// (search, list, read, history, schema, stats) must not hint destructive.
#[test]
fn read_only_tools_declare_read_only_hint() {
    let tools = tool_list();
    let read_only_names = [
        "wiki_search",
        "wiki_list",
        "wiki_content_read",
        "wiki_history",
        "wiki_schema",
        "wiki_stats",
        "wiki_graph",
        "wiki_resolve",
        "profile_get",
        "semantic_search",
        "semantic_get",
        "procedural_find",
        "procedural_get",
        "graph_neighbors",
        "audit_history",
    ];
    for name in read_only_names {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tool {name} should exist"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("tool {name} missing annotations"));
        assert!(
            ann.read_only_hint == Some(true),
            "tool {name} must declare read_only_hint=true, got {:?}",
            ann.read_only_hint
        );
    }
}

/// Destructive mutation tools declare `destructive_hint == true` and
/// `read_only_hint == false`. A tool that deletes or rewrites (content_write
/// with replace, index_rebuild, spaces_remove) must be marked.
#[test]
fn destructive_tools_declare_destructive_hint() {
    let tools = tool_list();
    let destructive_names = ["wiki_spaces_remove"];
    for name in destructive_names {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tool {name} should exist"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("tool {name} missing annotations"));
        assert!(
            ann.destructive_hint == Some(true),
            "tool {name} must declare destructive_hint=true"
        );
        assert!(
            ann.read_only_hint != Some(true),
            "destructive tool {name} must not be read_only"
        );
    }
}

/// Idempotent tools declare `idempotent_hint == true`. Tools that are safe to
/// retry with the same args (index_rebuild, index_status) should be marked.
#[test]
fn idempotent_tools_declare_idempotent_hint() {
    let tools = tool_list();
    let idempotent_names = ["wiki_index_rebuild", "wiki_index_status"];
    for name in idempotent_names {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tool {name} should exist"));
        let ann = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("tool {name} missing annotations"));
        assert!(
            ann.idempotent_hint == Some(true),
            "tool {name} must declare idempotent_hint=true"
        );
    }
}

// =============================================================================
// DoD: input schemas are valid JSON Schema objects
// =============================================================================

/// Every tool's input schema is a valid JSON Schema of type object with a
/// `properties` map. Catches malformed hand-written schemas.
#[test]
fn every_tool_input_schema_is_a_valid_object_schema() {
    let tools = tool_list();
    for tool in &tools {
        let schema = &tool.input_schema;
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool {} input schema must be type=object",
            tool.name
        );
        assert!(
            schema
                .get("properties")
                .and_then(|v| v.as_object())
                .is_some(),
            "tool {} input schema must have a properties object",
            tool.name
        );
    }
}

/// Required parameters must also appear in properties. Catches a common
/// schema-writing bug where a required name is misspelled.
#[test]
fn required_parameters_are_declared_in_properties() {
    let tools = tool_list();
    for tool in &tools {
        let schema = &tool.input_schema;
        let required: Vec<String> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("tool {} has no properties", tool.name));
        for req in &required {
            assert!(
                properties.contains_key(req),
                "tool {} requires '{}' but it is not in properties",
                tool.name,
                req
            );
        }
    }
}

// =============================================================================
// DoD: read tools return structured content + text fallback (§7.2)
// =============================================================================

/// `ToolResult` carries an optional `structured_content` so the rmcp
/// `CallToolResult` can populate both `content` (text) and
/// `structured_content` (JSON). Read tools that return JSON should populate
/// both; the text block is the fallback for clients that ignore structured
/// content.
#[test]
fn tool_result_has_structured_content_field() {
    // Compile-time proof the field exists and is reachable.
    let result = ToolResult {
        content: vec![],
        is_error: false,
        notify_uris: vec![],
        notify_resources_changed: false,
        structured_content: Some(serde_json::json!({"ok": true})),
    };
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"ok": true}))
    );
}

/// `ok_structured` builds a ToolResult with BOTH a text fallback (the JSON
/// pretty-printed) and the structured content set — so a read tool returns a
/// single coherent result that serves both kinds of client.
#[test]
fn ok_structured_populates_text_and_structured() {
    let payload = serde_json::json!({"pages": [], "count": 0});
    let result = llm_wiki::mcp::helpers::ok_structured(payload.clone());
    assert!(!result.content.is_empty(), "text fallback must be present");
    assert_eq!(
        result.structured_content,
        Some(payload),
        "structured content must match the payload"
    );
    assert!(!result.is_error);
}

// =============================================================================
// DoD: actionable structured errors (§7.2)
// =============================================================================

/// Every `WikiError` variant maps to a stable, machine-readable code string.
/// Clients parse the `[CODE]` prefix to decide retry/fallback behavior.
#[test]
fn wiki_error_codes_are_stable_strings() {
    let cases = [
        (WikiError::WikiNotFound, "WIKI_NOT_FOUND"),
        (WikiError::IndexNotOpen, "INDEX_NOT_OPEN"),
        (WikiError::InvalidUri, "INVALID_URI"),
        (WikiError::LockFailed, "LOCK_FAILED"),
        (WikiError::InternalError, "INTERNAL_ERROR"),
    ];
    for (err, expected_code) in cases {
        assert_eq!(err.code(), expected_code);
    }
}

/// `err_code` formats a structured error with the code prefix so the tool
/// result text is actionable (the client can extract the code and detail).
#[test]
fn err_code_prefixes_with_machine_readable_code() {
    let msg = err_code(WikiError::WikiNotFound, "no wiki named ghost");
    assert!(
        msg.starts_with("[WIKI_NOT_FOUND]"),
        "err_code must prefix the code: got {msg}"
    );
    assert!(msg.contains("no wiki named ghost"));
}

/// `err_structured` builds a ToolResult whose structured content carries the
/// machine-readable code + detail, and whose text block is actionable. This
/// is the error counterpart to `ok_structured`.
#[test]
fn err_structured_carries_code_and_detail() {
    let result = llm_wiki::mcp::helpers::err_structured(WikiError::InvalidUri, "bad uri");
    assert!(result.is_error);
    let structured = result
        .structured_content
        .as_ref()
        .expect("error must carry structured content");
    assert_eq!(structured["code"], "INVALID_URI");
    assert_eq!(structured["message"], "bad uri");
    // Text fallback is also present and actionable.
    assert!(!result.content.is_empty());
}
