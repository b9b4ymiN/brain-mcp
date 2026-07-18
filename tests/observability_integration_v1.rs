//! Task F2.1 — LogRedactor wired into the tracing pipeline (integration).
//!
//! Verifies the end-to-end path: a `tracing::info!` call carries a known
//! secret (Bearer token, sk- key, api_key= query, access_token= form) through
//! `tracing_subscriber::fmt` → `RedactingMakeWriter` → `RedactingWriter` →
//! `LogRedactor::redact` → in-memory sink. The captured bytes MUST contain
//! `[REDACTED]` and MUST NOT contain the raw secret value. A benign line MUST
//! survive untouched (no false positives corrupting ordinary logs).
//!
//! Each test installs the subscriber via `tracing::dispatcher::set_default`
//! (thread-local, restored on `DefaultGuard` drop) so nothing leaks into the
//! process-wide dispatcher and tests can run in parallel without interfering
//! with each other or with `observability_contract_v1`.

use std::io::Write;
use std::sync::{Arc, Mutex};

use llm_wiki::observability::RedactingMakeWriter;
use tracing_subscriber::fmt::MakeWriter;

/// In-memory sink shared between the subscriber and the test assertions.
/// `Mutex<Vec<u8>>` already implements `MakeWriter` (returning a
/// `MutexGuardWriter<Vec<u8>>`); our own `CaptureSink` adds an `io::Write`
/// impl so it can be plumbed through `RedactingMakeWriter` directly — the same
/// shape the production `init_logging` uses around `std::io::stderr` and
/// `tracing_appender::non_blocking::NonBlocking`.
#[derive(Clone, Default)]
struct CaptureSink(Arc<Mutex<Vec<u8>>>);

impl Write for CaptureSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureSink {
    type Writer = CaptureSink;
    fn make_writer(&'a self) -> Self::Writer {
        // Each call must hand back a writer that shares the SAME backing buffer
        // — otherwise the fmt layer's per-event writer would write to a
        // throwaway sink and the test would observe nothing.
        CaptureSink(Arc::clone(&self.0))
    }
}

/// Build a scoped subscriber over `RedactingMakeWriter<CaptureSink>`, install
/// it for the current thread, run `body`, and return the captured bytes.
fn with_redacting_subscriber(body: impl FnOnce()) -> String {
    let sink = CaptureSink::default();
    let make_writer = RedactingMakeWriter::new(sink.clone());

    let subscriber = tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(make_writer)
        .finish();

    let dispatch = tracing::dispatcher::Dispatch::new(subscriber);
    let guard = tracing::dispatcher::set_default(&dispatch);
    body();
    drop(guard);

    // The fmt layer writes complete lines synchronously; once the guard is
    // dropped and the subscriber is detached we can safely drain the buffer.
    let bytes = sink.0.lock().expect("sink poisoned").clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

// =============================================================================
// DoD: Bearer tokens are redacted in the tracing output
// =============================================================================

#[test]
fn redacts_bearer_token_in_tracing_output() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!("auth header: Authorization: Bearer sk-test-secret-12345");
    });

    assert!(
        captured.contains("[REDACTED]"),
        "expected [REDACTED] marker in: {captured}"
    );
    assert!(
        !captured.contains("sk-test-secret-12345"),
        "raw Bearer secret leaked into: {captured}"
    );
    // The marker prefix ("Bearer ") is intentionally preserved by the redactor
    // for debuggability — only the secret VALUE is stripped.
    assert!(
        captured.contains("Bearer [REDACTED]"),
        "expected 'Bearer [REDACTED]' marker pattern in: {captured}"
    );
}

// =============================================================================
// DoD: sk- OpenAI-style keys are redacted
// =============================================================================

#[test]
fn redacts_openai_style_sk_key() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!("calling provider with key=sk-proj-abcdef0123456789xyz");
    });

    assert!(
        captured.contains("[REDACTED]"),
        "no [REDACTED] in: {captured}"
    );
    assert!(
        !captured.contains("sk-proj-abcdef0123456789xyz"),
        "sk- key leaked into: {captured}"
    );
}

// =============================================================================
// DoD: api_key= query params are redacted
// =============================================================================

#[test]
fn redacts_api_key_query_param() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!("GET /mcp/v1/tool?api_key=sk-leaked1234567890abcdef");
    });

    assert!(
        captured.contains("[REDACTED]"),
        "no [REDACTED] in: {captured}"
    );
    assert!(
        !captured.contains("sk-leaked1234567890abcdef"),
        "api_key secret leaked into: {captured}"
    );
    // The "api_key=" marker must remain visible so operators see the leak
    // location without seeing the value.
    assert!(
        captured.contains("api_key=[REDACTED]"),
        "expected 'api_key=[REDACTED]' in: {captured}"
    );
}

// =============================================================================
// DoD: access_token= form values are redacted
// =============================================================================

#[test]
fn redacts_access_token_form_value() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!("oauth callback body: access_token=ya29.secret-token-0987654321");
    });

    assert!(
        captured.contains("[REDACTED]"),
        "no [REDACTED] in: {captured}"
    );
    assert!(
        !captured.contains("ya29.secret-token-0987654321"),
        "access_token value leaked into: {captured}"
    );
    assert!(
        captured.contains("access_token=[REDACTED]"),
        "expected 'access_token=[REDACTED]' in: {captured}"
    );
}

// =============================================================================
// DoD: benign log lines survive untouched (no false positives)
// =============================================================================

#[test]
fn preserves_benign_log_lines() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!("hello world");
    });

    assert!(
        captured.contains("hello world"),
        "benign line corrupted by redactor: {captured}"
    );
    assert!(
        !captured.contains("[REDACTED]"),
        "false-positive redaction on benign line: {captured}"
    );
}

// =============================================================================
// DoD: payload with multiple secrets redacts ALL of them
// =============================================================================

#[test]
fn redacts_multiple_secrets_in_single_line() {
    let captured = with_redacting_subscriber(|| {
        tracing::info!(
            "headers: Authorization: Bearer abc123XYZ_1234567890ff; \
             query: api_key=sk-multi-secret-aaaaaaaaaaaaaa"
        );
    });

    assert!(
        captured.contains("[REDACTED]"),
        "no [REDACTED] in: {captured}"
    );
    assert!(
        !captured.contains("abc123XYZ_1234567890ff"),
        "Bearer value leaked: {captured}"
    );
    assert!(
        !captured.contains("sk-multi-secret-aaaaaaaaaaaaaa"),
        "api_key value leaked: {captured}"
    );
}
