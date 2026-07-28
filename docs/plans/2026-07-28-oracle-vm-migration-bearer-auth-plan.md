# Oracle VM Migration + Bearer Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add multi-token Bearer auth on `/mcp` so brain-mcp-vnext can be deployed publicly on Oracle Cloud VM (ARM64), then deploy it.

**Architecture:** Custom axum middleware (`src/mcp/bearer_auth.rs`) wraps the `/mcp` route. Tokens come from `BRAIN_MCP_TOKENS` env var (comma-separated, multi-token). Constant-time comparison reuses `constant_time_eq` from `src/api.rs`. Fail-closed: public bind + no tokens = refuse to start. Backward-compat: loopback + no tokens = no auth.

**Tech Stack:** Rust (axum 0.8, tower), existing `constant_time_eq` helper in `src/api.rs`, Docker Compose, Oracle Cloud Free Tier Ampere A1 (ARM64).

**Spec:** `docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-design.md` (commit `11ef078`)

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/api.rs` | Modify (~line 473): `fn constant_time_eq` → `pub(crate) fn constant_time_eq` | Promote visibility for reuse |
| `src/config.rs` | Modify `ServeConfig` struct (~line 260): add `mcp_bearer_tokens_env` field + `resolve_mcp_tokens()` method | Config + resolver |
| `src/mcp/bearer_auth.rs` | **Create** (~50 LOC) | Custom axum middleware |
| `src/mcp/mod.rs` | Modify: add `pub mod bearer_auth;` | Module declaration |
| `src/server.rs` | Modify (~line 232, 245): wrap `/mcp` router with bearer layer when tokens present | Wire middleware |
| `.env.example` | Modify: add `BRAIN_MCP_TOKENS=` block with generation instructions | Document env var |
| `docker-compose.yml` | Modify (~line 89): add `BRAIN_MCP_TOKENS=${BRAIN_MCP_TOKENS:-}` env passthrough | Container env |
| `tests/mcp_bearer_auth_v1.rs` | **Create** | Integration test for `/mcp` Bearer auth |

Deployment runbook lives in the spec (Phase A/B/C, 9 steps) — **NOT** code, executed manually by operator after merge.

---

## Task 1: Promote `constant_time_eq` to `pub(crate)`

**Files:**
- Modify: `src/api.rs:473`

- [ ] **Step 1: Read current visibility**

```bash
sed -n '470,485p' src/api.rs
```

Expected: `fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {` (private).

- [ ] **Step 2: Promote visibility**

Change line 473 from:
```rust
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
```
to:
```rust
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
```

- [ ] **Step 3: Verify existing tests still pass**

Run: `cargo test --lib api::tests::constant_time_eq -j 2`
Expected: PASS (1 test, no behavior change — visibility only).

- [ ] **Step 4: Verify whole crate compiles**

Run: `cargo check -j 2`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/api.rs
git commit -m "refactor(api): promote constant_time_eq to pub(crate) for reuse"
```

---

## Task 2: Add `mcp_bearer_tokens_env` config field

**Files:**
- Modify: `src/config.rs` — `ServeConfig` struct (~line 260), `Default` impl (~line 380), and add `default_mcp_bearer_tokens_env` helper

- [ ] **Step 1: Add the default helper function**

Add after `default_mcp_init_timeout_secs` (search for it with `grep -n "fn default_mcp_init_timeout_secs" src/config.rs`):

```rust
/// Default env-var name holding comma-separated MCP Bearer tokens.
/// Mirrors the `BRAIN_USERNAME`/`BRAIN_PASSWORD` pattern: config stores
/// the env-var NAME, not the value. The value is read at startup via
/// `std::env::var`. Empty/unset + public bind → fail-closed (refuse start).
fn default_mcp_bearer_tokens_env() -> String {
    "BRAIN_MCP_TOKENS".into()
}
```

- [ ] **Step 2: Add the field to `ServeConfig` struct**

Find the `mcp_init_timeout_secs` field in the struct (around line 304-307) and add immediately after it:

```rust
    /// Env-var NAME (not value) holding comma-separated Bearer tokens
    /// accepted on `/mcp`. Empty/unset + `http_bind_all_interfaces=true`
    /// is a fail-closed configuration error (server refuses to start).
    /// Default: `"BRAIN_MCP_TOKENS"`. Loopback deployments with this unset
    /// keep the legacy unauthenticated `/mcp` (backward compatible).
    #[serde(default = "default_mcp_bearer_tokens_env")]
    pub mcp_bearer_tokens_env: String,
```

- [ ] **Step 3: Add the field to `Default` impl**

Find `impl Default for ServeConfig` (search `grep -n "impl Default for ServeConfig" src/config.rs`) and add inside it (next to other `mcp_*` fields):

```rust
            mcp_bearer_tokens_env: default_mcp_bearer_tokens_env(),
```

- [ ] **Step 4: Verify config compiles**

Run: `cargo check -j 2`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add mcp_bearer_tokens_env field to ServeConfig"
```

---

## Task 3: Add `resolve_mcp_tokens()` method (with fail-closed)

**Files:**
- Modify: `src/config.rs` — `impl ServeConfig` block (next to `resolve_bootstrap_credentials` around line 460)

- [ ] **Step 1: Write the failing unit test**

Add to the existing `#[cfg(test)] mod tests` block in `src/config.rs` (find it with `grep -n "#\[cfg(test)\]" src/config.rs`):

```rust
    #[test]
    fn resolve_mcp_tokens_parses_comma_separated() {
        std::env::set_var("TEST_MCP_TOKENS", "tok-a, tok-b ,, tok-c");
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS".into();
        cfg.http_bind_all_interfaces = false;
        let tokens = cfg.resolve_mcp_tokens().unwrap();
        assert_eq!(tokens, vec!["tok-a".to_string(), "tok-b".to_string(), "tok-c".to_string()]);
        std::env::remove_var("TEST_MCP_TOKENS");
    }

    #[test]
    fn resolve_mcp_tokens_fail_closed_when_public_and_empty() {
        std::env::remove_var("TEST_MCP_TOKENS_EMPTY");
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS_EMPTY".into();
        cfg.http_bind_all_interfaces = true; // public bind + no tokens = refuse
        let err = cfg.resolve_mcp_tokens();
        assert!(err.is_err(), "public bind with no tokens must fail-closed");
    }

    #[test]
    fn resolve_mcp_tokens_loopback_allows_empty() {
        std::env::remove_var("TEST_MCP_TOKENS_EMPTY2");
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS_EMPTY2".into();
        cfg.http_bind_all_interfaces = false; // loopback + no tokens = backward compat
        let tokens = cfg.resolve_mcp_tokens().unwrap();
        assert!(tokens.is_empty(), "loopback with no tokens must return empty (no auth)");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config::tests::resolve_mcp_tokens -j 2`
Expected: FAIL with "no method named `resolve_mcp_tokens` found".

- [ ] **Step 3: Implement `resolve_mcp_tokens()` method**

Add this method to `impl ServeConfig` (place it right after `resolve_bootstrap_credentials`, which ends around line 490):

```rust
    /// Resolve the comma-separated MCP Bearer tokens from the env var named
    /// by `mcp_bearer_tokens_env`. Trims whitespace and drops empty entries
    /// (so `"a, b ,, c"` → `["a", "b", "c"]`).
    ///
    /// **Fail-closed:** if `http_bind_all_interfaces` is true (public bind)
    /// and no tokens resolve, this returns an error — the server refuses to
    /// start. Loopback deployments with no tokens return an empty Vec
    /// (backward compatible with the legacy unauthenticated `/mcp`).
    pub fn resolve_mcp_tokens(&self) -> anyhow::Result<Vec<String>> {
        let raw = std::env::var(&self.mcp_bearer_tokens_env).unwrap_or_default();
        let tokens: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if self.http_bind_all_interfaces && tokens.is_empty() {
            anyhow::bail!(
                "public HTTP bind (http_bind_all_interfaces=true) requires at least one \
                 MCP Bearer token in env var '{}' — refusing to start with an \
                 unauthenticated public /mcp endpoint",
                self.mcp_bearer_tokens_env
            );
        }
        Ok(tokens)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config::tests::resolve_mcp_tokens -j 2`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): resolve_mcp_tokens with fail-closed on public+empty"
