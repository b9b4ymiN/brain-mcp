//! One-shot smoke for Task F2.1 — verifies that an `info!` line carrying a
//! fake secret is redacted at the writer boundary when the subscriber is built
//! the same way `init_logging` builds it (`RedactingMakeWriter::new(io::stderr)`
//! + `fmt::layer().compact()`). Run with:
//!
//! ```bash
//! cargo run --example verify_log_redaction
//! ```
//!
//! Expected: the printed line contains `Bearer [REDACTED]` and does NOT
//! contain `sk-proj-fakeSecretABCD1234567890XYZ`. No source-file modification
//! needed for the smoke — it mirrors the production wire shape exactly.

use std::io::Write;

use llm_wiki::observability::RedactingMakeWriter;
use tracing_subscriber::prelude::*;

fn main() {
    // === Path 1: stderr (mirrors init_logging's compact+stderr branch). ===
    let stderr_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_writer(RedactingMakeWriter::new(std::io::stderr));

    // Use a permissive filter for the smoke so all emitted lines reach stderr.
    let env_filter = tracing_subscriber::EnvFilter::new("trace");
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .init();

    tracing::info!("SMOKE-START benign marker: hello world");
    tracing::info!(
        "SMOKE-SECRET auth header: Authorization: Bearer sk-proj-fakeSecretABCD1234567890XYZ"
    );
    tracing::info!("SMOKE-SECRET query: GET /tool?api_key=sk-leaked-MULTI1234567890abcd");
    tracing::info!("SMOKE-END benign marker: goodbye world");

    // Force a stdout sync so the cargo-run wrapper reliably flushes before exit.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
}
