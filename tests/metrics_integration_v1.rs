//! Task F2.2 — `/metrics` Prometheus endpoint + metric registry (integration).
//!
//! Verifies the end-to-end metric pipeline:
//!
//! 1. The global Prometheus recorder installs once per process (parallel-test
//!    safe via a `OnceLock`).
//! 2. `GET /metrics` returns 200 + `Content-Type: text/plain` + a body that
//!    contains Prometheus exposition markers (`# TYPE` lines).
//! 3. Real handler-side counters increment observably: a wrong-secret login
//!    bumps `console_auth_failures_total`; a correct login bumps
//!    `console_logins_total`. The test reads the counter value back out of
//!    `/metrics` before + after to prove the wire is connected.
//!
//! Test router shape mirrors `server.rs`: `/metrics` at the top level (NOT
//! under `/api/v1`), `/api/v1/*` nested for the Console auth routes. The
//! metrics handler itself is a thin wrapper around
//! `observability::render_prometheus()` — same shape as
//! `server::metrics_handler`, duplicated here because the production handler
//! is private and the test needs to mount its own listener.

use std::sync::OnceLock;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use llm_wiki::api::{ConsoleApiState, router as console_router};
use llm_wiki::observability;
use serde_json::json;
use tempfile::TempDir;

const SECRET: &str = "dev-bootstrap-secret-metrics";

// ── Recorder install (once per process) ──────────────────────────────────────
//
// `metrics::set_global_recorder` panics on the second install. The integration
// tests run in parallel under one process, so the first test to land here
// installs the recorder; the rest see it already in place and skip. The
// recorder persists for the lifetime of the test process; counters accumulate
// across tests — that's why each test reads the value BEFORE + AFTER its
// action rather than asserting an absolute value.

static RECORDER_INSTALLED: OnceLock<()> = OnceLock::new();

fn ensure_recorder() {
    RECORDER_INSTALLED.get_or_init(|| {
        // Failure here is non-fatal but would make every assertion below
        // degenerate (no metrics recorded). Surface it loudly so the test
        // author sees the cause rather than a confusing "counter not found".
        if let Err(e) = observability::init_recorder() {
            panic!("init_recorder failed in test setup: {e:?}");
        }
    });
}

// ── Test router ──────────────────────────────────────────────────────────────

/// `/metrics` handler for the test router. Mirrors `server::metrics_handler`:
/// 200, `text/plain; version=0.0.4`, no-cache, body = the live Prometheus
/// exposition. Does NOT refresh the store-derived gauges (the test only
/// asserts counter behavior, and that side-effect needs a live store).
async fn metrics_handler() -> impl IntoResponse {
    let body = observability::render_prometheus();
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
}

fn make_store() -> (TempDir, std::sync::Arc<llm_wiki::semantic::SemanticStore>) {
    use llm_wiki::semantic::{SemanticConfig, SemanticStore};
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    (parent, std::sync::Arc::new(store))
}

async fn spawn(state: ConsoleApiState) -> String {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .nest("/api/v1", console_router(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// POST `/api/v1/auth/login` with the given secret. Returns the HTTP status.
async fn login_status(client: &reqwest::Client, base: &str, secret: &str) -> reqwest::StatusCode {
    client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "secret": secret }))
        .send()
        .await
        .expect("login send")
        .status()
}

/// GET `/metrics` and return the body text.
async fn metrics_body(
    client: &reqwest::Client,
    base: &str,
) -> (reqwest::StatusCode, String, String) {
    let resp = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .expect("metrics send");
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = resp.text().await.expect("metrics body");
    (status, ct, body)
}

/// Parse a Prometheus counter value out of a `# TYPE ... counter` +
/// `name value` pair. Matches `<name>{labels} <number>` OR `<name> <number>`.
/// Returns 0.0 when the metric is absent (counter never incremented).
fn counter_value(body: &str, name: &str) -> f64 {
    // The exposition format is:
    //   # TYPE <name> counter
    //   <name>{label="..."} <value>
    //   <name> <value>           # no labels
    //
    // We sum every sample line whose leading token matches `name` (so a
    // counter with label variants — e.g. `mcp_calls_total{tool="..."}` —
    // reports the SUM across variants, which is what callers want for "did
    // this counter go up?").
    let mut total = 0.0f64;
    let mut saw_type = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# TYPE ") {
            let mut iter = rest.split_whitespace();
            if iter.next() == Some(name) {
                saw_type = true;
            }
            continue;
        }
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        // Sample line: `<name>[{...}] <value> [timestamp]`
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
    let _ = saw_type; // presence flag retained for debugging; not asserted here
    total
}

