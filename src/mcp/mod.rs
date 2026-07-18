/// Production auth boundary — capabilities, auth policy, token redaction
/// (Task 3.3). Contract-level: real TLS/OAuth termination is Phase 6
/// deployment; this module defines the policy framework and capability
/// enforcement that the deployment layer plugs into.
pub mod auth;
/// MCP tool handler functions.
pub mod handlers;
/// MCP helper utilities — argument extraction and tool result types.
pub mod helpers;
/// MCP tool definitions and dispatch table.
pub mod tools;

use std::future::Future;
use std::sync::Arc;

use helpers::{WikiError, err_code};

use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::AnnotateAble;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, RawResource, ReadResourceRequestParams, ReadResourceResult,
    ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use tokio::sync::mpsc;

use crate::engine::{EngineState, WikiEngine};
use crate::markdown;
use crate::observability::IngestRateLimiter;
use crate::slug::{Slug, WikiUri};

// ── McpServer ─────────────────────────────────────────────────────────────────

/// MCP server — dispatches MCP tool calls and resource reads to the wiki engine.
#[derive(Clone)]
pub struct McpServer {
    /// Shared wiki engine handle.
    pub manager: Arc<WikiEngine>,
    /// Optional channel used by `serve --web` to refresh the Hugo server after writes.
    web_refresh_tx: Option<mpsc::Sender<String>>,
    /// Auth policy enforced before every tool dispatch (Task B3). When None,
    /// no auth enforcement (legacy/dev mode). When Some, every call_tool
    /// checks AuthPolicy::allows(principal, tool_name) before dispatch.
    auth_policy: Option<auth::AuthPolicy>,
    /// Optional SemanticStore for brain_* tools (Phase C). When None, brain_*
    /// tools return "brain not initialized". When Some, brain_search/get/status
    /// query the semantic ledger.
    pub semantic_store: Option<Arc<crate::semantic::SemanticStore>>,
    /// Optional AI provider for `brain_extract` (Task D3). When None,
    /// `brain_extract` returns an error rather than silently no-op-ing —
    /// there is no default/mock provider wired in here (Phase D "NO MOCK"
    /// decision); a caller that wants extraction must attach a real adapter.
    pub ai_provider: Option<Arc<dyn crate::provider::AiProvider>>,
    /// The authenticated principal for this server instance. In production
    /// this comes from a validated token at the transport edge; in dev the
    /// bootstrap principal has all capabilities.
    principal: auth::AuthPrincipal,
    /// Ingest limits (Task F2.3). `None` disables enforcement (legacy
    /// behavior — used by tests that pre-date the limit wiring and by any
    /// caller that builds an `McpServer` directly without going through
    /// `serve()`). `serve()` always attaches limits built from
    /// `ServeConfig`, so the production path is always gated.
    ingest_max_source_bytes: usize,
    rate_limiter: Option<IngestRateLimiter>,
}

impl McpServer {
    /// Create a new `McpServer` wrapping `manager` with NO auth enforcement
    /// (dev/legacy mode). The bootstrap principal has all capabilities.
    pub fn new(manager: Arc<WikiEngine>) -> Self {
        Self {
            manager,
            web_refresh_tx: None,
            auth_policy: None,
            semantic_store: None,
            ai_provider: None,
            principal: owner_principal(),
            ingest_max_source_bytes: default_ingest_max_source_bytes(),
            rate_limiter: None,
        }
    }

    /// Create a new `McpServer` with auth enforcement enabled (Task B3).
    /// Every `call_tool` checks `policy.allows(principal, tool_name)` before
    /// dispatch; a denied tool returns a structured capability-denied error.
    pub fn with_auth(
        manager: Arc<WikiEngine>,
        policy: auth::AuthPolicy,
        principal: auth::AuthPrincipal,
    ) -> Self {
        Self {
            manager,
            web_refresh_tx: None,
            auth_policy: Some(policy),
            semantic_store: None,
            ai_provider: None,
            principal,
            ingest_max_source_bytes: default_ingest_max_source_bytes(),
            rate_limiter: None,
        }
    }

    /// Enable auth enforcement on an already-built server (chainable), without
    /// dropping other fields such as the web-refresh channel or semantic store.
    /// This is how `serve()` activates the gate on the hot path (Task B3 / F1).
    pub fn with_auth_policy(
        mut self,
        policy: auth::AuthPolicy,
        principal: auth::AuthPrincipal,
    ) -> Self {
        self.auth_policy = Some(policy);
        self.principal = principal;
        self
    }

