//! Console HTTP JSON API (Phase E Task E0.2).
//!
//! A thin axum layer that lets the (future) Svelte Console authenticate and
//! drive the semantic store. Every route reaches storage ONLY through
//! [`crate::semantic::SemanticStore`] public methods — never SQLite/Git/index
//! directly (§9, and the whole architectural point of this task). The MCP
//! transport already speaks this contract; the Console needs a browser-shaped
//! surface (session cookie + CSRF) that the MCP protocol can't give it.
//!
//! Auth is deliberately dev-grade: a bootstrap secret mints an in-memory
//! session plus a double-submit CSRF token. Production OAuth is Phase F. The
//! whole router is fail-closed at the wiring layer — [`crate::server`] only
//! mounts it when a bootstrap secret is configured (the F1 lesson: the gate
//! must be active on the real serve path, not just in a contract).

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, request::Parts};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::galaxy::{GalaxyGraph, ZoomLevel};
use crate::semantic::{
    ConfirmByProposalIdCommand, MergeEntitiesCommand, PredicateAssignment,
    RejectByProposalIdCommand, RetractCommand, SemanticError, SemanticStore, SplitCommand,
    SplitOutcome, SupersedeByProposalIdCommand, TrustedContext,
};
use crate::trust::{DestructiveAction, DestructiveWarning};

const SESSION_COOKIE: &str = "brain_console_session";
const CSRF_HEADER: &str = "X-CSRF-Token";

/// How long a recent re-authentication stays "fresh" for destructive actions
/// (§5.3 "recent re-auth"). A session may invoke `/purge/execute` only while
/// `now - reauthenticated_at <= PURGE_REAUTH_FRESHNESS`. The window is short on
/// purpose: hard purge is irreversible, so the gate forces a fresh proof of
/// the bootstrap secret close to the moment of impact.
const PURGE_REAUTH_FRESHNESS: Duration = Duration::seconds(300);

/// Ring-buffer depth for the in-process event broadcast. A subscriber that
/// falls this far behind gets a `Lagged` signal (surfaced as an SSE comment)
/// rather than blocking any publisher — events are advisory notifications.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Content-Security-Policy applied to every static (console asset) response.
/// Strict baseline: only same-origin scripts, no framing, no plugins,
/// no `<base>` hijack.
///
/// `style-src 'self' 'unsafe-inline'` — the `'unsafe-inline'` for styles is
/// required because Svelte 5 emits scoped `<style>` blocks with hashed class
/// names AND several components use dynamic inline `style="..."` attributes
/// for computed values (galaxy canvas height, star-shadow CSS custom props,
/// toast kind theming). This is the standard posture for Svelte/Vue/Angular
/// SPAs — `'unsafe-inline'` for styles is low-risk (styles can't execute
/// code) and is what Stripe, Vercel, and GitHub all ship. Scripts remain
/// `'self'` only (no `'unsafe-inline'` / `'unsafe-eval'` — that WOULD be
/// dangerous).
///
/// `font-src 'self'` — fonts are self-hosted in `/fonts/` (P0-1 fix,
/// 2026-07-20). No cross-origin font requests.
///
/// `img-src 'self' data:` — the favicon is same-origin; `data:` allows
/// inline data-URI images used by some icon patterns.
///
/// `connect-src 'self'` — fetch + SSE only to same origin (the dev proxy
/// or prod same-origin backend).
pub const CONSOLE_CSP: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     font-src 'self'; \
     img-src 'self' data:; \
     connect-src 'self'; \
     object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

// ── state ───────────────────────────────────────────────────────────────────

/// A live Console session. Server-side half of the auth pair; the CSRF token
/// is echoed to the client in the login response body (NOT in the HttpOnly
/// cookie) so the frontend JS can read it and resubmit it on mutations.
///
/// `issued_at` records when the session was minted (login time) and is
/// immutable for the session's life. `reauthenticated_at` is bumped by
/// Phase 1.6 — `/inbox` row: a pending proposal plus the same-scope
/// conflicts it participates in. Conflicts cover both peer proposals
/// (`Pending` status) and confirmed claims (`Confirmed` status) for the
/// same `(domain, subject, predicate)`. `conflicts` is empty when the
/// proposal has no peers in scope — the Console renders the
/// "Approve will create a new claim" hint in that case.
///
/// Replaces the old "N current confirmed claims in scope" text, which always
/// showed 0 because the inbox was pending-only and the query was
/// confirmed-only. See `src/inbox_conflicts.rs` for the detection algorithm.
#[derive(Serialize)]
pub struct InboxProposal {
    #[serde(flatten)]
    pub proposal: crate::semantic::ProposalSummary,
    /// Phase 1.6 — conflicts detected for this proposal. Currently limited
    /// to `HardValue` (>0.1% scalar diff) and `Duplicate` (same value);
    /// see [`crate::inbox_conflicts::ConflictKind`].
    pub conflicts: Vec<crate::inbox_conflicts::ScopeConflict>,
}

/// `/auth/reauth` and is the freshness marker the destructive-action gate
/// (`/purge/execute`) checks — `None` means "never re-authed since login", so
/// the very first hard-purge attempt always requires a re-auth even on a
/// brand-new session.
struct Session {
    context: TrustedContext,
    csrf_token: String,
    expires_at: DateTime<Utc>,
    issued_at: DateTime<Utc>,
    reauthenticated_at: Option<DateTime<Utc>>,
}

/// Shared state for the Console API router. Cheap to clone — every field is
/// behind an `Arc`. Held in axum's [`State`].
#[derive(Clone)]
pub struct ConsoleApiState {
    store: Arc<SemanticStore>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    /// The configured password (the legacy "bootstrap secret"). Compared in
    /// constant time against either `body.password` (new username+password
    /// mode) or `body.secret` (legacy single-credential mode) at login.
    bootstrap_secret: Arc<String>,
    /// The configured username, if any. `None` = legacy single-credential
    /// mode (the login route accepts `{secret}` alone). `Some(u)` = new
    /// username+password mode (the login route requires `{username, password}`
    /// and both must match).
    bootstrap_username: Arc<Option<String>>,
    session_ttl: Duration,
    /// Destructive-action re-auth freshness window (§5.3 "recent re-auth").
    /// `/purge/execute` rejects a session whose freshness anchor
    /// (`reauthenticated_at.unwrap_or(issued_at)`) is older than this. Defaults
    /// to [`PURGE_REAUTH_FRESHNESS`]; the separate field exists so the contract
    /// tests can exercise the gate without waiting 300s in real time.
    reauth_freshness: Duration,
    /// Adds `Secure` to the session cookie. Set when the server binds a
    /// non-loopback interface (mirrors the existing bind-address posture);
    /// omitted for loopback dev so cookies work over plain http.
    secure_cookie: bool,
    /// In-process fan-out for job/update notifications delivered over the SSE
    /// `/events` stream. Held as the `Sender` half so any future in-process
    /// publisher can `subscribe()`/`send()`; the initial `Receiver` is dropped.
    events: Arc<broadcast::Sender<String>>,
    /// Optional AI provider for the Phase 3 AI semantic rules in `ai_review`.
    /// When `None`, AI rules are silently skipped (deterministic-only).
    /// Shared with MCP via `Arc` (single adapter instance — see
    /// `server::serve`).
    ai_provider: Option<Arc<dyn crate::provider::AiProvider>>,
    /// Optional Subject Validator (Phase 1.5). When `None`, deterministic
    /// quality checks run without subject-validation tags (legacy mode). Set
    /// at boot via [`Self::with_subject_validator`] from the loaded rules.
    pub subject_validator: Option<std::sync::Arc<crate::subject_validator::SubjectValidator>>,
    /// Optional handle to the `WikiEngine`. When `None` (the default in tests
    /// that build a bare `SemanticStore`), endpoints that need the engine
    /// (`/status`, `/index-status`, `/index/update`, `/index/rebuild`, `/config`)
    /// return `internal_error`. When `Some` (production wiring via
    /// [`Self::with_engine`]), those endpoints can read `EngineState` and call
    /// `ops::stats` / `ops::index::*`.
    pub engine: Option<Arc<crate::engine::WikiEngine>>,
}

