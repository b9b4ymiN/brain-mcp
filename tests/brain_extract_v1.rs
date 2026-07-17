//! Phase D Task D3.3 — brain_extract (real extraction pipeline wiring).
//!
//! `mcp_tool_dispatch_smoke_calls_every_registered_tool` in `tests/mcp.rs`
//! already proves the full happy path end to end (ingest -> chunk -> capture
//! -> read_capture -> prompt -> AI call -> repair_json -> parse_candidates
//! -> validate_with_rendition -> propose_inference). This file covers the
//! handler's own edge cases with a scripted `AiProvider` (no network) — the
//! full adversarial corpus (prompt-injection resistance against a REAL
//! model, SSRF, secret handling) is Task D3.4/D3.5.

use std::path::Path;
use std::sync::Arc;

use llm_wiki::engine::WikiEngine;
use llm_wiki::mcp::{McpServer, tools};
use llm_wiki::provider::{AiProvider, ProviderError, ProviderRequest, ProviderResult};
use llm_wiki::semantic::{SemanticConfig, SemanticStore};
use llm_wiki::spaces;
use serde_json::{Map, Value, json};

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn setup(dir: &Path) -> (Arc<WikiEngine>, Arc<SemanticStore>) {
    let config_path = dir.join("state").join("config.toml");
    let repo_root = dir.join("test");
    spaces::create(&repo_root, "test", None, false, true, &config_path, None).unwrap();
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());
    let semantic_root = dir.join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&semantic_root, SemanticConfig::enabled_for(dir)).unwrap();
    (manager, Arc::new(store))
}

struct ScriptedProvider {
    response: ProviderResult<String>,
}

impl AiProvider for ScriptedProvider {
    fn complete(&self, _request: &ProviderRequest) -> ProviderResult<String> {
        self.response.clone()
    }
    fn adapter_name(&self) -> &str {
        "scripted-test"
    }
}

fn server_with_ingested_chunk(dir: &Path, response: ProviderResult<String>) -> McpServer {
    let (manager, store) = setup(dir);
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_ai_provider(Arc::new(ScriptedProvider { response }));

    let ingest = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "src-1",
            "text": "GULF target price raised to 58 baht by analyst coverage.",
        })),
    );
    assert!(
        !ingest.is_error,
        "ingest must succeed to set up the fixture"
    );
    server
}

#[test]
fn supported_candidate_is_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let response = Ok(json!({
        "claims": [{
            "subject": "gulf",
            "predicate": "target_price",
            "value": 58,
            "claim_kind": "external_fact",
            "domain": "stocks",
            "confidence_basis_points": 9000,
            "supported": true
        }]
    })
    .to_string());
    let server = server_with_ingested_chunk(dir.path(), response);

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(!result.is_error, "extract should succeed: {text}");
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["proposed_count"], 1);
    assert_eq!(payload["filtered_count"], 0);
    assert_eq!(payload["proposed"][0]["status"], "proposed");
}

#[test]
fn unsupported_candidate_is_filtered_not_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let response = Ok(json!({
        "claims": [{
            "subject": "gulf",
            "predicate": "target_price",
            "value": 58,
            "claim_kind": "external_fact",
            "domain": "stocks",
            "confidence_basis_points": 3000,
            "supported": false
        }]
    })
    .to_string());
    let server = server_with_ingested_chunk(dir.path(), response);

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(
        !result.is_error,
        "extract call itself should succeed: {text}"
    );
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        payload["proposed_count"], 0,
        "unsupported candidate must not be proposed"
    );
    assert_eq!(payload["filtered_count"], 1);
}

#[test]
fn no_claims_extracted_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let response = Ok(json!({"claims": []}).to_string());
    let server = server_with_ingested_chunk(dir.path(), response);

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    assert!(!result.is_error);
    let text = result.content[0].as_text().unwrap().text.clone();
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["proposed_count"], 0);
}

#[test]
fn malformed_ai_response_is_discarded_not_guessed_at() {
    let dir = tempfile::tempdir().unwrap();
    // Genuinely broken — not recoverable by bounded repair (no markdown
    // fence to strip, no valid JSON substring at all).
    let response = Ok("The GULF stock looks promising based on the source.".to_owned());
    let server = server_with_ingested_chunk(dir.path(), response);

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    assert!(
        result.is_error,
        "a non-JSON response must be discarded, not guessed at"
    );
}

#[test]
fn provider_error_propagates_as_a_tool_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_ingested_chunk(dir.path(), Err(ProviderError::RateLimited));

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    assert!(result.is_error);
}

#[test]
fn missing_ai_provider_is_a_clear_error_not_a_silent_noop() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    // No .with_ai_provider(...) at all.
    let server = McpServer::new(manager).with_semantic_store(store);

    let ingest = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({"operation_id": "src-1", "text": "some source text"})),
    );
    assert!(!ingest.is_error);

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "src-1-chunk-0", "method": "llm_extraction"})),
    );
    assert!(
        result.is_error,
        "brain_extract without a configured AI provider must error, never fabricate a result"
    );
}

#[test]
fn unknown_capture_operation_id_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_ai_provider(Arc::new(ScriptedProvider {
            response: Ok(json!({"claims": []}).to_string()),
        }));

    let result = tools::call(
        &server,
        "brain_extract",
        &args(json!({"capture_operation_id": "never-ingested", "method": "llm_extraction"})),
    );
    assert!(result.is_error);
}
