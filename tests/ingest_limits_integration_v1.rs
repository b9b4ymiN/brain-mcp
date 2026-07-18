//! Task F2.3 — Ingest limits enforced (413/429) end-to-end integration.
//!
//! Drives the real MCP ingest handlers (`brain_ingest_source`,
//! `brain_capture`) through `tools::call` with a real `McpServer` that has
//! ingest limits attached, asserting:
//!
//! 1. `size_limit_rejects_oversized` — an 11 MB source when the cap is 10 MB
//!    is rejected; the error string carries the `PAYLOAD_TOO_LARGE` code
//!    prefix and the `ingest_rejected_total{reason="size"}` counter
//!    increments.
//! 2. `rate_limit_rejects_burst` — the 61st call when the cap is 60/min is
//!    rejected; the error carries `RATE_LIMITED` and
//!    `ingest_rejected_total{reason="rate"}` increments.
//! 3. `size_limit_allows_under` — a 9 MB source when the cap is 10 MB is
//!    accepted.
//! 4. `rate_limit_allows_under` — 60 calls when the cap is 60 all succeed,
//!    and the 61st is rejected (proves the window boundary: `<` not `<=`).
//!
//! The unit tests on `IngestRateLimiter` (in `src/observability.rs`) cover
//! the limiter's own semantics; this file covers the WIRE — that the
//! handlers route through the gate, surface the right error code, and bump
//! the right metric label.

use std::path::Path;
use std::sync::Arc;

use llm_wiki::engine::WikiEngine;
use llm_wiki::mcp::{McpServer, tools};
use llm_wiki::observability::IngestRateLimiter;
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

/// Build an `McpServer` with a semantic store AND ingest limits attached at
/// the specified cap values. Mirrors the `serve()` production wiring.
fn server_with_limits(
    dir: &Path,
    max_source_bytes: usize,
    max_sources_per_minute: u32,
) -> McpServer {
    let (manager, store) = setup(dir);
    McpServer::new(manager)
        .with_semantic_store(store)
        .with_ingest_limits(
            max_source_bytes,
            IngestRateLimiter::new(max_sources_per_minute),
        )
}

/// Read the current Prometheus exposition and extract the SUM of a labeled
/// counter (mirrors `metrics_integration_v1::counter_value`). The global
/// recorder is shared across the parallel test process — that's why each
/// test snapshots the value BEFORE + AFTER its action rather than asserting
/// an absolute value.
fn counter_total(body: &str, name: &str) -> f64 {
    let mut total = 0.0f64;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(head) = parts.next() else {
            continue;
        };
        let (sample_name, _labels) = match head.split_once('{') {
            Some((n, _)) => (n, Some(head)),
            None => (head, None),
        };
        if sample_name != name {
            continue;
        }
        if let Some(v) = parts.next()
            && let Ok(parsed) = v.parse::<f64>()
        {
            total += parsed;
        }
    }
    total
}

// =============================================================================
// DoD: size limit rejects oversized (413 / PAYLOAD_TOO_LARGE)
// =============================================================================

#[test]
fn size_limit_rejects_oversized_brain_ingest_source() {
    let dir = tempfile::tempdir().unwrap();
    // Cap at 10 MB; feed 11 MB.
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 60);

    let oversized = "a".repeat(11 * 1024 * 1024);
    let result = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "big-src",
            "text": oversized,
        })),
    );

    assert!(
        result.is_error,
        "11MB source over 10MB cap must be rejected"
    );
    let msg = result
        .content
        .first()
        .map(|c| c.as_text().map(|t| t.text.clone()).unwrap_or_default())
        .unwrap_or_default();
    assert!(
        msg.contains("PAYLOAD_TOO_LARGE"),
        "error must carry the structured code prefix, got: {msg}"
    );
    assert!(
        msg.contains("max bytes"),
        "error must be actionable (mention max bytes), got: {msg}"
    );
}

