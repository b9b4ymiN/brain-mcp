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
use axum::http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::galaxy::{GalaxyGraph, ZoomLevel};
use crate::semantic::{
    ConfirmByProposalIdCommand, RejectByProposalIdCommand, SemanticError, SemanticStore,
    SupersedeByProposalIdCommand, TrustedContext,
};

const SESSION_COOKIE: &str = "brain_console_session";
const CSRF_HEADER: &str = "X-CSRF-Token";

/// Ring-buffer depth for the in-process event broadcast. A subscriber that
/// falls this far behind gets a `Lagged` signal (surfaced as an SSE comment)
/// rather than blocking any publisher — events are advisory notifications.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Content-Security-Policy applied to every static (console asset) response.
/// Strict, no-inline baseline: only same-origin scripts/styles, no framing,
/// no plugins, no `<base>` hijack. The Task E1 build output must satisfy this
/// (Task 5.1 DoD: "CSP block inline"); it is deliberately stricter than the
/// API needs so the frontend can never regress into inline `<script>`.
pub const CONSOLE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

// ── state ───────────────────────────────────────────────────────────────────

/// A live Console session. Server-side half of the auth pair; the CSRF token
/// is echoed to the client in the login response body (NOT in the HttpOnly
/// cookie) so the frontend JS can read it and resubmit it on mutations.
struct Session {
    context: TrustedContext,
    csrf_token: String,
    expires_at: DateTime<Utc>,
}

/// Shared state for the Console API router. Cheap to clone — every field is
/// behind an `Arc`. Held in axum's [`State`].
#[derive(Clone)]
pub struct ConsoleApiState {
    store: Arc<SemanticStore>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    bootstrap_secret: Arc<String>,
    session_ttl: Duration,
    /// Adds `Secure` to the session cookie. Set when the server binds a
    /// non-loopback interface (mirrors the existing bind-address posture);
    /// omitted for loopback dev so cookies work over plain http.
    secure_cookie: bool,
    /// In-process fan-out for job/update notifications delivered over the SSE
    /// `/events` stream. Held as the `Sender` half so any future in-process
    /// publisher can `subscribe()`/`send()`; the initial `Receiver` is dropped.
    events: Arc<broadcast::Sender<String>>,
}

impl ConsoleApiState {
    /// Builds state with the default 24h session TTL.
    pub fn new(store: Arc<SemanticStore>, bootstrap_secret: String, secure_cookie: bool) -> Self {
        Self::with_session_ttl(store, bootstrap_secret, secure_cookie, Duration::hours(24))
    }

    /// Builds state with an explicit session TTL (test seam for expiry).
    pub fn with_session_ttl(
        store: Arc<SemanticStore>,
        bootstrap_secret: String,
        secure_cookie: bool,
        session_ttl: Duration,
    ) -> Self {
        let (events, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            store,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            bootstrap_secret: Arc::new(bootstrap_secret),
            session_ttl,
            secure_cookie,
            events: Arc::new(events),
        }
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
        .route("/events", get(events))
        .route("/search", get(search))
        .route("/get", get(get_subject))
        .route("/entity/timeline", get(timeline))
        .route("/inbox", get(inbox))
        .route("/inbox/{proposal_id}/evidence", get(evidence))
        .route("/inbox/{proposal_id}/approve", post(approve))
        .route("/inbox/{proposal_id}/reject", post(reject))
        .route("/inbox/{proposal_id}/supersede", post(supersede))
        .route("/galaxy", get(galaxy))
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

    fn forbidden_csrf() -> Self {
        Self::new(StatusCode::FORBIDDEN, "csrf_failed")
    }

    fn invalid_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request")
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

/// The result of a successful session lookup, used by both extractors.
struct ResolvedSession {
    session_id: String,
    context: TrustedContext,
    csrf_token: String,
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
/// wins as 401 even on a mutating route.
struct CsrfSession {
    context: TrustedContext,
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
        Ok(CsrfSession {
            context: resolved.context,
        })
    }
}

// ── auth routes ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    secret: String,
}

async fn login(State(state): State<ConsoleApiState>, body: Option<Json<LoginRequest>>) -> Response {
    let Some(Json(body)) = body else {
        return ApiError::new(StatusCode::BAD_REQUEST, "invalid_request").into_response();
    };
    if !constant_time_eq(body.secret.as_bytes(), state.bootstrap_secret.as_bytes()) {
        return ApiError::unauthorized().into_response();
    }

    let context = match state.store.register_client("console") {
        Ok(context) => context,
        Err(error) => return map_semantic_error(&error).into_response(),
    };
    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + state.session_ttl;
    state.sessions.write().insert(
        session_id.clone(),
        Session {
            context,
            csrf_token: csrf_token.clone(),
            expires_at,
        },
    );

    let cookie = session_cookie_header(&session_id, state.secure_cookie);
    let Ok(cookie_value) = HeaderValue::from_str(&cookie) else {
        return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error").into_response();
    };
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
    let query = params.query.to_lowercase();
    let top_k = params.top_k.unwrap_or(10);
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
            claim.subject.to_lowercase().contains(&query)
                || claim.predicate.to_lowercase().contains(&query)
        })
        .take(top_k)
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
            })
        })
        .collect();

    Ok(Json(json!({
        "query": query,
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
    domain: String,
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
        .claim_timeline(&params.domain, &params.subject, &params.predicate)
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(timeline).into_response())
}

async fn inbox(
    State(state): State<ConsoleApiState>,
    _session: AuthSession,
) -> Result<Response, ApiError> {
    let pending = state
        .store
        .list_pending_proposals()
        .map_err(|e| map_semantic_error(&e))?;
    Ok(Json(pending).into_response())
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
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONSOLE_CSP),
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
