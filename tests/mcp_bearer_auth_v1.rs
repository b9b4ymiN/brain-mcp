//! Integration test: `/mcp` bearer auth middleware.
//!
//! Verifies the end-to-end behavior of the `bearer_auth` layer applied to an
//! axum `Router` — complementing the unit tests in `src/mcp/bearer_auth.rs`
//! (which only exercise `extract_bearer` and `constant_time_eq` in isolation).
//! Here we build a real `Router` with the layer wrapping a stub `/mcp`
//! handler and drive real `Request`s through it via `tower::ServiceExt::oneshot`,
//! so the State extraction, header parsing, and layer wiring are all exercised
//! exactly as `src/server.rs` wires them in production.
//!
//! Success criteria mapped (plan `docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-plan.md`):
//! - no Authorization header        -> 401  (criteria #3)
//! - wrong token                    -> 401  (criteria #3)
//! - valid token from the list      -> not 401  (criteria #4, multi-token)
//! - malformed scheme (Basic ...)   -> 401  (edge case)

use std::sync::Arc;

use axum::{Router, body::Body, extract::Request, http::StatusCode, routing::any};
use llm_wiki::mcp::bearer_auth;
// axum 0.8 re-exports a *different* `ServiceExt` (without `oneshot`); we need
// tower's. `tower` is a direct `[dev-dependencies]` entry for this reason.
use tower::ServiceExt;

/// Build a minimal axum app with the `bearer_auth` layer on `/mcp`.
///
/// The `/mcp` handler is a stub that returns 200 `"ok"` — these tests assert
/// on auth-layer behavior, not MCP protocol semantics (those are covered by
/// the other `tests/mcp_*` integration tests). If auth fails the layer
/// short-circuits with 401 and the handler never runs; if auth passes the
/// request reaches the handler and we get 200.
fn build_mcp_router(tokens: Arc<Vec<String>>) -> Router {
    Router::new()
        .route("/mcp", any(|| async { "ok" }))
        .layer(bearer_auth::layer(tokens))
}

#[tokio::test]
async fn mcp_rejects_request_without_authorization_header() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = build_mcp_router(tokens);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router oneshot");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_rejects_request_with_wrong_token() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = build_mcp_router(tokens);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router oneshot");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_accepts_request_with_valid_token() {
    // Two-token list proves multi-token matching: the SECOND token must also
    // pass the auth layer, not just the first.
    let tokens = Arc::new(vec!["token-a".to_string(), "token-b".to_string()]);
    let app = build_mcp_router(tokens);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Bearer token-b")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router oneshot");
    // Anything that isn't 401 proves the auth layer passed the request
    // through to the stub handler (which returns 200 "ok").
    assert_ne!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "valid token must pass the auth layer"
    );
}

#[tokio::test]
async fn mcp_rejects_malformed_authorization_header() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = build_mcp_router(tokens);

    // A non-Bearer scheme (Basic here) must be rejected — only `Bearer <tok>`
    // is accepted per RFC 6750 §2.1 as implemented by `extract_bearer`.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router oneshot");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