#[test]
fn size_limit_rejects_oversized_brain_capture() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 60);

    let oversized = "a".repeat(11 * 1024 * 1024);
    let result = tools::call(
        &server,
        "brain_capture",
        &args(json!({
            "operation_id": "big-utterance",
            "utterance": oversized,
            "subject": "s",
            "predicate": "p",
            "value": "v",
            "domain": "d",
        })),
    );

    assert!(result.is_error, "oversized utterance must be rejected");
    let msg = result
        .content
        .first()
        .map(|c| c.as_text().map(|t| t.text.clone()).unwrap_or_default())
        .unwrap_or_default();
    assert!(
        msg.contains("PAYLOAD_TOO_LARGE"),
        "error must carry the structured code prefix, got: {msg}"
    );
}

// =============================================================================
// DoD: rate limit rejects burst (429 / RATE_LIMITED)
// =============================================================================

#[test]
fn rate_limit_rejects_burst_brain_ingest_source() {
    let dir = tempfile::tempdir().unwrap();
    // Cap at 5 sources/min so the test stays fast.
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 5);

    // First 5 calls must succeed.
    for i in 0..5 {
        let result = tools::call(
            &server,
            "brain_ingest_source",
            &args(json!({
                "operation_id": format!("src-{i}"),
                "text": "ok",
            })),
        );
        assert!(
            !result.is_error,
            "call #{i} must succeed under the rate cap, got: {:?}",
            result.content
        );
    }

    // 6th must be rejected with RATE_LIMITED.
    let result = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "src-over",
            "text": "ok",
        })),
    );
    assert!(result.is_error, "6th call must hit the rate limit");
    let msg = result
        .content
        .first()
        .map(|c| c.as_text().map(|t| t.text.clone()).unwrap_or_default())
        .unwrap_or_default();
    assert!(
        msg.contains("RATE_LIMITED"),
        "rate-limit error must carry the structured code prefix, got: {msg}"
    );
    assert!(
        msg.contains("5/min"),
        "error must report the configured per-minute cap, got: {msg}"
    );
}

#[test]
fn rate_limit_rejects_burst_brain_capture() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 3);

    for i in 0..3 {
        let result = tools::call(
            &server,
            "brain_capture",
            &args(json!({
                "operation_id": format!("cap-{i}"),
                "utterance": "ok",
                "subject": "s",
                "predicate": "p",
                "value": "v",
                "domain": "d",
            })),
        );
        assert!(!result.is_error, "capture #{i} must succeed");
    }
    let result = tools::call(
        &server,
        "brain_capture",
        &args(json!({
            "operation_id": "cap-over",
            "utterance": "ok",
            "subject": "s",
            "predicate": "p",
            "value": "v",
            "domain": "d",
        })),
    );
    assert!(result.is_error);
    let msg = result
        .content
        .first()
        .map(|c| c.as_text().map(|t| t.text.clone()).unwrap_or_default())
        .unwrap_or_default();
    assert!(msg.contains("RATE_LIMITED"), "got: {msg}");
}

// =============================================================================
// DoD: under-limit accepted (9MB over 10MB cap, 60 over 60/min boundary)
// =============================================================================

#[test]
fn size_limit_allows_under() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 60);

    let under = "a".repeat(9 * 1024 * 1024);
    let result = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "nine-mb",
            "text": under,
        })),
    );
    assert!(
        !result.is_error,
        "9MB under 10MB cap must be accepted, got: {:?}",
        result.content
    );
}

#[test]
fn rate_limit_allows_under_boundary() {
    let dir = tempfile::tempdir().unwrap();
    // Boundary check: cap = 3 means 3 calls succeed, the 4th fails. Proves
    // the comparison is `<` (i.e. the first N=max calls are allowed).
    let server = server_with_limits(dir.path(), 10 * 1024 * 1024, 3);

    for i in 0..3 {
        let result = tools::call(
            &server,
            "brain_ingest_source",
            &args(json!({
                "operation_id": format!("under-{i}"),
                "text": "x",
            })),
        );
        assert!(!result.is_error, "call #{i} of 3 must succeed");
    }
    // The next call must fail — proves the boundary is correct.
    let result = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "under-over",
            "text": "x",
        })),
    );
    assert!(result.is_error, "call after the cap must be rejected");
}