    /// The dispatch-time auth gate. Returns `Err(message)` when the current
    /// principal lacks the capability the tool requires; `Ok(())` when allowed
    /// or when no auth policy is configured (dev/legacy). `call_tool` calls this
    /// before every dispatch, so a test that calls it exercises the real gate.
    pub fn check_capability(&self, tool: &str) -> Result<(), String> {
        let Some(policy) = &self.auth_policy else {
            return Ok(());
        };
        if policy.allows(&self.principal, tool) {
            return Ok(());
        }
        let required = policy
            .required_capability(tool)
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|| "unknown".to_owned());
        Err(format!(
            "capability denied: tool '{tool}' requires {required} (principal '{}')",
            self.principal.id
        ))
    }

    /// Derive the store-layer `TrustedContext` for the current MCP
    /// `principal` (Phase D Task D2 / F2). `brain_*` handlers call this
    /// instead of hardcoding `store.trusted_context()`, so a store-layer
    /// mutation is actually attributed to — and gated by the capabilities
    /// of — whoever is really calling, not always the owner.
    ///
    /// The owner (bootstrap) principal maps directly to
    /// `store.trusted_context()`: `SemanticStore::register_client_scoped`
    /// rejects any label starting with `__` (the bootstrap client's own
    /// label is `__bootstrap__`), so the owner can never go through the
    /// registration path — it doesn't need to, since the dispatch-level
    /// `check_capability` gate (using `AuthPolicy`) already grants the
    /// owner every capability.
    ///
    /// Any other principal is registered (idempotently, by `principal.id`
    /// as the client label) with the store-layer capability strings that
    /// correspond to its dispatch-layer `Capability`s. Registration is
    /// first-write-wins (`register_client_scoped` does not escalate an
    /// already-registered label's capabilities on a later call — see
    /// `tests/semantic_capability_v1.rs`), so a worker's store-layer grant
    /// stays pinned to what it had on first contact.
    pub fn brain_context(
        &self,
        store: &crate::semantic::SemanticStore,
    ) -> Result<crate::semantic::TrustedContext, String> {
        if self.principal.id == OWNER_PRINCIPAL_ID {
            return Ok(store.trusted_context());
        }
        let capabilities: Vec<&str> = self
            .principal
            .capabilities
            .iter()
            .filter_map(|capability| match capability {
                auth::Capability::Propose => Some("propose"),
                auth::Capability::Confirm => Some("confirm"),
                auth::Capability::Purge => Some("purge"),
                auth::Capability::Read | auth::Capability::Capture | auth::Capability::Admin => {
                    None
                }
            })
            .collect();
        store
            .register_client_scoped(&self.principal.id, &capabilities)
            .map_err(|e| format!("{e}"))
    }

    /// Return a strong reference to the shared engine (Task 3.2). Cloning an
    /// `McpServer` clones the `Arc`, not the engine — so every session and
    /// every reconnect observes the same engine state. This accessor makes
    /// that invariant testable: two server clones must return `Arc` pointers
    /// to the same underlying `WikiEngine`. (Named `shared_engine` to avoid
    /// clashing with the existing `engine()` read-guard accessor.)
    pub fn shared_engine(&self) -> Arc<WikiEngine> {
        Arc::clone(&self.manager)
    }

    /// Attach a SemanticStore so brain_* tools can query the semantic ledger.
    /// (Phase C Task C1.)
    pub fn with_semantic_store(mut self, store: Arc<crate::semantic::SemanticStore>) -> Self {
        self.semantic_store = Some(store);
        self
    }

    /// Attach an `AiProvider` so `brain_extract` can run real extraction
    /// (Task D3). Without this, `brain_extract` errors rather than
    /// fabricating a response.
    pub fn with_ai_provider(mut self, provider: Arc<dyn crate::provider::AiProvider>) -> Self {
        self.ai_provider = Some(provider);
        self
    }

    /// Attach ingest size + rate limits (Task F2.3). `max_source_bytes` is the
    /// per-source byte cap; `rate_limiter` is the per-client sliding-window
    /// limiter (already constructed with the configured per-minute cap). When
    /// attached, the ingest entry points (`brain_ingest_source`,
    /// `brain_capture`, `wiki_ingest`) check both before touching the store.
    /// When NOT attached (the default for tests), the limits are unenforced —
    /// matching the pre-F2.3 behavior so existing tests keep working without
    /// every fixture constructing a limiter.
    pub fn with_ingest_limits(
        mut self,
        max_source_bytes: usize,
        rate_limiter: IngestRateLimiter,
    ) -> Self {
        self.ingest_max_source_bytes = max_source_bytes;
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Ingest-limit gate (Task F2.3). Ingest entry points call this with the
    /// byte size of the source they are about to ingest; it returns:
    /// - `Ok(())` when under both caps (the rate-limit attempt is recorded).
    /// - `Err(WikiError::PayloadTooLarge)` when `bytes > max_source_bytes`.
    /// - `Err(WikiError::RateLimited)` when the per-minute cap is hit.
    ///
    /// Order: size first, rate second. Rationale: a size rejection is
    /// deterministic (no client state mutation), and recording it against the
    /// rate limiter would consume budget for a call that never reached the
    /// store — wrong. A rate rejection records nothing (see
    /// [`IngestRateLimiter::check_and_record`]).
    ///
    /// When no limiter is attached (`self.rate_limiter.is_none()`), this is a
    /// no-op `Ok(())` — the size cap is also bypassed in that shape, matching
    /// the pre-F2.3 behavior so legacy callers and tests are unaffected.
    fn check_ingest_limits(&self, bytes: usize) -> Result<(), helpers::WikiError> {
        let Some(limiter) = &self.rate_limiter else {
            return Ok(());
        };
        if bytes > self.ingest_max_source_bytes {
            return Err(helpers::WikiError::PayloadTooLarge);
        }
        if !limiter.check_and_record(&self.principal.id) {
            return Err(helpers::WikiError::RateLimited);
        }
        Ok(())
    }

    /// The configured per-source byte cap (read-only). Used by the
    /// rejection-message helper to render the actionable limit value back to
    /// the client. Returns the default when no limiter is attached.
    pub(super) fn ingest_max_source_bytes(&self) -> usize {
        self.ingest_max_source_bytes
    }

    /// The configured per-minute rate cap (read-only). `None` when no limiter
    /// is attached (i.e. limits are disabled). Used by the rejection-message
    /// helper to render the actionable limit value.
    pub(super) fn rate_limiter_max_per_minute(&self) -> Option<u32> {
        self.rate_limiter.as_ref().map(|l| l.max_per_minute())
    }

    /// Create a new `McpServer` with web-refresh notifications enabled.
    pub fn with_web_refresh(
        manager: Arc<WikiEngine>,
        web_refresh_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            manager,
            web_refresh_tx: Some(web_refresh_tx),
            auth_policy: None,
            semantic_store: None,
            ai_provider: None,
            principal: owner_principal(),
            ingest_max_source_bytes: default_ingest_max_source_bytes(),
            rate_limiter: None,
        }
    }

    /// Acquire a read guard on the engine state.
    pub fn engine(&self) -> parking_lot::RwLockReadGuard<'_, EngineState> {
        self.manager.state.read()
    }

    /// Notify the optional web supervisor that content for `wiki_name` changed.
    pub fn notify_web_refresh(&self, wiki_name: &str) {
        if let Some(tx) = &self.web_refresh_tx
            && let Err(e) = tx.try_send(wiki_name.to_string())
        {
            tracing::warn!(wiki = %wiki_name, error = %e, "web refresh notification dropped");
        }
    }

    fn list_wiki_resources(&self) -> Vec<rmcp::model::Resource> {
        let engine = self.manager.state.read();
        let mut resources = Vec::new();
        for (wiki_name, space) in &engine.spaces {
            let walker = walkdir::WalkDir::new(&space.wiki_root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().is_file()
                        && e.path().extension().and_then(|x| x.to_str()) == Some("md")
                });
            for entry in walker {
                if let Ok(slug) = Slug::from_path(entry.path(), &space.wiki_root) {
                    let uri = format!("wiki://{wiki_name}/{slug}");
                    resources.push(RawResource::new(uri, slug.title()).no_annotation());
                }
            }
        }
        resources
    }
}

