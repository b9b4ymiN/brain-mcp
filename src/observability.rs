//! Observability + operations contract (Task 6.2).
//!
//! GOAL-vNext §13 Task 6.2 + §12.2: structured metrics (Prometheus-format),
//! log redaction (no token/source secrets leak), ingest rate/size/time limits.
//! Contract-level: the deployment layer wires these to a real metrics/log
//! pipeline.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use tracing::Metadata;
use tracing_subscriber::fmt::MakeWriter;

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

// ── Tracing writer plumbing (Task F2.1) ──────────────────────────────────────

/// `io::Write` adapter that runs every buffered chunk through
/// [`LogRedactor::redact`] before forwarding to the inner writer. Used to plug
/// redaction into a `tracing_subscriber::fmt` layer at the line boundary (one
/// place to maintain, instead of every `info!`/`debug!` call site).
///
/// Reports the ORIGINAL buffer length on success (`Ok(buf.len())`) — never the
/// redacted length. `tracing-subscriber` tracks bytes accepted vs. actually
/// written; reporting a different length on a successful write would desync its
/// accounting and could trigger spurious retries or lost trailing bytes when
/// the redacted string is shorter than the input.
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Lossy decode: log payloads are UTF-8 by construction in tracing, and
        // any byte-level corruption (rare) is preferable to dropping the line.
        let text = String::from_utf8_lossy(buf);
        let redacted = LogRedactor::redact(&text);
        self.inner.write_all(redacted.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// `MakeWriter` adapter that wraps each per-event writer produced by `inner`
/// in a [`RedactingWriter`]. Satisfies the `for<'a> MakeWriter<'a>` higher-rank
/// bound that `tracing_subscriber::fmt::Layer::with_writer` requires whenever
/// `inner` does — this lets us redact across all existing writer kinds in
/// `init_logging` (`std::io::stderr`, `tracing_appender::non_blocking::NonBlocking`,
/// and the `Arc<Mutex<Vec<u8>>>` test writer) without changing the call sites.
pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    /// Wrap an existing `MakeWriter` so its output is redacted line-by-line.
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer(),
        }
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer_for(meta),
        }
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

    #[test]
    fn redacting_writer_redacts_bearer_and_reports_original_len() {
        let mut sink: Vec<u8> = Vec::new();
        let mut w = RedactingWriter { inner: &mut sink };
        let line = "Authorization: Bearer sk-test-secret-1234567890ab";
        let n = w.write(line.as_bytes()).expect("write");
        // Reports the ORIGINAL (un-redacted) length, never the shorter one —
        // tracing's internal byte accounting depends on this.
        assert_eq!(n, line.len());
        let out = String::from_utf8_lossy(&sink);
        assert!(out.contains("[REDACTED]"), "got: {out}");
        assert!(!out.contains("sk-test-secret-1234567890ab"), "got: {out}");
    }

    #[test]
    fn redacting_writer_preserves_benign_text() {
        let mut sink: Vec<u8> = Vec::new();
        let mut w = RedactingWriter { inner: &mut sink };
        let line = "hello world — nothing to redact here";
        w.write_all(line.as_bytes()).expect("write_all");
        assert_eq!(String::from_utf8_lossy(&sink), line);
    }
}