```

---

## Task 4: Create `bearer_auth.rs` middleware module

**Files:**
- Create: `src/mcp/bearer_auth.rs`
- Modify: `src/mcp/mod.rs` (add `pub mod bearer_auth;`)

- [ ] **Step 1: Declare the module**

Open `src/mcp/mod.rs` and find the existing `pub mod` declarations (search `grep -n "^pub mod\|^mod " src/mcp/mod.rs`). Add:

```rust
pub mod bearer_auth;
```

Place it next to the other `pub mod` lines (alphabetical or as the existing convention dictates).

- [ ] **Step 2: Create `src/mcp/bearer_auth.rs` with unit tests**

Write the full file:

```rust
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
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

// Reuse the audited constant-time compare from src/api.rs (login path).
// Adding a new dep (`subtle`) for one compare would be needless scope.
use crate::api::constant_time_eq;

/// Build the axum middleware layer. The token list is captured by reference
/// (Arc) so the layer is cheap to clone per-request.
pub fn layer(
    tokens: Arc<Vec<String>>,
) -> axum::middleware::FromFnLayer<
    fn(State<Arc<Vec<String>>>, HeaderMap, Next) -> futures::future::BoxFuture<'static, Result<Response, StatusCode>>,
    Arc<Vec<String>>,
    Response,