// =============================================================================
// DoD: ingest_rejected_total counter increments on rejection
// =============================================================================

#[test]
fn ingest_rejected_total_increments_on_size_rejection() {
    // Snapshot the counter BEFORE; reject once; snapshot AFTER.
    // Ensure the recorder is installed (no-op if already up — the metrics
    // integration tests share the process-wide recorder).
    let _ = llm_wiki::observability::init_recorder();

    let dir = tempfile::tempdir().unwrap();
    let server = server_with_limits(dir.path(), 1024, 60);

    let before = counter_total(
        &llm_wiki::observability::render_prometheus(),
        "ingest_rejected_total",
    );

    // Source of 2 KB over 1 KB cap → size rejection.
    let oversized = "a".repeat(2 * 1024);
    let result = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "rejected-size",
            "text": oversized,
        })),
    );
    assert!(result.is_error);

    let after = counter_total(
        &llm_wiki::observability::render_prometheus(),
        "ingest_rejected_total",
    );
    assert!(
        after > before,
        "ingest_rejected_total must increase on size rejection: before={before}, after={after}"
    );
    // The label `reason="size"` must appear on at least one sample line.
    let body = llm_wiki::observability::render_prometheus();
    assert!(
        body.contains("ingest_rejected_total") && body.contains("reason=\"size\""),
        "expected ingest_rejected_total{{reason=\"size\"}} in exposition:\n{body}"
    );
}

#[test]
fn ingest_rejected_total_increments_on_rate_rejection() {
    let _ = llm_wiki::observability::init_recorder();

    let dir = tempfile::tempdir().unwrap();
    let server = server_with_limits(dir.path(), 1024, 2);

    let before = counter_total(
        &llm_wiki::observability::render_prometheus(),
        "ingest_rejected_total",
    );

    // Exhaust the 2-call budget, then trigger a rate rejection.
    for i in 0..2 {
        let r = tools::call(
            &server,
            "brain_ingest_source",
            &args(json!({
                "operation_id": format!("rate-ok-{i}"),
                "text": "x",
            })),
        );
        assert!(!r.is_error);
    }
    let r = tools::call(
        &server,
        "brain_ingest_source",
        &args(json!({
            "operation_id": "rate-over",
            "text": "x",
        })),
    );
    assert!(r.is_error);

    let after = counter_total(
        &llm_wiki::observability::render_prometheus(),
        "ingest_rejected_total",
    );
    assert!(
        after > before,
        "ingest_rejected_total must increase on rate rejection: before={before}, after={after}"
    );
    let body = llm_wiki::observability::render_prometheus();
    assert!(
        body.contains("reason=\"rate\""),
        "expected ingest_rejected_total{{reason=\"rate\"}} in exposition:\n{body}"
    );
}

// =============================================================================
// DoD: no limits attached → legacy path works (back-compat)
// =============================================================================

#[test]
fn no_limits_means_no_enforcement() {
    // A server built via `McpServer::new` (NOT `serve()`) must NOT enforce
    // ingest limits — the gate is a no-op until `with_ingest_limits` attaches
    // one. This is what keeps every pre-F2.3 test working unchanged.
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager).with_semantic_store(store);

    // An arbitrary number of large calls must all succeed.
    for i in 0..10 {
        let result = tools::call(
            &server,
            "brain_ingest_source",
            &args(json!({
                "operation_id": format!("unlimited-{i}"),
                "text": "a".repeat(1024 * 1024),
            })),
        );
        assert!(
            !result.is_error,
            "no-limits server must accept the call #{i}, got: {:?}",
            result.content
        );
    }
}