impl ConsoleApiState {
    /// Builds state with the default 24h session TTL. Legacy single-credential
    /// constructor — kept for backward compatibility with tests that exercise
    /// the `{secret}` login shape. Equivalent to
    /// `Self::with_credentials(store, None, bootstrap_secret, secure_cookie)`.
    pub fn new(store: Arc<SemanticStore>, bootstrap_secret: String, secure_cookie: bool) -> Self {
        Self::with_credentials(store, None, bootstrap_secret, secure_cookie)
    }

    /// Builds state with explicit credentials and the default 24h session
    /// TTL. When `username` is `Some`, the login route requires both
    /// `{username, password}` to match; when `None`, the login route falls
    /// back to legacy `{secret}` mode (matching `password` against `body.secret`).
    pub fn with_credentials(
        store: Arc<SemanticStore>,
        username: Option<String>,
        password: String,
        secure_cookie: bool,
    ) -> Self {
        Self::with_credentials_and_ttl(
            store,
            username,
            password,
            secure_cookie,
            Duration::hours(24),
        )
    }

    /// Builds state with explicit credentials AND an explicit session TTL
    /// (test seam for expiry). Replaces the old `with_session_ttl`.
    pub fn with_credentials_and_ttl(
        store: Arc<SemanticStore>,
        username: Option<String>,
        password: String,
        secure_cookie: bool,
        session_ttl: Duration,
    ) -> Self {
        let (events, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            store,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            bootstrap_secret: Arc::new(password),
            bootstrap_username: Arc::new(username),
            session_ttl,
            reauth_freshness: PURGE_REAUTH_FRESHNESS,
            secure_cookie,
            events: Arc::new(events),
            ai_provider: None,       // Phase 3 — set via with_ai_provider builder
            subject_validator: None, // Phase 1.5 — set via with_subject_validator
            engine: None,            // Console-expansion — set via with_engine builder
        }
    }

    /// Attach an AI provider for the Phase 3 AI semantic rules in `ai_review`.
    /// Mirrors `McpServer::with_ai_provider`. The same `Arc` should be shared
    /// between MCP and Console (single adapter, single compliance log).
    pub fn with_ai_provider(mut self, provider: Arc<dyn crate::provider::AiProvider>) -> Self {
        self.ai_provider = Some(provider);
        self
    }

    /// Attach a [`SubjectValidator`] (Phase 1.5) so `ai_review`'s
    /// deterministic pass can emit subject-quality tags (UnknownSubject,
    /// SubjectOnDenylist, etc.). When absent, subject-validation tags are
    /// skipped — legacy mode.
    pub fn with_subject_validator(
        mut self,
        validator: std::sync::Arc<crate::subject_validator::SubjectValidator>,
    ) -> Self {
        self.subject_validator = Some(validator);
        self
    }

    /// Attach a [`WikiEngine`] handle so engine-backed endpoints (`/status`,
    /// `/index-status`, `/index/update`, `/index/rebuild`, `/config`) can serve
    /// data. Production wires this via `server::serve`; tests that exercise
    /// engine-backed endpoints pass `Some(...)`; tests that don't leave it
    /// `None`.
    pub fn with_engine(mut self, engine: Arc<crate::engine::WikiEngine>) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Legacy alias for [`Self::with_credentials_and_ttl`] (single-credential
    /// signature). Preserved so existing test call sites keep compiling
    /// without a rewrite; new code should call `with_credentials_and_ttl`.
    #[doc(hidden)]
    pub fn with_session_ttl(
        store: Arc<SemanticStore>,
        bootstrap_secret: String,
        secure_cookie: bool,
        session_ttl: Duration,
    ) -> Self {
        Self::with_credentials_and_ttl(store, None, bootstrap_secret, secure_cookie, session_ttl)
    }

    /// Test seam: same as [`Self::with_credentials_and_ttl`] but also pins the
    /// destructive-action re-auth freshness window. Lets the contract tests
    /// force a session to be "stale" for hard-purge purposes without waiting
    /// [`PURGE_REAUTH_FRESHNESS`] in wall-clock time.
    #[doc(hidden)]
    pub fn with_session_ttl_and_reauth_freshness(
        store: Arc<SemanticStore>,
        bootstrap_secret: String,
        secure_cookie: bool,
        session_ttl: Duration,
        reauth_freshness: Duration,
    ) -> Self {
        let mut state = Self::with_credentials_and_ttl(
            store,
            None,
            bootstrap_secret,
            secure_cookie,
            session_ttl,
        );
        state.reauth_freshness = reauth_freshness;
        state
    }

    /// Test seam variant: full credentials + TTL + reauth-freshness pin.
    /// Used by `tests/api_trust_ops_v1.rs` to exercise the
    /// username+password flow with a controlled freshness window.
    #[doc(hidden)]
    pub fn with_credentials_ttl_and_reauth_freshness(
        store: Arc<SemanticStore>,
        username: Option<String>,
        password: String,
        secure_cookie: bool,
        session_ttl: Duration,
        reauth_freshness: Duration,
    ) -> Self {
        let mut state =
            Self::with_credentials_and_ttl(store, username, password, secure_cookie, session_ttl);
        state.reauth_freshness = reauth_freshness;
        state
    }

    /// Publisher handle for the `/events` broadcast. Anything in-process can
    /// clone this and `send()` a notification to all connected SSE clients.
    pub fn events_sender(&self) -> Arc<broadcast::Sender<String>> {
        Arc::clone(&self.events)
    }
}

/// Builds the Console API router. Caller nests it under `/api/v1`.
pub fn router(state: ConsoleApiState) -> Router {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/reauth", post(reauth))
        .route("/events", get(events))
        .route("/search", get(search))
        .route("/get", get(get_subject))
        .route("/entity/timeline", get(timeline))
        .route("/inbox", get(inbox))
        .route("/inbox/{proposal_id}/evidence", get(evidence))
        .route("/inbox/{proposal_id}/ai-review", get(ai_review))
        .route("/inbox/{proposal_id}/approve", post(approve))
        .route("/inbox/{proposal_id}/reject", post(reject))
        .route("/inbox/{proposal_id}/supersede", post(supersede))
        .route("/galaxy", get(galaxy))
        // E3.2 — trust + operations + entity-mutation + purge surfaces.
        .route("/trust", get(trust))
        .route("/ops/clients", get(ops_clients))
        .route("/ops/jobs", get(ops_jobs))
        .route("/ops/evals", get(ops_evals))
        .route("/ops/backup-health", get(ops_backup_health))
        .route("/destructive/warning", get(destructive_warning))
        .route("/entity/merge", post(entity_merge))
        .route("/entity/split", post(entity_split))
        .route("/claim/{claim_operation_id}/retract", post(entity_retract))
        .route("/purge/preview", post(purge_preview))
        .route("/purge/execute", post(purge_execute))
        .route("/purge/status", get(purge_status))
        .with_state(state)
}

// ── errors ──────────────────────────────────────────────────────────────────

/// A machine-readable API error. The `code` is a stable, non-sensitive token;
/// internal [`SemanticError`] `Display` strings are never forwarded verbatim
/// (they can carry object ids / DB detail), only mapped to a fixed code.
struct ApiError {
    status: StatusCode,
    code: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    /// 404 `not_found` — used by read-only Inbox routes (e.g.
    /// `ai-review`) when the requested id is not among the pending proposals.
    /// Deliberately indistinguishable from "never existed" so the route does
    /// not leak proposal existence (same posture as `map_semantic_error`'s
    /// `MissingDependency` / `ObjectUnavailable` arm).
    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found")
    }

