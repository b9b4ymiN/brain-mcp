//! Phase D Task D3.5 — evidence-span exactness on annotated multi-chunk
//! fixtures. `adversarial_028` (extraction.rs unit tests) already proves the
//! mechanical hash-match property in isolation; this file proves it holds
//! ACROSS a realistic multi-chunk document end to end through the MCP
//! tools — specifically that chunk N's proposal is validated against chunk
//! N's own bytes, never a different chunk's.

use std::path::Path;
use std::sync::Arc;

use llm_wiki::engine::WikiEngine;
use llm_wiki::mcp::{McpServer, tools};
use llm_wiki::provider::{AiProvider, ProviderRequest, ProviderResult};
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

/// Returns a DIFFERENT claim per chunk, keyed by a marker word present only
/// in that chunk's own text — this is how the test proves each proposal's
/// evidence was validated against ITS OWN chunk, not another one: the mock
/// "reads" (in the sense of matching against) the actual prompt text passed
/// to it, exactly like an annotated fixture pins expected output per input.
struct AnnotatedProvider;

impl AiProvider for AnnotatedProvider {
    fn complete(&self, request: &ProviderRequest) -> ProviderResult<String> {
        let subject = if request.prompt.contains("ALPHA-MARKER") {
            "alpha-claim"
        } else if request.prompt.contains("BETA-MARKER") {
            "beta-claim"
        } else if request.prompt.contains("GAMMA-MARKER") {
            "gamma-claim"
        } else {
            "unknown-claim"
        };
        Ok(json!({
            "claims": [{
                "subject": subject, "predicate": "p", "value": "v",
                "claim_kind": "external_fact", "domain": "d",
                "confidence_basis_points": 9000, "supported": true
            }]
        })
        .to_string())
    }
    fn adapter_name(&self) -> &str {
        "annotated-fixture"
    }
}

#[test]
fn each_chunks_proposal_is_validated_against_its_own_bytes_not_a_sibling_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_ai_provider(Arc::new(AnnotatedProvider));

    // Three distinct paragraphs -> three distinct chunks (chunk_text splits
    // on blank-line boundaries; each of these easily fits in one chunk each
    // given the default max_chunk_bytes, but we pass a tiny limit to FORCE
    // three separate chunks regardless).
    let source = "First paragraph carries the ALPHA-MARKER only.\n\n\
                  Second paragraph carries the BETA-MARKER only.\n\n\
                  Third paragraph carries the GAMMA-MARKER only.";
    let ingest = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({"operation_id": "doc", "text": source, "max_chunk_bytes": 10})),
    );
    assert!(!ingest.is_error);
    let ingest_payload: Value =
        serde_json::from_str(&ingest.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(
        ingest_payload["chunk_count"], 3,
        "expected 3 distinct chunks"
    );

    // Extract from each chunk and confirm the subject reported matches THAT
    // chunk's own marker — proving the rendition bytes read back for
    // validation were the right chunk's, not a sibling's.
    let expected = [
        ("doc-chunk-0", "alpha-claim"),
        ("doc-chunk-1", "beta-claim"),
        ("doc-chunk-2", "gamma-claim"),
    ];
    for (capture_operation_id, expected_subject) in expected {
        let result = tools::call(
            &server,
            "brain_extract",
            &args(json!({
                "capture_operation_id": capture_operation_id,
                "method": "llm_extraction",
            })),
        );
        let text = result.content[0].as_text().unwrap().text.clone();
        assert!(!result.is_error, "{capture_operation_id}: {text}");
        let payload: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(payload["proposed_count"], 1, "{capture_operation_id}");
        // The proposed claim's subject must match THIS chunk's own marker —
        // not cross-contaminated with a sibling chunk's evidence/content.
        assert_eq!(
            payload["proposed"][0]["subject"], expected_subject,
            "{capture_operation_id} produced the wrong subject: {text}"
        );
    }
}

#[test]
fn evidence_span_covers_exactly_the_captured_chunk_byte_length() {
    // Direct proof at the extraction-policy level, using the REAL bytes a
    // chunk would carry (not a synthetic placeholder): the whole-rendition
    // span's byte_end equals the exact chunk length, and matches_rendition
    // holds for those exact bytes but fails for a 1-byte-truncated slice —
    // proving the span is pinned to the precise boundary, not "close enough".
    let chunk = b"GULF target price raised to 58 baht by analyst coverage.";
    let span = llm_wiki::extraction::EvidenceSpan::whole_rendition("chunk-x", chunk);
    assert_eq!(span.byte_start, 0);
    assert_eq!(span.byte_end, chunk.len() as u64);
    assert!(span.matches_rendition(chunk));
    assert!(
        !span.matches_rendition(&chunk[..chunk.len() - 1]),
        "a span computed for the full chunk must NOT validate against a truncated version of it"
    );
    assert!(
        !span.matches_rendition(b"GULF target price raised to 58 baht by analyst coverage!"), // trailing '!' not '.'
        "a single differing trailing byte must break the hash match"
    );
}
