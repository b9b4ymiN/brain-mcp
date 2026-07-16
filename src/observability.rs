//! Observability + operations contract (Task 6.2).
//!
//! GOAL-vNext §13 Task 6.2 + §12.2: structured metrics (Prometheus-format),
//! log redaction (no token/source secrets leak), ingest rate/size/time limits.
//! Contract-level: the deployment layer wires these to a real metrics/log
//! pipeline.

use serde::{Deserialize, Serialize};

// ── Metrics ──────────────────────────────────────────────────────────────────

/// Metric kind (Prometheus-compatible).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

/// One metric data point. §12.2 `brain_mcp_tool_calls_total{tool,status}` etc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricPoint {
    pub kind: MetricKind,
    pub name: String,
    pub value: f64,
}

// ── Log redaction ────────────────────────────────────────────────────────────

/// Strip secrets from log lines before they reach the log pipeline. §6.2
/// "log redaction tests ไม่รั่ว token/source secrets". Reuses the provider
/// module's `detect_secret` patterns (Bearer, sk-, access_token=, api_key=).
pub struct LogRedactor;

impl LogRedactor {
    /// Redact known secret patterns from `input`, replacing values with
    /// `[REDACTED]`.
    pub fn redact(input: &str) -> String {
        crate::provider::OutboundPolicy::check_text_redact(input)
    }
}

// ── Ingest limits ────────────────────────────────────────────────────────────

/// Rate/size/time limits to prevent runaway ingest and provider cost/quota.
/// §6.2 "rate/size/time limits ป้องกัน runaway ingest และ provider cost/quota".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestLimits {
    pub max_source_bytes: u64,
    pub max_sources_per_minute: u32,
    pub max_tokens_per_request: u32,
}

impl IngestLimits {
    /// True if `bytes` is within the size cap.
    pub fn is_size_allowed(&self, bytes: u64) -> bool {
        bytes <= self.max_source_bytes
    }

    /// True if `count_this_minute` is within the rate cap.
    pub fn is_rate_allowed(&self, count_this_minute: u32) -> bool {
        count_this_minute < self.max_sources_per_minute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_basics() {
        let l = IngestLimits {
            max_source_bytes: 100,
            max_sources_per_minute: 5,
            max_tokens_per_request: 100,
        };
        assert!(l.is_size_allowed(50));
        assert!(!l.is_size_allowed(200));
        assert!(l.is_rate_allowed(4));
        assert!(!l.is_rate_allowed(6));
    }
}