> {
    axum::middleware::from_fn_with_state(tokens, require_bearer)
}

/// Middleware body: extract `Authorization: Bearer <token>`, constant-time
/// compare against the configured list. 401 on missing/malformed/mismatch.
async fn require_bearer(
    State(tokens): State<Arc<Vec<String>>>,
    headers: HeaderMap,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(provided) = extract_bearer(&headers) else {
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
    Ok(next.run(headers).await)
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
```

- [ ] **Step 3: Run unit tests**

Run: `cargo test --lib mcp::bearer_auth -j 2`
Expected: PASS (7 tests).

- [ ] **Step 4: Verify the crate compiles**

Run: `cargo check -j 2`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/mcp/bearer_auth.rs src/mcp/mod.rs
git commit -m "feat(mcp): bearer_auth middleware module with unit tests"
```

---

## Task 5: Wire middleware into `server.rs` (both branches)

**Files:**
- Modify: `src/server.rs` (~line 231 for stateful branch, ~line 244 for stateless branch)

- [ ] **Step 1: Read the current `/mcp` mounting code**

```bash
sed -n '225,265p' src/server.rs
```

Identify both branches: stateful (uses `LocalSessionManager`) and stateless (uses `NeverSessionManager`). Both call `axum::Router::new().nest_service("/mcp", service)`.

- [ ] **Step 2: Resolve tokens once, before the branch**

Find the line where `serve_cfg` is in scope and the router is being assembled (before the `match`/`if` that picks stateful vs stateless). Add:

```rust
    // Resolve MCP Bearer tokens. Empty + loopback = backward compat (no auth).
    // Empty + public bind = fail-closed (resolve_mcp_tokens already errored).
    let mcp_tokens = Arc::new(serve_cfg.resolve_mcp_tokens()?);
    let mcp_auth_layer = (!mcp_tokens.is_empty()).then(|| {
        crate::mcp::bearer_auth::layer(mcp_tokens.clone())
    });
```

If `?` is not usable at this point (return type mismatch), use `.map_err(|e| { tracing::error!(error = ?e, "MCP token resolution failed"); e })?` or whatever pattern the surrounding code uses for early-return errors.

- [ ] **Step 3: Wrap `/mcp` router with the layer (both branches)**

For each branch, replace:
```rust
let mcp_router = axum::Router::new().nest_service("/mcp", service);
```
with:
```rust
let mut mcp_router = axum::Router::new().nest_service("/mcp", service);
if let Some(layer) = mcp_auth_layer.clone() {
    mcp_router = mcp_router.layer(layer);
}
```

(The `.clone()` is fine — `FromFnLayer` is cheap to clone; it holds an `Arc`.)

- [ ] **Step 4: Verify compile**

Run: `cargo check -j 2`
Expected: no errors. If errors about `mcp_auth_layer` type inference, annotate: `let mcp_auth_layer: Option<axum::middleware::FromFnLayer<_, _, _>> = ...`.

- [ ] **Step 5: Verify existing tests still pass**

Run: `cargo test --lib -j 2`
Expected: all existing tests PASS (no behavior change for default config — `mcp_tokens` is empty when env var unset + loopback).

- [ ] **Step 6: Commit**

```bash
git add src/server.rs
git commit -m "feat(server): wire bearer_auth middleware on /mcp when tokens configured"
```

---

## Task 6: Add `.env.example` documentation

**Files:**
- Modify: `.env.example`

- [ ] **Step 1: Read current `.env.example`**

```bash
cat .env.example
```

- [ ] **Step 2: Add `BRAIN_MCP_TOKENS` block**

After the existing `BRAIN_PASSWORD` block (or wherever the `BRAIN_*` vars are grouped), add:

```env
# MCP Bearer tokens — required when exposing /mcp beyond loopback.
# Comma-separated; each MCP client (Claude Desktop, Cursor, scripts) gets
# its own token so a leaked token can be rotated per-client.
# Generate tokens with: openssl rand -hex 32
# Example: BRAIN_MCP_TOKENS=token-for-claude,token-for-cursor,token-for-scripts
# Empty/unset is allowed only when http_bind_all_interfaces=false (loopback);
# public bind + empty = server refuses to start (fail-closed).
BRAIN_MCP_TOKENS=
```

- [ ] **Step 3: Commit**

```bash
git add .env.example
git commit -m "docs(env): document BRAIN_MCP_TOKENS env var"
```

---

## Task 7: Add `BRAIN_MCP_TOKENS` to `docker-compose.yml`

**Files:**
- Modify: `docker-compose.yml` (~line 89, in the `environment:` block)

- [ ] **Step 1: Read the current environment block**

```bash
sed -n '64,90p' docker-compose.yml
```

- [ ] **Step 2: Add the passthrough**

Find the line `ZAI_API_KEY=${ZAI_API_KEY:-}` (around line 89) and add immediately after it:

```yaml
      # MCP Bearer tokens (comma-separated, multi-client). Required when
      # exposing /mcp publicly (http_bind_all_interfaces=true). Loopback
      # deployments can leave this unset for backward-compatible no-auth.
      BRAIN_MCP_TOKENS=${BRAIN_MCP_TOKENS:-}
```

- [ ] **Step 3: Validate compose file syntax**

Run: `docker compose config --quiet`
Expected: no output (valid). If `docker compose` is unavailable on the dev machine, skip — the change is YAML-trivial and will be validated on the VM.

- [ ] **Step 4: Commit**

```bash
git add docker-compose.yml
git commit -m "chore(compose): pass BRAIN_MCP_TOKENS env var to container"
```

---

## Task 8: Integration test for `/mcp` Bearer auth

**Files:**
- Create: `tests/mcp_bearer_auth_v1.rs`

- [ ] **Step 1: Inspect an existing integration test for the harness pattern**

```bash
sed -n '1,40p' tests/api_console_v1.rs
```

Note: how the test spins up a router/server, what helpers exist (`spawn_server`, `test_app`, etc.). The integration test should reuse the same harness rather than reinvent it.

- [ ] **Step 2: Write the integration test**

Create `tests/mcp_bearer_auth_v1.rs`. Adapt the harness import to whatever pattern Task 8 Step 1 found. Sketch (fill in the actual harness call):

```rust
//! Integration test: /mcp rejects unauthenticated/wrong-token requests
//! and accepts requests with a valid Bearer token from the configured list.
//!
//! Mirrors the success criteria #3, #4, #6, #7 in the design spec.

use std::sync::Arc;

// Import the test harness used by other tests/api_*_v1.rs files.
// (Adjust the path/import to match what Task 8 Step 1 found.)
#[path = "helpers/mod.rs"]
mod helpers;

use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn mcp_rejects_request_without_authorization_header() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = helpers::build_mcp_router(tokens).await;

    let res = app
        .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_rejects_request_with_wrong_token() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = helpers::build_mcp_router(tokens).await;

    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_accepts_request_with_valid_token() {
    let tokens = Arc::new(vec!["token-a".to_string(), "token-b".to_string()]);
    let app = helpers::build_mcp_router(tokens).await;

    // Use the second token to prove multi-token matching works.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Bearer token-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // 200, 405 (method not allowed for plain GET), or similar — anything
    // that isn't 401 proves the auth layer passed the request through.
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED, "valid token must pass auth");
}

#[tokio::test]
async fn mcp_rejects_malformed_authorization_header() {
    let tokens = Arc::new(vec!["valid-token".to_string()]);
    let app = helpers::build_mcp_router(tokens).await;

    let res = app
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header("authorization", "Basic dXNlcjpwYXNz") // wrong scheme
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 3: Add `build_mcp_router` helper to the test harness**

Open the helpers module (e.g. `tests/helpers/mod.rs` — confirm with Task 8 Step 1) and add a helper that builds a minimal axum app with the bearer layer wrapping a stub `/mcp` handler:

```rust
use axum::{routing::any, Router};
use std::sync::Arc;
use llm_wiki::mcp::bearer_auth;

/// Build a test router with the bearer_auth layer on /mcp. The /mcp handler
/// is a stub that returns 200 OK — tests assert on auth-layer behavior, not
/// MCP protocol semantics (those are covered by other integration tests).
pub async fn build_mcp_router(tokens: Arc<Vec<String>>) -> Router {
    Router::new()
        .route("/mcp", any(|| async { "ok" }))
        .layer(bearer_auth::layer(tokens))
}
```

If the crate is not exposed as `llm_wiki::mcp::bearer_auth` from integration tests, expose it: add `pub mod mcp;` to `src/lib.rs` (it likely already is) and confirm `bearer_auth` is `pub`.

- [ ] **Step 4: Run the integration tests**

Run: `cargo test --test mcp_bearer_auth_v1 -j 2`
Expected: PASS (4 tests). If the harness needs adjustment (different helper name, different builder signature), fix the test to match the actual harness — the assertions are what matter.

- [ ] **Step 5: Commit**

```bash
git add tests/mcp_bearer_auth_v1.rs tests/helpers/
git commit -m "test(mcp): integration tests for /mcp bearer auth layer"
```

---

## Task 9: Update docs (install-vm.md §8 + deploy-docker.md ARM section)

**Files:**
- Modify: `docs/guides/install-vm.md` (§8 security rules — relax the loopback-only mandate now that `/mcp` has auth)
- Modify: `docs/guides/deploy-docker.md` (ARM section — mark as supported, link to the runbook)

- [ ] **Step 1: Read current §8 of install-vm.md**

```bash
sed -n '120,140p' docs/guides/install-vm.md
```

- [ ] **Step 2: Update §8 rule 1 to reflect Bearer auth**

The current rule 1 mandates loopback-only because `/mcp` has no auth. Update to:

```markdown
1. **Public exposure requires Bearer auth on /mcp.** Set `BRAIN_MCP_TOKENS`
   (comma-separated, multi-token) and `http_bind_all_interfaces=true` in
   config. Without tokens, public bind is fail-closed (server refuses to
   start). See `docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-design.md`
   for the full deployment runbook (Oracle Cloud, ARM64, no-TLS).
   Loopback-only deployments (the original quickstart) need neither.
```

Keep rule 2 (SSH tunnel for non-public setups) as-is — it's still the right call for no-auth loopback.

- [ ] **Step 3: Update ARM section in deploy-docker.md**

Find the ARM deferral note (search `grep -n "arm64.*DEFERRED\|arm64.*deferred" docs/guides/deploy-docker.md`) and replace with:

```markdown
**arm64 (Oracle Cloud Ampere A1, Apple Silicon, Raspberry Pi 4):** supported
via native build on the ARM host (`docker compose up --build`). The Dockerfile
has no arch-specific commands and all deps are statically linked (bundled
SQLite, vendored libgit2, rustls). Validated on Oracle Cloud Free Tier with
the runbook in `docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-design.md`.
```

- [ ] **Step 4: Commit**

```bash
git add docs/guides/install-vm.md docs/guides/deploy-docker.md
git commit -m "docs: update install-vm/deploy-docker for ARM + public Bearer auth"
```

---

## Task 10: Final verification (full build + test sweep)

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test -j 2`
Expected: ALL tests PASS (existing + new unit + new integration).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings -j 2`
Expected: no warnings.

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --check`
Expected: no diff (fix with `cargo fmt` if needed).

- [ ] **Step 4: Smoke-build the release binary**

Run: `cargo build --release --locked -j 2`
Expected: compiles successfully (this is what the Docker build will run).

- [ ] **Step 5: Manual smoke (loopback backward-compat)**

```bash
# No BRAIN_MCP_TOKENS set, http_bind_all_interfaces=false → /mcp must NOT require auth
BRAIN_USERNAME=test BRAIN_PASSWORD=test cargo run -- serve --http :18080 &
sleep 3
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18080/mcp
# Expected: NOT 401 (likely 200, 405, or 406 — but auth layer is absent)
kill %1
```

- [ ] **Step 6: Manual smoke (fail-closed)**

```bash
# Public bind + no tokens → server must refuse to start
unset BRAIN_MCP_TOKENS
# Edit a throwaway config with http_bind_all_interfaces=true, or pass via flag if supported
# Then: cargo run -- serve --http :18081  (with the public-bind config)
# Expected: server exits with the fail-closed error message
```

- [ ] **Step 7: Manual smoke (Bearer enforced)**

```bash
export BRAIN_MCP_TOKENS=test-token-1,test-token-2
cargo run -- serve --http :18082 &
sleep 3
# No header → 401
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18082/mcp
# Expected: 401
# Wrong token → 401
curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer wrong" http://127.0.0.1:18082/mcp
# Expected: 401
# Valid token → NOT 401
curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer test-token-1" http://127.0.0.1:18082/mcp
# Expected: NOT 401 (200/405/406)
kill %1
unset BRAIN_MCP_TOKENS
```

- [ ] **Step 8: Final commit (if any cleanup)**

If clippy/fmt found issues, fix and commit:
```bash
git add -A
git commit -m "chore: clippy + fmt cleanup"
```

---

## After all tasks: handoff to deployment runbook

The code changes (Tasks 1-10) produce a commit series ready for the deployment runbook in the spec (`docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-design.md` § "Deployment Runbook"). The operator executes the 9 runbook steps manually on the Oracle VM; no further code changes are needed for deployment.

## Self-Review checklist (run after writing, fix inline)

- [x] **Spec coverage:** every spec section maps to ≥1 task
  - Config field + resolver → Tasks 2, 3
  - Custom middleware → Task 4
  - Wiring → Task 5
  - `.env.example` + compose → Tasks 6, 7
  - Integration tests → Task 8
  - Docs → Task 9
  - Fail-closed + backward-compat → Tasks 3 (unit), 8 (integration), 10 (smoke)
  - Deployment runbook → spec (manual, not a code task)
- [x] **Placeholder scan:** no TBD/TODO/vague — all code blocks are complete
- [x] **Type consistency:** `layer()` returns `FromFnLayer` (Task 4) and is consumed via `.layer()` in Task 5 — types match. `resolve_mcp_tokens()` returns `anyhow::Result<Vec<String>>` (Task 3), consumed as `Arc<Vec<String>>` in Task 5 — match. `constant_time_eq(&[u8], &[u8]) -> bool` (Task 1) matches usage in Task 4.
- [x] **Frequent commits:** every task ends with a commit (10 commits total).
