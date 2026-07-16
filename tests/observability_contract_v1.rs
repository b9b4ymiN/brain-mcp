//! Task 6.2 — Observability + operations contract (RED stage).
//!
//! GOAL-vNext §13 Task 6.2: structured logs, metrics, log redaction,
//! rate/size/time limits.

use llm_wiki::observability::{IngestLimits, LogRedactor, MetricKind, MetricPoint};

// =============================================================================
// DoD: structured logs + metrics
// =============================================================================

#[test]
fn metric_point_carries_kind_name_value() {
    let p = MetricPoint {
        kind: MetricKind::Counter,
        name: "brain_mcp_tool_calls_total".to_owned(),
        value: 42.0,
    };
    assert_eq!(p.name, "brain_mcp_tool_calls_total");
    assert_eq!(p.value, 42.0);
}

// =============================================================================
// DoD: log redaction — no token/source secrets leak
// =============================================================================

#[test]
fn log_redactor_strips_bearer_tokens() {
    let redacted = LogRedactor::redact("Authorization: Bearer sk-secret-1234567890abcdef");
    assert!(!redacted.contains("sk-secret"));
    assert!(redacted.contains("[REDACTED]"));
}

#[test]
fn log_redactor_strips_api_key_query_params() {
    let redacted = LogRedactor::redact("GET /mcp?api_key=sk-leaked1234567890abcdef");
    assert!(!redacted.contains("sk-leaked"));
}

// =============================================================================
// DoD: rate/size/time limits prevent runaway ingest
// =============================================================================

#[test]
fn ingest_limits_enforce_size_cap() {
    let limits = IngestLimits {
        max_source_bytes: 1_000_000,
        max_sources_per_minute: 10,
        max_tokens_per_request: 4096,
    };
    assert!(limits.is_size_allowed(500_000));
    assert!(!limits.is_size_allowed(2_000_000));
}

#[test]
fn ingest_limits_enforce_rate_cap() {
    let limits = IngestLimits {
        max_source_bytes: 1_000_000,
        max_sources_per_minute: 10,
        max_tokens_per_request: 4096,
    };
    assert!(limits.is_rate_allowed(5));
    assert!(!limits.is_rate_allowed(15));
}