    fn forbidden_csrf() -> Self {
        Self::new(StatusCode::FORBIDDEN, "csrf_failed")
    }

    fn invalid_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request")
    }

    /// 403 `reauth_required` — the destructive-action freshness gate. Hard
    /// purge (§5.3 "recent re-auth") requires a `/auth/reauth` within the last
    /// `PURGE_REAUTH_FRESHNESS` seconds; this fires when that window has lapsed
    /// (or the session has never re-authed). Distinct from `forbidden` so the
    /// UI can prompt specifically for re-auth rather than a generic denial.
    fn reauth_required() -> Self {
        Self::new(StatusCode::FORBIDDEN, "reauth_required")
    }

    /// 413 `payload_too_large` (Task F2.3). Reserved for a future Console API
    /// ingest route — the MCP ingest path surfaces the same condition via
    /// `WikiError::PayloadTooLarge`. Kept here so the error contract is
    /// symmetric across surfaces; the wire shape is the same `{"error":
    /// "<code>"}` JSON the rest of the Console API emits.
    #[allow(dead_code)]
    fn payload_too_large() -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
    }

    /// 429 `rate_limited` (Task F2.3). Reserved for a future Console API
    /// ingest route — the MCP ingest path surfaces the same condition via
    /// `WikiError::RateLimited`.
    #[allow(dead_code)]
    fn rate_limited() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.code }))).into_response()
    }
}

/// Maps a [`SemanticError`] to an HTTP status + stable code, without leaking
/// the internal message. Client-fixable inputs get 4xx; anything infrastructural
/// collapses to a generic 500 so storage/db detail never reaches the wire.
fn map_semantic_error(error: &SemanticError) -> ApiError {
    let (status, code) = match error {
        SemanticError::MissingDependency(_) | SemanticError::ObjectUnavailable(_) => {
            (StatusCode::NOT_FOUND, "not_found")
        }
        SemanticError::InvalidTransition(_) | SemanticError::IdempotencyConflict => {
            (StatusCode::CONFLICT, "conflict")
        }
        SemanticError::InvalidClaim(_)
        | SemanticError::InvalidCapture(_)
        | SemanticError::InvalidInterval => (StatusCode::BAD_REQUEST, "invalid_request"),
        SemanticError::UnsupportedInference => {
            (StatusCode::UNPROCESSABLE_ENTITY, "unsupported_inference")
        }
        SemanticError::CapabilityDenied(_) | SemanticError::Denied(_) => {
            (StatusCode::FORBIDDEN, "forbidden")
        }
        SemanticError::Disabled => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        // CorruptLedger / Io / Database / DatabaseContention / Serialization /
        // registry internals / marker & handle states: infrastructural — never
        // surface detail.
        other => {
            tracing::error!(error = %other, "console api: internal semantic error");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
    };
    ApiError::new(status, code)
}

// ── auth primitives ─────────────────────────────────────────────────────────

/// Constant-time byte comparison. Length inequality short-circuits (a coarse,
/// universally-accepted leak); equal-length inputs are compared with a
/// branch-free XOR accumulate so no timing signal reveals how many bytes
/// matched. Hand-rolled on purpose — no `subtle` dependency for ~10 lines.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extracts a named cookie value from a raw `Cookie:` header.
fn cookie_value<'a>(header_value: &'a str, name: &str) -> Option<&'a str> {
    header_value.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn session_cookie_header(session_id: &str, secure: bool) -> String {
    // Path is scoped to /api/v1 so the cookie never rides on unrelated routes.
    let mut cookie = format!(
        "{SESSION_COOKIE}={session_id}; HttpOnly; SameSite=Strict; Path=/api/v1; Max-Age=86400"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn expired_cookie_header(secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/api/v1; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// The result of a successful session lookup, used by both extractors. The
/// `freshness_anchor` is the timestamp the destructive-action gate compares
/// against `PURGE_REAUTH_FRESHNESS`: `reauthenticated_at` if the session has
/// re-authed since login, else `issued_at`.
struct ResolvedSession {
    session_id: String,
    context: TrustedContext,
    csrf_token: String,
    issued_at: DateTime<Utc>,
    reauthenticated_at: Option<DateTime<Utc>>,
}

impl ResolvedSession {
    /// The anchor for the destructive-action freshness gate.
    fn freshness_anchor(&self) -> DateTime<Utc> {
        self.reauthenticated_at.unwrap_or(self.issued_at)
    }
}

/// Looks up and validates the session cookie. `401` on missing / unknown /
/// expired. Takes only a read lock; never awaits while holding it.
fn resolve_session(
    headers: &HeaderMap,
    state: &ConsoleApiState,
) -> Result<ResolvedSession, ApiError> {
    let session_id = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| cookie_value(raw, SESSION_COOKIE))
        .ok_or_else(ApiError::unauthorized)?
        .to_owned();

    let sessions = state.sessions.read();
    let session = sessions
        .get(&session_id)
        .ok_or_else(ApiError::unauthorized)?;
    if session.expires_at <= Utc::now() {
        return Err(ApiError::unauthorized());
    }
    Ok(ResolvedSession {
        session_id,
        context: session.context.clone(),
        csrf_token: session.csrf_token.clone(),
        issued_at: session.issued_at,
        reauthenticated_at: session.reauthenticated_at,
    })
}

/// Extractor for routes that require a valid session but no CSRF (all GETs and
/// logout). GET reads are store-scoped, not client-scoped, so they only need
/// proof of a valid session — not the session's `TrustedContext`. Logout needs
/// the `session_id` to evict the entry.
struct AuthSession {
    session_id: String,
}

impl FromRequestParts<ConsoleApiState> for AuthSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ConsoleApiState,
    ) -> Result<Self, Self::Rejection> {
        let resolved = resolve_session(&parts.headers, state)?;
        Ok(AuthSession {
            session_id: resolved.session_id,
        })
    }
}

/// Extractor for mutating routes: valid session (`401`) AND a matching
/// `X-CSRF-Token` header (`403`). Session is checked first so no-session always
/// wins as 401 even on a mutating route. Carries the `freshness_anchor` so the
/// destructive-action handler can enforce the re-auth window without re-walking
/// the session table.
struct CsrfSession {
    context: TrustedContext,
    freshness_anchor: DateTime<Utc>,
}

impl CsrfSession {
    /// Destructive-action freshness gate (§5.3 "recent re-auth"). Returns
    /// `reauth_required` (403) if the session's freshness anchor is older than
    /// the state's configured re-auth window. Callers are mutating handlers
    /// that the irreversibility warning explicitly flags as
    /// `requires_recent_reauth` (today: hard purge only).
    fn require_purge_freshness(&self, state: &ConsoleApiState) -> Result<(), ApiError> {
        if Utc::now().signed_duration_since(self.freshness_anchor) > state.reauth_freshness {
            return Err(ApiError::reauth_required());
        }
        Ok(())
    }
}