// =============================================================================
// DoD: /metrics returns Prometheus text exposition
// =============================================================================

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_text() {
    ensure_recorder();
    let (_parent, store) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    // Bump a known counter so it shows up in the exposition. A wrong-secret
    // login is the cheapest reachable path.
    assert_eq!(
        login_status(&client, &base, "wrong").await,
        StatusCode::UNAUTHORIZED
    );

    let (status, content_type, body) = metrics_body(&client, &base).await;

    assert_eq!(status, StatusCode::OK, "/metrics must return 200");
    assert!(
        content_type.starts_with("text/plain"),
        "/metrics Content-Type must be text/plain, got: {content_type}"
    );
    assert!(
        body.contains("# TYPE"),
        "/metrics body must contain Prometheus # TYPE lines, got:\n{body}"
    );
    assert!(
        body.contains("console_auth_failures_total"),
        "/metrics body must contain the bumped counter name, got:\n{body}"
    );
}

// =============================================================================
// DoD: counter increments after a wrong-secret login
// =============================================================================

#[tokio::test]
async fn auth_failure_counter_increments() {
    ensure_recorder();
    let (_parent, store) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    // Snapshot BEFORE.
    let (_, _, body_before) = metrics_body(&client, &base).await;
    let before = counter_value(&body_before, "console_auth_failures_total");

    // Fire one wrong-secret login.
    assert_eq!(
        login_status(&client, &base, "nope").await,
        StatusCode::UNAUTHORIZED
    );

    // Snapshot AFTER — must be strictly greater.
    let (_, _, body_after) = metrics_body(&client, &base).await;
    let after = counter_value(&body_after, "console_auth_failures_total");
    assert!(
        after > before,
        "console_auth_failures_total must increase after a wrong-secret login: \
         before={before}, after={after}"
    );
}

// =============================================================================
// DoD: counter increments after a successful login
// =============================================================================

#[tokio::test]
async fn login_success_counter_increments() {
    ensure_recorder();
    let (_parent, store) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    // Snapshot BEFORE.
    let (_, _, body_before) = metrics_body(&client, &base).await;
    let before = counter_value(&body_before, "console_logins_total");

    // Fire one correct login. Re-auth increments the same counter, so this
    // test does not reauth — login alone is enough to prove the wire.
    assert_eq!(login_status(&client, &base, SECRET).await, StatusCode::OK);

    // Snapshot AFTER — must be strictly greater.
    let (_, _, body_after) = metrics_body(&client, &base).await;
    let after = counter_value(&body_after, "console_logins_total");
    assert!(
        after > before,
        "console_logins_total must increase after a correct login: \
         before={before}, after={after}"
    );
}

// =============================================================================
// DoD: render_prometheus is callable directly + idempotent re-install
// =============================================================================

#[test]
fn render_prometheus_returns_text_when_recorder_installed() {
    ensure_recorder();
    // Once a recorder is installed the exposition is non-empty as soon as ANY
    // counter has been touched. The other tests in this file have already
    // bumped counters; we additionally bump one here so this test is
    // order-independent.
    metrics::counter!("metrics_integration_self_test_total").increment(1);
    let body = observability::render_prometheus();
    assert!(
        body.contains("# TYPE") || body.contains("metrics_integration_self_test_total"),
        "render_prometheus must yield Prometheus exposition text, got:\n{body}"
    );
}

#[test]
fn init_recorder_is_idempotent() {
    ensure_recorder();
    // A second call MUST succeed (init_recorder guards the global install).
    // If it ever returned Err here, the parallel-test harness would panic
    // on the second test that landed.
    observability::init_recorder().expect("init_recorder must be idempotent");
}