/// The dispatch-layer principal id reserved for the local owner — matches
/// `SemanticStore`'s `BOOTSTRAP_CLIENT_LABEL` at the store layer (Task D2).
const OWNER_PRINCIPAL_ID: &str = "__bootstrap__";

/// The default per-source byte cap when an `McpServer` is built without
/// explicit ingest limits (i.e. NOT via `serve()`). Matches
/// `ServeConfig::default().ingest_max_source_bytes` so direct-constructor
/// callers get the same 10MB default — only `rate_limiter` differs (None),
/// which makes the gate a no-op until `with_ingest_limits` attaches one.
fn default_ingest_max_source_bytes() -> usize {
    10 * 1024 * 1024
}

/// The owner principal has all capabilities (single-owner local mode). In
/// production the transport edge replaces this with a validated-token principal
/// (e.g. a proposal-only worker). `serve()` uses this so the local owner keeps
/// full access while the auth gate stays active on the hot path.
pub fn owner_principal() -> auth::AuthPrincipal {
    auth::AuthPrincipal {
        id: OWNER_PRINCIPAL_ID.to_owned(),
        capabilities: vec![
            auth::Capability::Read,
            auth::Capability::Capture,
            auth::Capability::Propose,
            auth::Capability::Confirm,
            auth::Capability::Purge,
            auth::Capability::Admin,
        ],
    }
}