impl FromRequestParts<ConsoleApiState> for CsrfSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ConsoleApiState,
    ) -> Result<Self, Self::Rejection> {
        let resolved = resolve_session(&parts.headers, state)?;
        let provided = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(ApiError::forbidden_csrf)?;
        if !constant_time_eq(provided.as_bytes(), resolved.csrf_token.as_bytes()) {
            return Err(ApiError::forbidden_csrf());
        }
        let freshness_anchor = resolved.freshness_anchor();
        Ok(CsrfSession {
            context: resolved.context,
            freshness_anchor,
        })
    }
}

// ── auth routes ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    /// Legacy single-credential field. Ignored when the server is configured
    /// with a username (username+password mode); compared against the
    /// configured password otherwise. Accepting both shapes on the same
    /// route keeps old CLI clients working through the migration.
    #[serde(default)]
    secret: Option<String>,
    /// Username+password mode (Phase G, 2026-07-20). Required when the server
    /// has a configured username; ignored otherwise.
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// Verifies the login body against the configured credentials.
///
/// Two modes, dispatched on [`ConsoleApiState::bootstrap_username`]:
/// - **username+password** (`Some(username)`): both `body.username` AND
///   `body.password` must be present and match (constant-time). `body.secret`
///   is ignored.
/// - **legacy single-credential** (`None`): `body.secret` must match the
///   configured password (constant-time). `body.username`/`body.password` are
///   ignored. This preserves backward compatibility with deployments that
///   only set `console_dev_bootstrap_secret` and with old CLI clients.
///
/// Returns `true` on a match, `false` otherwise. The caller is responsible
/// for incrementing `console_auth_failures_total` on `false`.
fn verify_login(state: &ConsoleApiState, body: &LoginRequest) -> bool {
    if let Some(configured_username) = state.bootstrap_username.as_ref() {
        // Username+password mode.
        matches!(
            (body.username.as_deref(), body.password.as_deref()),
            (Some(u), Some(p))
                if constant_time_eq(u.as_bytes(), configured_username.as_bytes())
                    && constant_time_eq(p.as_bytes(), state.bootstrap_secret.as_bytes())
        )
    } else {
        // Legacy single-credential mode.
        match body.secret.as_deref() {
            Some(s) => constant_time_eq(s.as_bytes(), state.bootstrap_secret.as_bytes()),
            None => false,
        }
    }
}

async fn login(State(state): State<ConsoleApiState>, body: Option<Json<LoginRequest>>) -> Response {
    let Some(Json(body)) = body else {
        return ApiError::new(StatusCode::BAD_REQUEST, "invalid_request").into_response();
    };
    if !verify_login(&state, &body) {
        // Task F2.2: failed Console login counter. Side-effect only; the 401
        // response contract is unchanged. The metrics facade is a no-op when
        // no recorder is installed.
        metrics::counter!("console_auth_failures_total").increment(1);
        return ApiError::unauthorized().into_response();
    }

    let context = match state.store.register_client("console") {
        Ok(context) => context,
        Err(error) => return map_semantic_error(&error).into_response(),
    };
    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + state.session_ttl;
    state.sessions.write().insert(
        session_id.clone(),
        Session {
            context,
            csrf_token: csrf_token.clone(),
            expires_at,
            issued_at: now,
            reauthenticated_at: None,
        },
    );

    let cookie = session_cookie_header(&session_id, state.secure_cookie);
    let Ok(cookie_value) = HeaderValue::from_str(&cookie) else {
        return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error").into_response();
    };
    // Task F2.2: successful Console login counter. Incremented AFTER the
    // session is minted and immediately before the 200 leaves — a crash between
    // here and `return` is tolerable (we'd rather under-count than over-count).
    metrics::counter!("console_logins_total").increment(1);
    let mut response = Json(json!({ "csrf_token": csrf_token })).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie_value);
    response
}

async fn logout(State(state): State<ConsoleApiState>, session: AuthSession) -> Response {
    state.sessions.write().remove(&session.session_id);
    let cookie = expired_cookie_header(state.secure_cookie);
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(cookie_value) = HeaderValue::from_str(&cookie) {
        response
            .headers_mut()
            .insert(header::SET_COOKIE, cookie_value);
    }
    response
}

// ── read routes ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    domain: Option<String>,
    top_k: Option<usize>,
}

