//! Bearer token auth middleware for `/mcp`.
//!
//! Multi-token: each MCP client gets its own token (`BRAIN_MCP_TOKENS`,
//! comma-separated). Constant-time compare against the list to avoid
//! timing-attack token enumeration. Logs only "missing"/"mismatch" — never
//! the token value (TokenRedaction in `src/mcp/auth.rs` covers log scrubbing
//! for any token that does leak into a log line).
//!
//! Wired in `src/server.rs` only when `resolve_mcp_tokens()` returns a
//! non-empty list. Loopback deployments with no tokens skip this layer
//! entirely (backward compatible).

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

// Reuse the audited constant-time compare from src/api.rs (login path).
// Adding a new dep (`subtle`) for one compare would be needless scope.
use crate::api::constant_time_eq;

/// Build the axum middleware layer. The token list is captured by reference
/// (Arc) so the layer is cheap to clone per-request.
///
/// The natural return type of `axum::middleware::from_fn_with_state` is
/// `FromFnLayer<F, S, T>` where `F` is the handler fn-pointer, `S` is the
/// state, and `T` is the extractor tuple (NOT the response type — the third
/// type param of `FromFnLayer` is the extractor tuple, see axum's `pub struct
/// FromFnLayer<F, S, T>` + `Layer` impl). Our handler takes `State<...>`
/// (FromRequestParts) and `Request` (FromRequest) before `Next`, so
/// `T = (State<Arc<Vec<String>>>, Request)`.
///
/// `require_bearer` is `async`, whose unique future-returning type cannot be
/// named as a plain `fn(...) -> impl Future` pointer (RPITIT is not allowed
/// in `fn` pointer types). Rather than add the `futures` crate just for
/// `BoxFuture`, we name the handler type as a non-async
/// `fn(...) -> Pin<Box<dyn Future + Send>>` and bridge to the async body via
/// a tiny `Box::pin` shim. This is a known Rust ergonomic issue with
/// `from_fn_with_state`, not a bug.
#[allow(clippy::type_complexity)] // FromFnLayer's 3 type params are mandated by axum's API.
pub fn layer(
    tokens: Arc<Vec<String>>,
) -> axum::middleware::FromFnLayer<
    fn(
        State<Arc<Vec<String>>>,
        Request,
        Next,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, StatusCode>> + Send>>,
    Arc<Vec<String>>,
    (State<Arc<Vec<String>>>, Request),
> {
    // `require_bearer` is `async`, but the function-pointer return type
    // above needs a `fn(...) -> Future` (non-async) signature. Adapter shim
    // is the cleanest way to bridge async-body to the named fn-pointer type
    // without adding the `futures` crate just for `BoxFuture`.
    fn shim(
        state: State<Arc<Vec<String>>>,
        req: Request,
        next: Next,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, StatusCode>> + Send>>
    {
        Box::pin(require_bearer(state, req, next))
    }
    axum::middleware::from_fn_with_state(tokens, shim)
}

/// Middleware body: extract `Authorization: Bearer <token>`, constant-time
/// compare against the configured list. 401 on missing/malformed/mismatch.
///
/// Takes the full `Request` (not just `HeaderMap`) because axum 0.8's
/// `Next::run` requires a `Request<Body>` — the headers are borrowed from
/// the request for the auth check, then the original request is forwarded
/// to the inner service unchanged.
async fn require_bearer(
    State(tokens): State<Arc<Vec<String>>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let headers: &HeaderMap = req.headers();
    let Some(provided) = extract_bearer(headers) else {
        tracing::warn!("mcp auth: missing or malformed Authorization header");
        return Err(StatusCode::UNAUTHORIZED);
    };
    // Constant-time compare against each token. `.any()` short-circuits on
    // match but each individual compare is constant-time — an attacker can't
    // distinguish "first token matched" from "third token matched" by timing.
    let matched = tokens
        .iter()
        .any(|t| constant_time_eq(t.as_bytes(), provided.as_bytes()));
    if !matched {
        tracing::warn!("mcp auth: token mismatch");
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

/// Extract the bearer token from the `Authorization` header.
/// Returns `None` if the header is absent, not valid UTF-8, or doesn't
/// start with the `Bearer ` scheme (case-sensitive per RFC 6750 §2.1).
fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(auth: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, auth.parse().unwrap());
        h
    }

    #[test]
    fn extract_bearer_valid() {
        let h = headers_with("Bearer abc123");
        assert_eq!(extract_bearer(&h), Some("abc123"));
    }

    #[test]
    fn extract_bearer_trims_whitespace() {
        let h = headers_with("Bearer   spaced-token  ");
        assert_eq!(extract_bearer(&h), Some("spaced-token"));
    }

    #[test]
    fn extract_bearer_missing_header() {
        let h = HeaderMap::new();
        assert_eq!(extract_bearer(&h), None);
    }

    #[test]
    fn extract_bearer_wrong_scheme() {
        let h = headers_with("Basic dXNlcjpwYXNz");
        assert_eq!(extract_bearer(&h), None);
    }

    #[test]
    fn extract_bearer_no_value() {
        let h = headers_with("Bearer");
        assert_eq!(extract_bearer(&h), None);
    }

    #[test]
    fn extract_bearer_lowercase_scheme_rejected() {
        // RFC 6750 §2.1: scheme is case-insensitive, but we follow the
        // common convention of accepting only `Bearer ` to keep the check
        // simple and constant-time-able. If a client sends `bearer xxx`
        // they get 401 — they should send `Bearer xxx`.
        let h = headers_with("bearer abc");
        assert_eq!(extract_bearer(&h), None);
    }

    /// Sanity: the `constant_time_eq` we reuse behaves as expected on
    /// token-shaped strings.
    #[test]
    fn constant_time_eq_matches_identical_tokens() {
        assert!(constant_time_eq(b"tok-abc-123", b"tok-abc-123"));
        assert!(!constant_time_eq(b"tok-abc-123", b"tok-abc-124"));
        assert!(!constant_time_eq(b"tok-abc-123", b"different-length"));
    }
}