// ── ServerHandler impl ────────────────────────────────────────────────────────

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("llm-wiki", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: tools::tool_list(),
            next_cursor: None,
            meta: None,
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let args = request.arguments.unwrap_or_default();
        let name = request.name.to_string();
        let server = self.clone();

        // Task B3: enforce auth policy before dispatch (inside async block
        // so both paths return the same Future type).
        async move {
            // Auth gate: check before dispatch (same method a dispatch-level
            // test drives — Task B3 / F1 fix).
            if let Err(msg) = server.check_capability(&name) {
                tracing::warn!(
                    tool = %name,
                    principal = %server.principal.id,
                    "tool call DENIED by auth policy"
                );
                return Err(McpError::internal_error(msg, None));
            }

            // Task F2.2: count every tool dispatch by name. Incremented AFTER
            // the auth gate so a denied tool does NOT inflate the success
            // counter; a denied call surfaces as an error in the response
            // (already counted by the existing tracing::warn above). The
            // metrics facade is a no-op when no recorder is installed.
            metrics::counter!("mcp_calls_total", "tool" => name.clone()).increment(1);
            // Keep a clone for the post-dispatch per-status counter — `name`
            // itself is moved into the `spawn_blocking` closure below.
            let name_for_status = name.clone();

            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                tokio::task::spawn_blocking(move || tools::call(&server, &name, &args)),
            )
            .await
            .map_err(|_| McpError::internal_error("tool call timed out after 30s", None))?
            .map_err(|e| McpError::internal_error(format!("tool task failed: {e}"), None))?;

            // Send resource update notifications for ingested pages.
            if !result.notify_uris.is_empty() {
                let peer = context.peer.clone();
                let uris = result.notify_uris.clone();
                tokio::spawn(async move {
                    for uri in uris {
                        if let Err(e) = peer
                            .notify_resource_updated(
                                rmcp::model::ResourceUpdatedNotificationParam { uri: uri.clone() },
                            )
                            .await
                        {
                            tracing::warn!(error = %e, uri = %uri, "resource notification failed");
                        }
                    }
                });
            }

            // Send resource list changed notification for space operations.
            if result.notify_resources_changed {
                let peer = context.peer.clone();
                tokio::spawn(async move {
                    if let Err(e) = peer.notify_resource_list_changed().await {
                        tracing::warn!(error = %e, "resource list changed notification failed");
                    }
                });
            }

            let mut tool_result = if result.is_error {
                // Task F2.2: per-status MCP dispatch counter so dashboards can
                // derive tool-level error rates from
                // `mcp_calls_total{status="error"} / mcp_calls_total{status="ok"}`.
                metrics::counter!("mcp_calls_total", "tool" => name_for_status.clone(), "status" => "error")
                    .increment(1);
                CallToolResult::error(result.content)
            } else {
                metrics::counter!("mcp_calls_total", "tool" => name_for_status.clone(), "status" => "ok")
                    .increment(1);
                CallToolResult::success(result.content)
            };
            // Task 3.1 §7.2: propagate structured content (if the handler set
            // it) so clients that understand structured_content read JSON
            // directly while others fall back to the text block.
            tool_result.structured_content = result.structured_content;

            Ok(tool_result)
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let resources = self.list_wiki_resources();
        std::future::ready(Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let uri = &request.uri;
        let result = if uri.starts_with("wiki://") {
            let engine = self.manager.state.read();
            match WikiUri::resolve(uri, None, &engine.config) {
                Ok((entry, slug)) => {
                    let wiki_root = engine
                        .space(&entry.name)
                        .map(|s| s.wiki_root.clone())
                        .unwrap_or_else(|_| std::path::PathBuf::from(&entry.path).join("wiki"));
                    match markdown::read_page(&slug, &wiki_root, false) {
                        Ok(content) => Ok(ReadResourceResult::new(vec![
                            ResourceContents::text(content, uri.to_string())
                                .with_mime_type("text/markdown"),
                        ])),
                        Err(e) => Err(McpError::internal_error(
                            err_code(WikiError::InternalError, format!("failed to read: {e}")),
                            None,
                        )),
                    }
                }
                Err(e) => Err(McpError::invalid_params(
                    err_code(WikiError::InvalidUri, format!("{e}")),
                    None,
                )),
            }
        } else {
            Err(McpError::invalid_params(
                err_code(
                    WikiError::InvalidUri,
                    format!("unsupported URI scheme: {uri}"),
                ),
                None,
            ))
        };
        std::future::ready(result)
    }
}