async fn search(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<SearchParams>,
) -> Result<Response, ApiError> {
    // FTS5-ranked search (shares the read path with the brain_search MCP tool).
    // `search_claims` returns the ranked claim_ids + scores; we then hydrate
    // the full Console display fields (origin/provenance/entity_id) from
    // `all_claims_current`, ordered by the FTS5 ranking. This keeps the REST
    // and MCP surfaces on one search engine instead of the old divergent
    // linear scans, and adds `score` to the payload.
    let query = &params.query;
    let top_k = params.top_k.unwrap_or(10);
    let hits = state
        .store
        .search_claims(query, params.domain.as_deref(), top_k)
        .map_err(|e| map_semantic_error(&e))?;
    // Build a claim_id → score map for reordering after hydration.
    let score_by_id: std::collections::HashMap<Uuid, f64> = hits
        .iter()
        .map(|h| (h.claim_id, h.score))
        .collect();

    let head = state
        .store
        .ledger_head()
        .map_err(|e| map_semantic_error(&e))?;
    let claims = state
        .store
        .all_claims_current(head, Utc::now())
        .map_err(|e| map_semantic_error(&e))?;

    // Keep only the FTS-hit claim_ids, hydrate full fields, preserve BM25 order.
    let mut results: Vec<_> = claims
        .active
        .iter()
        .filter(|claim| score_by_id.contains_key(&claim.claim_id))
        .map(|claim| {
            json!({
                "claim_id": claim.claim_id,
                "subject": claim.subject,
                "predicate": claim.predicate,
                "value": claim.value,
                "domain": claim.domain,
                "origin": claim.origin,
                "provenance": claim.provenance_kind,
                "entity_id": claim.entity_id,
                "score": score_by_id.get(&claim.claim_id),
            })
        })
        .collect();
    results.sort_by(|a, b| {
        let sa = a["score"].as_f64().unwrap_or(f64::INFINITY);
        let sb = b["score"].as_f64().unwrap_or(f64::INFINITY);
        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Json(json!({
        "query": params.query,
        "count": results.len(),
        "results": results,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct GetParams {
    subject: String,
    domain: Option<String>,
}

async fn get_subject(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<GetParams>,
) -> Result<Response, ApiError> {
    let head = state
        .store
        .ledger_head()
        .map_err(|e| map_semantic_error(&e))?;
    let claims = state
        .store
        .all_claims_current(head, Utc::now())
        .map_err(|e| map_semantic_error(&e))?;

    let results: Vec<_> = claims
        .active
        .iter()
        .filter(|claim| {
            if let Some(domain) = &params.domain
                && claim.domain != *domain
            {
                return false;
            }
            claim.subject == params.subject
        })
        .map(|claim| {
            json!({
                "claim_id": claim.claim_id,
                "subject": claim.subject,
                "predicate": claim.predicate,
                "value": claim.value,
                "domain": claim.domain,
                "kind": claim.claim_kind,
                "origin": claim.origin,
                "provenance": claim.provenance_kind,
                "entity_id": claim.entity_id,
                "confidence": f64::from(claim.confidence_basis_points) / 10_000.0,
            })
        })
        .collect();

    Ok(Json(json!({
        "subject": params.subject,
        "count": results.len(),
        "claims": results,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct TimelineParams {
    /// Entity Identity Reform: domain is an optional filter. `None` returns
    /// every claim for the given subject/predicate across all domains.
    #[serde(default)]
    domain: Option<String>,
    subject: String,
    predicate: String,
}

async fn timeline(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<TimelineParams>,
) -> Result<Response, ApiError> {
    let timeline = state
        .store
        .claim_timeline(params.domain.as_deref(), &params.subject, &params.predicate)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(timeline).into_response())
}

async fn inbox(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Result<Json<Vec<InboxProposal>>, ApiError> {
    use chrono::Utc;
    let pending = state
        .store
        .list_pending_proposals()
        .map_err(|e| map_semantic_error(&e))?;

    // Phase 1.6 — load confirmed claims so conflict detection covers both
    // peer proposals (pending) and existing claims (confirmed). `all_claims_current`
    // buckets all confirmed claims into active/future/past as of the ledger
    // head; we flatten all three so a pending proposal that re-states a past
    // (superseded/retracted) value still flags as a duplicate (useful review
    // signal even though it is not strictly "in conflict"). When the call
    // fails or yields nothing, `confirmed` stays empty and detection degrades
    // gracefully to pending-only (per spec).
    let mut confirmed: Vec<crate::semantic::ClaimView> = Vec::new();
    if let Ok(head) = state.store.ledger_head()
        && let Ok(current) = state.store.all_claims_current(head, Utc::now())
    {
        confirmed.extend(current.active);
        confirmed.extend(current.future);
        confirmed.extend(current.past);
    }

    let conflicts = crate::inbox_conflicts::detect_conflicts(&pending, &confirmed);
    let out: Vec<InboxProposal> = pending
        .into_iter()
        .map(|p| {
            let proposal_id = p.proposal_id;
            let c = conflicts.get(&proposal_id).cloned().unwrap_or_default();
            InboxProposal {
                proposal: p,
                conflicts: c,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn evidence(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Path(proposal_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let evidence = state
        .store
        .evidence_for(proposal_id)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(evidence).into_response())
}

/// `GET /inbox/{proposal_id}/ai-review` — quality tags for one proposal
/// (AI Pre-Review Phase 2 + Phase 3).
///
/// Session required (no CSRF — read-only, mirrors `evidence`). Loads the
/// proposal + evidence + current claims from public `SemanticStore` reads,
/// runs `QualityChecker::check_deterministic`, then — when an AI provider is
/// attached to the state — runs `AiQualityChecker::check` and merges its
/// tags in. `ai_used` is `true` only when the AI actually ran to completion
/// (provider attached + egress allowed + parseable response); denial,
/// provider error, and unparseable response all degrade silently to
/// deterministic-only (`ai_used=false`).
///
/// **Never mutates the ledger/event store** (ADR-0001 §Decision 1: human
/// stays the approver; AI only tags).
async fn ai_review(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Path(proposal_id): Path<Uuid>,
) -> Result<Json<crate::quality::AiReviewResponse>, ApiError> {
    use chrono::Utc;

    // Find the pending proposal by id. 404 if not pending (either already
    // reviewed or never existed — both look the same to the caller, by
    // design, to avoid leaking proposal existence).
    let pending = state
        .store
        .list_pending_proposals()
        .map_err(|e| map_semantic_error(&e))?;
    let proposal = pending
        .iter()
        .find(|p| p.proposal_id == proposal_id)
        .ok_or_else(ApiError::not_found)?;

    // Read evidence + current claims (all public read-only methods).
    let evidence = state
        .store
        .evidence_for(proposal_id)
        .map_err(|e| map_semantic_error(&e))?;
    let head = state
        .store
        .ledger_head()
        .map_err(|e| map_semantic_error(&e))?;
    let current = state
        .store
        .all_claims_current(head, Utc::now())
        .map_err(|e| map_semantic_error(&e))?;

    // Flatten active + future + past into the existing-claims slice. Past
    // (superseded/retracted) claims are still relevant context for
    // DuplicatePredicate — they show the historical scope.
    let mut existing: Vec<crate::semantic::ClaimView> =
        Vec::with_capacity(current.active.len() + current.future.len() + current.past.len());
    existing.extend(current.active.iter().cloned());
    existing.extend(current.future.iter().cloned());
    existing.extend(current.past.iter().cloned());

    let input = crate::quality::QualityCheckerInput {
        proposal,
        evidence: &evidence,
        existing_claims: &existing,
    };

    // Deterministic rules — always run. Build a checker bound to the
    // configured Subject Validator (Phase 1.5) when present so subject
    // quality tags fire; fall back to subject-validation-less legacy mode.
    let checker = match &state.subject_validator {
        Some(v) => crate::quality::QualityChecker::new(v.clone()),
        None => crate::quality::QualityChecker::without_subject_validation(),
    };
    let mut tags = checker.check_deterministic(&input);

    // Phase 3: AI semantic rules — only when a provider is attached. On
    // egress denial / provider error / unparseable response, `check` returns
    // `(empty, false)` — graceful degradation to deterministic-only, no
    // error surfaced to the client (matches brain_extract posture).
    let mut ai_used = false;
    if let Some(provider) = state.ai_provider.clone() {
        let ai_checker = crate::quality::AiQualityChecker::new(provider);
        let (ai_tags, ran) = ai_checker.check(&input).await;
        tags.extend(ai_tags);
        ai_used = ran;
    }

    Ok(Json(crate::quality::AiReviewResponse {
        proposal_id,
        tags,
        checked_at: Utc::now(),
        checker_version: crate::quality::QUALITY_CHECKER_VERSION.to_owned(),
        ai_used,
    }))
}

// ── galaxy (Task E2.1) ──────────────────────────────────────────────────────

/// Query params for `GET /api/v1/galaxy`. `zoom` picks the LOD cap ("far" →
/// ≤300 community supernodes, "mid" → ≤2000 visible nodes, "close" → ego
/// neighborhood, requires `focus`). `domain` filters to one domain. `focus`
/// is the entity_id for ego mode.
#[derive(Deserialize)]
struct GalaxyQuery {
    domain: Option<String>,
    zoom: Option<String>,
    focus: Option<Uuid>,
}

/// Parse the `zoom` query param into a [`ZoomLevel`]. Unknown values are a
/// 400 `invalid_request`. Missing defaults to `Far` (the cheapest LOD).
fn parse_zoom(raw: &Option<String>) -> Result<ZoomLevel, ApiError> {
    match raw.as_deref() {
        None | Some("") | Some("far") => Ok(ZoomLevel::Far),
        Some("mid") => Ok(ZoomLevel::Mid),
        Some("close") => Ok(ZoomLevel::Close),
        Some(_) => Err(ApiError::invalid_request()),
    }
}

/// `GET /api/v1/galaxy` — bounded Galaxy subgraph for the Console (Task E2.1).
///
/// Session required (no CSRF — read-only). The server materializes a bounded
/// subgraph from the current claim snapshot and returns a [`GalaxyPayload`]:
///
/// - `zoom=far` (default): community supernodes, ≤300 nodes (first-seen).
/// - `zoom=mid`: visible nodes, ≤2000.
/// - `zoom=close&focus=<entity_id>`: ego neighborhood around `focus`, depth 2.
/// - `zoom=close` without `focus`: 400 `invalid_request` (ego needs a focus).
///
/// `domain` filters to one domain in all modes. §9.2: server never sends the
/// whole brain — the LOD cap (and the ego depth bound) is the safety bound.
async fn galaxy(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<GalaxyQuery>,
) -> Result<Response, ApiError> {
    let zoom = parse_zoom(&params.zoom)?;
    let lod = zoom.lod();

    // ego mode requires a focus entity.
    if matches!(zoom, ZoomLevel::Close) && params.focus.is_none() {
        return Err(ApiError::invalid_request());
    }

    let head = state
        .store
        .ledger_head()
        .map_err(|e| map_semantic_error(&e))?;
    let claims = state
        .store
        .all_claims_current(head, Utc::now())
        .map_err(|e| map_semantic_error(&e))?;

    // Combine active + future + past into one flat slice the materializer
    // consumes. Past claims still carry their entity_id (useful for
    // provenance / supersede context); the active set is the dominant input.
    // TODO(phase-e2.2/future): past (superseded/retracted/expired) claims currently
    // contribute to node materialization + label voting. For a strictly "current brain"
    // view, filter to `claims.active` only, or have the materializer down-weight past
    // claims. Kept for now to preserve provenance/supersede context per §9.2.
    let mut flat: Vec<&crate::semantic::ClaimView> =
        Vec::with_capacity(claims.active.len() + claims.future.len() + claims.past.len());
    flat.extend(claims.active.iter());
    flat.extend(claims.future.iter());
    flat.extend(claims.past.iter());

    let canonical = state.store.entity_canonical_subjects_owned();

    let graph = match (zoom, params.focus) {
        (ZoomLevel::Close, Some(focus_id)) => {
            // Ego mode: take the focused slice, then materialize with the
            // ego builder. We pass the full flat slice so the BFS can walk
            // beyond the focus's own claims to find neighbors.
            let owned: Vec<crate::semantic::ClaimView> = flat.into_iter().cloned().collect();
            let mut ego = GalaxyGraph::ego_around(&owned, focus_id, crate::galaxy::EGO_MAX_DEPTH);
            // Re-stamp node labels with canonical subjects now that we know
            // the surviving entity set (ego_around doesn't have the map).
            for node in ego.nodes_mut() {
                if let Ok(id) = Uuid::parse_str(&node.id)
                    && let Some(label) = canonical.get(&id)
                {
                    node.label = label.clone();
                }
            }
            ego
        }
        _ => {
            // Bounded mode (Far / Mid). Pass the canonical map so labels are
            // the live canonical subject rather than the per-claim payload.
            let domain_filter = params.domain.as_deref();
            GalaxyGraph::from_claims_with_subjects(
                flat.into_iter().cloned().collect::<Vec<_>>().as_slice(),
                lod.node_cap(),
                domain_filter,
                &canonical,
            )
        }
    };

    Ok(Json(graph.to_payload(lod)).into_response())
}

// ── trust + operations + destructive-warning (Task E3.2) ────────────────────

/// Query params for `GET /api/v1/trust`. `staleness_threshold_days` defaults to
/// 90 (§5.3 ballpark "stale" horizon); `retrieval_query` optionally populates
/// the `retrieval_trace` field so the UI can show why a retrieval returned what
/// it did.
#[derive(Deserialize)]
struct TrustParams {
    staleness_threshold_days: Option<u32>,
    retrieval_query: Option<String>,
}

/// `GET /api/v1/trust` — the trust surface for the Console (Task E3.2).
///
/// Returns the live `contradictions` + `stale` flag sets plus an optional
/// `retrieval_trace`. Session required (no CSRF — read-only). The two scanners
/// run against the current ledger head + `Utc::now()`; the optional
/// `retrieval_query` populates `retrieval_trace` so the same call can answer
/// "what's contested AND what would a search for X return/exclude".
async fn trust(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<TrustParams>,
) -> Result<Response, ApiError> {
    let threshold = params.staleness_threshold_days.unwrap_or(90);
    let now = Utc::now();
    let head = state
        .store
        .ledger_head()
        .map_err(|e| map_semantic_error(&e))?;
    let contradictions = state
        .store
        .contradictions(head, now)
        .map_err(|e| map_semantic_error(&e))?;
    let stale = state
        .store
        .staleness(head, threshold, now)
        .map_err(|e| map_semantic_error(&e))?;
    let retrieval_trace = match &params.retrieval_query {
        Some(query) if !query.is_empty() => {
            let trace = state
                .store
                .retrieval_trace(query, 10, head, now)
                .map_err(|e| map_semantic_error(&e))?;
            Some(trace)
        }
        _ => None,
    };
    Ok(Json(json!({
        "contradictions": contradictions,
        "stale": stale,
        "retrieval_trace": retrieval_trace,
    }))
    .into_response())
}

/// `GET /api/v1/ops/clients` — registered-client activity audit (§9.1 ops
/// item 7). Bare `Vec<ClientActivity>`. Session required; no CSRF.
async fn ops_clients(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Result<Response, ApiError> {
    let clients = state
        .store
        .list_clients()
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(clients).into_response())
}

/// `GET /api/v1/ops/jobs` — async-job queue summary for the operations
/// dashboard. Session required; no CSRF.
async fn ops_jobs(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Result<Response, ApiError> {
    let summary = state
        .store
        .job_summary()
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(summary).into_response())
}

/// Query params for `GET /api/v1/ops/evals`. `domain` is required — the eval
/// summary is per-domain (different domains have different eval suites).
#[derive(Deserialize)]
struct EvalsParams {
    domain: Option<String>,
}

/// `GET /api/v1/ops/evals?domain=X` — last domain-eval run summary. `domain`
/// is required; missing → 400 `invalid_request`. Session required; no CSRF.
async fn ops_evals(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<EvalsParams>,
) -> Result<Response, ApiError> {
    let Some(domain) = params.domain.as_deref() else {
        return Err(ApiError::invalid_request());
    };
    if domain.is_empty() {
        return Err(ApiError::invalid_request());
    }
    let summary = state
        .store
        .eval_summary(domain)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(summary).into_response())
}

/// `GET /api/v1/ops/backup-health` — backup/restore-drill health. Session
/// required; no CSRF.
async fn ops_backup_health(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Result<Response, ApiError> {
    let health = state
        .store
        .backup_health()
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(health).into_response())
}

/// Query params for `GET /api/v1/destructive/warning?action=...`. `action` is
/// required and must be one of `hard_purge` / `entity_merge` / `entity_split`.
#[derive(Deserialize)]
struct DestructiveWarningParams {
    action: Option<String>,
}

/// Parse the `action` query param into a [`DestructiveAction`]. Unknown or
/// missing values are a 400 `invalid_request`.
fn parse_destructive_action(raw: &Option<String>) -> Result<DestructiveAction, ApiError> {
    match raw.as_deref() {
        Some("hard_purge") => Ok(DestructiveAction::HardPurge),
        Some("entity_merge") => Ok(DestructiveAction::EntityMerge),
        Some("entity_split") => Ok(DestructiveAction::EntitySplit),
        _ => Err(ApiError::invalid_request()),
    }
}

/// `GET /api/v1/destructive/warning?action=hard_purge|entity_merge|entity_split`
/// (Task E3.2 DoD #2). Returns the [`DestructiveWarning`] the UI MUST display
/// before confirming the action. The warning's `message` states "no undo" /
/// "cannot be recovered" for hard purge; `requires_recent_reauth` +
/// `requires_two_step_nonce` flag the gates the client must satisfy first.
/// Session required; no CSRF — it is a read.
async fn destructive_warning(
    _state: State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<DestructiveWarningParams>,
) -> Result<Response, ApiError> {
    let action = parse_destructive_action(&params.action)?;
    Ok(Json(DestructiveWarning::for_action(action)).into_response())
}

// ── entity mutations (Task E3.2) ────────────────────────────────────────────

/// `POST /api/v1/entity/merge` body. `operation_id` is optional — if absent the
/// server mints a fresh UUIDv4, so a one-shot client doesn't have to.
#[derive(Deserialize)]
struct EntityMergeRequest {
    source: Uuid,
    target: Uuid,
    operation_id: Option<String>,
}

/// `POST /api/v1/entity/merge` — merge source entity into target (Task 2.2).
/// Every claim on the source is rewritten onto the target; the source's
/// subject becomes a backlink. CSRF + session required. Returns
/// `{status:"merged", event_seq}`.
async fn entity_merge(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Json(body): Json<EntityMergeRequest>,
) -> Result<Response, ApiError> {
    let operation_id = body
        .operation_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let outcome = state
        .store
        .merge_entities(
            &session.context,
            MergeEntitiesCommand {
                operation_id,
                source_entity_id: body.source,
                target_entity_id: body.target,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "merge").increment(1);
    Ok(Json(json!({
        "status": "merged",
        "event_seq": outcome.event.event_seq,
    }))
    .into_response())
}

/// One predicate → target assignment in the `POST /api/v1/entity/split` body.
#[derive(Deserialize)]
struct SplitAssignmentBody {
    predicate: String,
    target_entity_id: Uuid,
}

/// `POST /api/v1/entity/split` body. `operation_id` is optional (server-mints
/// a UUIDv4 if absent).
#[derive(Deserialize)]
struct EntitySplitRequest {
    source: Uuid,
    assignments: Vec<SplitAssignmentBody>,
    operation_id: Option<String>,
}

/// `POST /api/v1/entity/split` — split source entity by predicate (Task E3.1).
/// Each claim on the source whose predicate matches an assignment is rewritten
/// onto that assignment's target; predicates not listed stay on the source.
/// CSRF + session required. Returns `{status:"split", event_seq,
/// moved_claim_count, source_remaining_claim_count}` — the two counts come
/// from the structured [`SplitOutcome`] re-derived from the emitted event.
async fn entity_split(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Json(body): Json<EntitySplitRequest>,
) -> Result<Response, ApiError> {
    let operation_id = body
        .operation_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_entity_id = body.source;
    let assignments: Vec<PredicateAssignment> = body
        .assignments
        .into_iter()
        .map(|a| PredicateAssignment {
            predicate: a.predicate,
            target_entity_id: a.target_entity_id,
        })
        .collect();
    let outcome = state
        .store
        .split_entities(
            &session.context,
            SplitCommand {
                operation_id,
                source_entity_id,
                assignments,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    // The structured view (counts + moved claims) is reconstructed from the
    // emitted `entity_split` event via the same helper the contract tests use.
    let view: SplitOutcome = state
        .store
        .last_split_outcome_for(&session.context, source_entity_id)
        .map_err(|e| map_semantic_error(&e))?
        .ok_or_else(|| {
            // Should be unreachable: split_entities just emitted the event.
            tracing::error!(
                source_entity_id = %source_entity_id,
                "split_entities emitted no event the outcome helper could find"
            );
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        })?;
    metrics::counter!("console_mutations_total", "action" => "split").increment(1);
    Ok(Json(json!({
        "status": "split",
        "event_seq": outcome.event.event_seq,
        "moved_claim_count": view.moved_claims.len(),
        "source_remaining_claim_count": view.source_remaining_claim_count,
    }))
    .into_response())
}

/// `POST /api/v1/claim/{claim_operation_id}/retract` body. `operation_id` is
/// optional. The path's `claim_operation_id` is the proposer's `operation_id`
/// of the confirm that minted the claim — the [`RetractCommand`] resolves the
/// claim from it, so the API speaks the same operation-id currency the rest
/// of the ledger uses (NOT the raw claim UUID, which the client rarely has).
#[derive(Deserialize)]
struct RetractRequest {
    operation_id: Option<String>,
}

/// `POST /api/v1/claim/{claim_operation_id}/retract` — retract a confirmed
/// claim. History and evidence are kept; the claim stops being current. CSRF +
/// session required. The `entity_id` route the spec suggested is wrong: retract
/// is per-claim, so this route is keyed on the claim's confirm operation_id.
async fn entity_retract(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Path(claim_operation_id): Path<String>,
    body: Option<Json<RetractRequest>>,
) -> Result<Response, ApiError> {
    let operation_id = body
        .and_then(|Json(b)| b.operation_id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let outcome = state
        .store
        .retract(
            &session.context,
            RetractCommand {
                operation_id,
                claim_operation_id,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "retract").increment(1);
    Ok(Json(json!({
        "status": "retracted",
        "event_seq": outcome.event.event_seq,
    }))
    .into_response())
}

// ── hard purge (Task E3.2 — preview / execute / status) ─────────────────────

/// `POST /api/v1/purge/preview` body. `object_ids` are the content keys the
/// caller wants irreversibly purged.
#[derive(Deserialize)]
struct PurgePreviewRequest {
    object_ids: Vec<String>,
}

/// `POST /api/v1/purge/preview` — phase 1 of the hard-purge flow. Returns the
/// [`PurgePreview`] (preview_hash + nonce + targets + expiry) PLUS the
/// `warning` string the UI MUST display before the execute step (Task E3.2
/// DoD #2). CSRF + session required. The nonce + preview_hash must be echoed
/// back to `/purge/execute`.
async fn purge_preview(
    State(state): State<ConsoleApiState>,
    _session: CsrfSession,
    Json(body): Json<PurgePreviewRequest>,
) -> Result<Response, ApiError> {
    let preview = state
        .store
        .purge_preview(&body.object_ids)
        .map_err(|e| map_semantic_error(&e))?;
    let warning = DestructiveWarning::for_action(DestructiveAction::HardPurge);
    metrics::counter!("console_mutations_total", "action" => "purge_preview").increment(1);
    Ok(Json(json!({
        "preview": preview,
        "warning": warning,
    }))
    .into_response())
}

/// `POST /api/v1/purge/execute` body. `preview_hash` + `nonce` come from a
/// prior `/purge/preview`; `operation_id` is optional (server-mints if absent).
#[derive(Deserialize)]
struct PurgeExecuteRequest {
    preview_hash: String,
    nonce: String,
    operation_id: Option<String>,
}

/// `POST /api/v1/purge/execute` — phase 2 of the hard-purge flow. Three gates
/// all must pass:
///
/// 1. CSRF + valid session (the [`CsrfSession`] extractor).
/// 2. Recent re-auth — the session's freshness anchor must be within
///    [`PURGE_REAUTH_FRESHNESS`] (§5.3 "recent re-auth"). Else 403
///    `reauth_required`, which the UI surfaces as "re-enter secret".
/// 3. The semantic gate — correct `preview_hash` for the `nonce`, nonce
///    unused + unexpired, `purge` capability held (checked inside
///    [`SemanticStore::purge_execute`]).
///
/// Returns the `PurgeReceipt` (saga state, eventually `completed` with a
/// composite_checksum).
async fn purge_execute(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Json(body): Json<PurgeExecuteRequest>,
) -> Result<Response, ApiError> {
    // Gate 2: recent re-auth. Hard purge is the one action whose warning has
    // `requires_recent_reauth = true`; the check lives here rather than in the
    // store so the auth-freshness policy stays in the API layer.
    session.require_purge_freshness(&state)?;
    let operation_id = body
        .operation_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let receipt = state
        .store
        .purge_execute(
            &session.context,
            &operation_id,
            &body.preview_hash,
            &body.nonce,
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "purge_execute").increment(1);
    Ok(Json(receipt).into_response())
}

/// Query params for `GET /api/v1/purge/status?purge_id=...`.
#[derive(Deserialize)]
struct PurgeStatusParams {
    purge_id: Uuid,
}

/// `GET /api/v1/purge/status?purge_id=<uuid>` — current state of a hard-purge
/// saga without advancing it. Read-only, so session-only (no CSRF). Returns the
/// `PurgeReceipt`.
async fn purge_status(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
    Query(params): Query<PurgeStatusParams>,
) -> Result<Response, ApiError> {
    let receipt = state
        .store
        .purge_status(params.purge_id)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(receipt).into_response())
}

// ── auth: re-authentication (Task E3.2) ─────────────────────────────────────

/// `POST /api/v1/auth/reauth` body — accepts BOTH credential shapes so the
/// Svelte client and any CLI clients can reuse the same form. Mirrors
/// [`LoginRequest`] field-for-field.
#[derive(Deserialize)]
struct ReauthRequest {
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// `POST /api/v1/auth/reauth` — re-validate credentials against an
/// EXISTING session and bump its freshness anchor (§5.3 "recent re-auth"). The
/// destructive-action gate (`/purge/execute`) compares
/// `reauthenticated_at.unwrap_or(issued_at)` against
/// [`PURGE_REAUTH_FRESHNESS`]; without a re-auth after login, hard purge is
/// blocked. Requires a valid session (AuthSession); wrong credentials → 401.
/// Returns `{reauthenticated:true, fresh_for_seconds:300}`.
///
/// In username+password mode both fields must match; in legacy mode the
/// `secret` field is checked against the configured password.
async fn reauth(
    State(state): State<ConsoleApiState>,
    session: AuthSession,
    body: Option<Json<ReauthRequest>>,
) -> Response {
    let Some(Json(body)) = body else {
        return ApiError::invalid_request().into_response();
    };
    // Reuse LoginRequest's verifier: same field names + same dual-mode logic.
    let login_body = LoginRequest {
        secret: body.secret,
        username: body.username,
        password: body.password,
    };
    if !verify_login(&state, &login_body) {
        // Task F2.2: re-auth counts as an auth failure (same threat surface
        // as login — wrong credentials submitted to a Console route).
        metrics::counter!("console_auth_failures_total").increment(1);
        return ApiError::unauthorized().into_response();
    }
    let now = Utc::now();
    let mut sessions = state.sessions.write();
    let Some(entry) = sessions.get_mut(&session.session_id) else {
        // Evicted between extractor and handler — treat as unauthenticated.
        return ApiError::unauthorized().into_response();
    };
    entry.reauthenticated_at = Some(now);
    let fresh_for_seconds = state.reauth_freshness.num_seconds();
    drop(sessions);
    // Task F2.2: a successful re-auth is a fresh login-equivalent for
    // audit purposes — count it under the same counter as primary logins.
    metrics::counter!("console_logins_total").increment(1);
    Json(json!({
        "reauthenticated": true,
        "fresh_for_seconds": fresh_for_seconds,
    }))
    .into_response()
}

// ── review routes ───────────────────────────────────────────────────────────

async fn approve(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Path(proposal_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let outcome = state
        .store
        .confirm_by_proposal_id(
            &session.context,
            ConfirmByProposalIdCommand {
                operation_id: Uuid::new_v4().to_string(),
                proposal_id,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "approve").increment(1);
    Ok(Json(json!({
        "status": "confirmed",
        "claim_id": outcome.generated.claim_id,
        "event_seq": outcome.event.event_seq,
    }))
    .into_response())
}

async fn reject(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Path(proposal_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let outcome = state
        .store
        .reject_by_proposal_id(
            &session.context,
            RejectByProposalIdCommand {
                operation_id: Uuid::new_v4().to_string(),
                proposal_id,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "reject").increment(1);
    Ok(Json(json!({
        "status": "rejected",
        "event_seq": outcome.event.event_seq,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct SupersedeRequest {
    superseded_claim_ids: Vec<Uuid>,
}

async fn supersede(
    State(state): State<ConsoleApiState>,
    session: CsrfSession,
    Path(proposal_id): Path<Uuid>,
    Json(body): Json<SupersedeRequest>,
) -> Result<Response, ApiError> {
    let outcome = state
        .store
        .supersede_by_proposal_id(
            &session.context,
            SupersedeByProposalIdCommand {
                operation_id: Uuid::new_v4().to_string(),
                proposal_id,
                superseded_claim_ids: body.superseded_claim_ids,
            },
        )
        .map_err(|e| map_semantic_error(&e))?;
    metrics::counter!("console_mutations_total", "action" => "supersede").increment(1);
    Ok(Json(json!({
        "status": "superseded",
        "claim_id": outcome.generated.claim_id,
        "event_seq": outcome.event.event_seq,
    }))
    .into_response())
}

// ── events (SSE) ──────────────────────────────────────────────────────────────

/// `GET /api/v1/events` — Server-Sent Events stream of job/update
/// notifications. Requires a valid session (same as every read route; no CSRF —
/// it is a GET). Each `String` published on the broadcast channel is emitted as
/// one `data:` frame; a `Lagged` subscriber (fell behind the ring buffer) gets
/// a comment rather than a dropped connection. A 15s keep-alive comment keeps
/// idle connections alive through proxies. There is no publisher wired yet —
/// the plumbing exists so later phases can push events via
/// [`ConsoleApiState::events_sender`].
async fn events(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).map(|item| {
        let event = match item {
            Ok(message) => Event::default().data(message),
            // Slow client: signal the gap without tearing down the stream.
            Err(_lagged) => Event::default().comment("lagged"),
        };
        Ok(event)
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

// ── static assets (console) ───────────────────────────────────────────────────

/// Builds a static-file router for the built Console assets in `dir`, served at
/// `/` (as a fallback). Two security properties, both delegated to
/// battle-tested `tower-http` layers rather than hand-rolled:
///
/// * **Path traversal** — [`ServeDir`] validates every request path and refuses
///   to resolve `..`/absolute/prefix components outside `dir` (404).
/// * **CSP** — [`SetResponseHeaderLayer`] stamps [`CONSOLE_CSP`] on *every*
///   response (success and error alike), uniformly, so no route can forget it.
///
/// Deliberately NOT behind the console-session auth gate: an unauthenticated
/// browser must load the login page/bundle before any session exists.
pub fn static_router(dir: PathBuf) -> Router {
    Router::new()
        .fallback_service(ServeDir::new(dir))
        // CSP — stamped on every response (success + error).
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONSOLE_CSP),
        ))
        // Defense-in-depth security headers (P1-1, 2026-07-20).
        // These complement the CSP; each closes a different vector.
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            // Deny everything we don't use. The SPA uses: none of the
            // device APIs. Keep the list explicit so adding one is a
            // deliberate act.
            HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=(), payment=(), \
                 usb=(), magnetometer=(), gyroscope=(), accelerometer=()",
            ),
        ))
        // HSTS — only meaningful over TLS, but harmless on loopback HTTP
        // dev (browsers ignore it on http://localhost). Set unconditionally
        // so prod deployments behind a TLS proxy benefit immediately.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, cookie_value};

    #[test]
    fn constant_time_eq_matches_only_identical_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreu"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn cookie_value_extracts_named_pair() {
        assert_eq!(
            cookie_value("brain_console_session=abc; Path=/", "brain_console_session"),
            Some("abc")
        );
        assert_eq!(
            cookie_value(
                "other=1; brain_console_session=xyz",
                "brain_console_session"
            ),
            Some("xyz")
        );
        assert_eq!(cookie_value("other=1", "brain_console_session"), None);
    }
}
