use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ── Section structs ───────────────────────────────────────────────────────────

/// The `[global]` section of the global config file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalSection {
    /// Name of the wiki used when no `--wiki` flag is given.
    #[serde(default)]
    pub default_wiki: String,
}

/// A registered wiki entry in the `[[wikis]]` array of the global config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiEntry {
    /// Short identifier used in `wiki://` URIs and the `--wiki` flag.
    pub name: String,
    /// Absolute path to the wiki repository root on disk.
    pub path: String,
    /// Optional one-line description shown in `spaces list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional git remote URL for the wiki repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// Default values for CLI flags that can be overridden per-wiki via `wiki.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    /// Maximum number of search results returned (default: 10).
    #[serde(default = "default_search_top_k")]
    pub search_top_k: u32,
    /// Whether to include BM25 excerpts in search output (default: true).
    #[serde(default = "default_true")]
    pub search_excerpt: bool,
    /// Whether to include section index pages in search results (default: false).
    #[serde(default)]
    pub search_sections: bool,
    /// Page display mode: `"flat"` or `"hierarchical"` (default: `"flat"`).
    #[serde(default = "default_page_mode")]
    pub page_mode: String,
    /// Number of pages returned per `list` call (default: 20).
    #[serde(default = "default_list_page_size")]
    pub list_page_size: u32,
    /// Default output format: `"text"` or `"json"` (default: `"text"`).
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Maximum number of tag facet values to return (default: 10).
    #[serde(default = "default_facets_top_tags")]
    pub facets_top_tags: u32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            search_top_k: 10,
            search_excerpt: true,
            search_sections: false,
            page_mode: "flat".into(),
            list_page_size: 20,
            output_format: "text".into(),
            facets_top_tags: 10,
        }
    }
}

/// `[read]` section — controls how pages are read back.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReadConfig {
    /// Strip frontmatter from `content read` output when true (default: false).
    #[serde(default)]
    pub no_frontmatter: bool,
}

/// `[index]` section — Tantivy index configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    /// Automatically rebuild the index on startup when stale (default: false).
    #[serde(default)]
    pub auto_rebuild: bool,
    /// Automatically recover a corrupt index by rebuilding (default: true).
    #[serde(default = "default_true")]
    pub auto_recovery: bool,
    /// Tantivy index writer memory budget in megabytes (default: 50).
    #[serde(default = "default_memory_budget_mb")]
    pub memory_budget_mb: u32,
    /// Tantivy tokenizer name (default: `"en_stem"`).
    #[serde(default = "default_tokenizer")]
    pub tokenizer: String,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            auto_rebuild: false,
            auto_recovery: true,
            memory_budget_mb: 50,
            tokenizer: "en_stem".into(),
        }
    }
}

/// Graph rendering and community detection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphConfig {
    /// Default graph output format: `"mermaid"`, `"dot"`, or `"llms"` (default: `"mermaid"`).
    #[serde(default = "default_graph_format")]
    pub format: String,
    /// Default hop depth for subgraph extraction (default: 3).
    #[serde(default = "default_graph_depth")]
    pub depth: u32,
    /// Page types to include when no `--type` flag is given (empty = all).
    #[serde(default)]
    pub r#type: Vec<String>,
    /// Default output file path for graph commands (empty = stdout).
    #[serde(default)]
    pub output: String,
    /// Minimum local node count before Louvain community detection runs (default 30).
    #[serde(default = "default_min_nodes_for_communities")]
    pub min_nodes_for_communities: usize,
    /// Maximum community-peer suggestions returned by `wiki_suggest` strategy 4 (default 2).
    #[serde(default = "default_community_suggestions_limit")]
    pub community_suggestions_limit: usize,
    /// Enable snapshot warm-start for the graph cache (default: true).
    /// Set false in CI or tests to avoid snapshot files.
    #[serde(default = "default_true")]
    pub snapshot: bool,
    /// Number of snapshots to retain per wiki space (default: 3).
    #[serde(default = "default_snapshot_keep")]
    pub snapshot_keep: u32,
    /// Snapshot format: "bincode+lz4" | "bincode" | "bincode+zstd" (default: "bincode+lz4").
    #[serde(default = "default_snapshot_format")]
    pub snapshot_format: String,
    /// Enable structural topology algorithms in wiki_stats (diameter, radius, center).
    /// Lint rules articulation-point, bridge, periphery are always available via --rules.
    /// Default: true. Set false to skip structural computation in stats entirely.
    #[serde(default = "default_true")]
    pub structural_algorithms: bool,
    /// Maximum local node count before O(n²) diameter/radius/center/periphery algorithms are skipped (default: 2000).
    #[serde(default = "default_max_nodes_for_diameter")]
    pub max_nodes_for_diameter: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            format: "mermaid".into(),
            depth: 3,
            r#type: Vec::new(),
            output: String::new(),
            min_nodes_for_communities: default_min_nodes_for_communities(),
            community_suggestions_limit: default_community_suggestions_limit(),
            snapshot: true,
            snapshot_keep: 3,
            snapshot_format: "bincode+lz4".into(),
            structural_algorithms: true,
            max_nodes_for_diameter: default_max_nodes_for_diameter(),
        }
    }
}

fn default_min_nodes_for_communities() -> usize {
    30
}

fn default_community_suggestions_limit() -> usize {
    2
}

fn default_snapshot_keep() -> u32 {
    3
}

fn default_snapshot_format() -> String {
    "bincode+lz4".into()
}
fn default_max_nodes_for_diameter() -> usize {
    2000
}

fn default_acp_max_sessions() -> usize {
    20
}

/// Default env var name holding the Console dev USERNAME (Phase G:
/// username/password migration, 2026-07-20). Operators can override the
/// name in TOML if they prefer a different env key, but the default
/// matches `.env.example` so a vanilla deployment "just works".
///
/// **Namespaced as `BRAIN_*` (not bare `USERNAME`/`PASSWORD`)** because bare
/// `USERNAME` collides with the Windows built-in env var (always set to the
/// host user's login name), which silently overrides the `.env` value via
/// docker-compose `${USERNAME:-}` substitution. Found live 2026-07-20 when
/// `docker compose up` rejected every login because the container saw
/// `USERNAME=Mining_Admin` instead of the operator-configured value.
fn default_console_username_env() -> String {
    "BRAIN_USERNAME".to_string()
}

/// Default env var name holding the Console dev PASSWORD. See
/// [`default_console_username_env`].
fn default_console_password_env() -> String {
    "BRAIN_PASSWORD".to_string()
}
fn default_acp_session_ttl_secs() -> u64 {
    1800
}

fn default_mcp_session_keep_alive_secs() -> u64 {
    21_600
}

fn default_mcp_init_timeout_secs() -> u64 {
    60
}

/// Default env-var name holding comma-separated MCP Bearer tokens.
/// Mirrors the `BRAIN_USERNAME`/`BRAIN_PASSWORD` pattern: config stores
/// the env-var NAME, not the value. The value is read at startup via
/// `std::env::var`. Empty/unset + public bind → fail-closed (refuse start).
fn default_mcp_bearer_tokens_env() -> String {
    "BRAIN_MCP_TOKENS".into()
}

fn default_mcp_completed_cache_ttl_secs() -> u64 {
    60
}

/// Default per-tool-call timeout for MCP tool dispatch (300 s / 5 min).
///
/// Found live 2026-07-19: the previous hardcoded 30 s timeout was too short
/// for `brain_extract` against reasoning models (glm-4.6/glm-5.2 spend
/// 15-30 s of `reasoning_content` before writing the final `content`, and
/// larger extraction chunks run longer). When the timeout fired mid-call,
/// the MCP layer returned an empty response body that downstream parsed as
/// "AI response was not valid JSON after bounded repair (discarded): EOF
/// while parsing a value at line 1 column 0" — masking the real cause.
/// 300 s leaves ~10× headroom over the longest observed reasoning-model
/// extraction while still bounding a stuck tool call. Operators with a
/// faster provider or smaller prompts can lower this in `[serve]`.
fn default_mcp_tool_call_timeout_secs() -> u64 {
    300
}

fn default_mcp_stateful_mode() -> bool {
    false
}

fn default_mcp_json_response() -> bool {
    true
}

/// Resolved Console credentials produced by
/// [`ServeConfig::resolve_bootstrap_credentials`]. `username` may be `None`
/// (legacy single-credential mode); `password` is always present and
/// non-empty when this struct is `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapCredentials {
    pub username: Option<String>,
    pub password: String,
}

/// `[serve]` section — HTTP and ACP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeConfig {
    /// Enable the HTTP transport by default (default: false).
    #[serde(default)]
    pub http: bool,
    /// TCP port for the HTTP server (default: 8080).
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    /// Hostnames accepted by the HTTP server (default: localhost variants).
    #[serde(default = "default_http_allowed_hosts")]
    pub http_allowed_hosts: Vec<String>,
    /// Bind address for the HTTP server (default: `127.0.0.1` — loopback only).
    /// **Security:** the default is loopback to prevent unauthenticated exposure.
    /// Set `http_bind_all_interfaces: true` to bind `0.0.0.0` instead — only do
    /// this behind a reverse proxy with authentication (S1 finding, Task B1).
    #[serde(default = "default_http_bind_address")]
    pub http_bind_address: String,
    /// Explicit opt-in to bind all interfaces (`0.0.0.0`). Default: false.
    /// When true, emits a startup warning about unauthenticated exposure.
    #[serde(default)]
    pub http_bind_all_interfaces: bool,
    /// Enable the ACP transport by default (default: false).
    #[serde(default)]
    pub acp: bool,
    /// Maximum automatic restart attempts after a server crash (default: 10).
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    /// Seconds to wait between restart attempts (default: 1).
    #[serde(default = "default_restart_backoff")]
    pub restart_backoff: u32,
    /// Interval in seconds between ACP heartbeat pings (default: 60).
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u32,
    /// Maximum number of concurrent ACP sessions (default: 20). Rejects NewSession when reached.
    #[serde(default = "default_acp_max_sessions")]
    pub acp_max_sessions: usize,
    /// Seconds before an idle ACP session is cleaned up (default: 1800 / 30 min).
    /// Set to 0 to disable idle cleanup.
    #[serde(default = "default_acp_session_ttl_secs")]
    pub acp_session_ttl_secs: u64,
    /// Seconds before an idle MCP HTTP session is closed (default: 21600 / 6h).
    /// Set to 0 to disable the idle timeout.
    #[serde(default = "default_mcp_session_keep_alive_secs")]
    pub mcp_session_keep_alive_secs: u64,
    /// Seconds to wait for MCP HTTP initialize after session creation (default: 60).
    /// Set to 0 to disable the initialize timeout.
    #[serde(default = "default_mcp_init_timeout_secs")]
    pub mcp_init_timeout_secs: u64,
    /// Env-var NAME (not value) holding comma-separated Bearer tokens
    /// accepted on `/mcp`. Empty/unset + `http_bind_all_interfaces=true`
    /// is a fail-closed configuration error (server refuses to start).
    /// Default: `"BRAIN_MCP_TOKENS"`. Loopback deployments with this unset
    /// keep the legacy unauthenticated `/mcp` (backward compatible).
    #[serde(default = "default_mcp_bearer_tokens_env")]
    pub mcp_bearer_tokens_env: String,
    /// Seconds to keep completed MCP request stream caches for late resume requests (default: 60).
    #[serde(default = "default_mcp_completed_cache_ttl_secs")]
    pub mcp_completed_cache_ttl_secs: u64,
    /// Per-tool-call timeout for MCP tool dispatch in seconds (default: 300).
    /// Set to 0 to disable the timeout (a stuck tool call then runs until the
    /// client disconnects). Found live 2026-07-19: the previous hardcoded
    /// 30 s timeout was too short for `brain_extract` against reasoning
    /// models (glm-4.6/glm-5.2) — see `default_mcp_tool_call_timeout_secs`.
    #[serde(default = "default_mcp_tool_call_timeout_secs")]
    pub mcp_tool_call_timeout_secs: u64,
    /// Use stateful Streamable HTTP sessions (default: false).
    #[serde(default = "default_mcp_stateful_mode")]
    pub mcp_stateful_mode: bool,
    /// Return direct JSON responses in stateless HTTP mode (default: true).
    #[serde(default = "default_mcp_json_response")]
    pub mcp_json_response: bool,
    /// Dev-grade bootstrap secret for the Console HTTP API (Phase E Task E0.2).
    /// **Fail-closed:** absent (`None`) means the `/api/v1` Console router is
    /// not mounted at all — no console API surface exists. Set only for local
    /// dev; production auth is OAuth (Phase F).
    #[serde(default)]
    pub console_dev_bootstrap_secret: Option<String>,
    /// Path to a file containing the bootstrap secret (Phase F1.2). If set,
    /// the file's trimmed contents OVERRIDE `console_dev_bootstrap_secret`.
    /// Use this in production (Docker secrets, systemd `LoadCredential`) to
    /// avoid leaking the secret via env/config-file inspection — env values
    /// are visible via `docker inspect` whereas a bind-mounted secret file
    /// with mode 0600 is not. **Fail-closed:** a set path that cannot be read
    /// (missing/unreadable) is a startup error; the server does NOT fall back
    /// to an empty/direct secret. Priority: `file > direct string`.
    #[serde(default)]
    pub console_dev_bootstrap_secret_file: Option<std::path::PathBuf>,
    /// Env var name holding the Console dev USERNAME (Phase G:
    /// username/password migration, 2026-07-20). Read at server startup via
    /// [`std::env::var`]; the value never enters config-file or disk. Default
    /// `"USERNAME"`. When the env var is unset, no username is enforced —
    /// legacy deployments that only set `console_dev_bootstrap_secret` keep
    /// working (single-credential mode). When the env var IS set, the login
    /// route requires `body.username` to match it.
    #[serde(default = "default_console_username_env")]
    pub console_dev_bootstrap_username_env: String,
    /// Env var name holding the Console dev PASSWORD. Default `"PASSWORD"`.
    /// Priority for the password credential:
    ///   1. this env var (if set at startup)
    ///   2. `console_dev_bootstrap_secret_file` (legacy Docker-secret path)
    ///   3. `console_dev_bootstrap_secret` (legacy direct-string fallback)
    ///      See [`Self::resolve_bootstrap_credentials`].
    #[serde(default = "default_console_password_env")]
    pub console_dev_bootstrap_password_env: String,
    /// Directory of built Console static assets, served at `/` as a fallback
    /// with a strict CSP and path-traversal protection (Phase E Task E0.3).
    /// `None` (default) = no static serving. Points at the Task E1 Svelte build
    /// output (e.g. `web/console/dist`); unlike the API router this is not an
    /// auth gate — the login page must load before a session exists.
    #[serde(default)]
    pub console_static_dir: Option<std::path::PathBuf>,
    /// Maximum bytes allowed for a single ingest source (Task F2.3). Default:
    /// 10 MB. Sources exceeding this are rejected with 413 Payload Too Large
    /// (the MCP surface surfaces this as a structured error — see
    /// `WikiError::PayloadTooLarge`). Enforced at the MCP ingest entry points
    /// (`brain_ingest_source`, `brain_capture`, `wiki_ingest`).
    #[serde(default = "default_ingest_max_source_bytes")]
    pub ingest_max_source_bytes: usize,
    /// Maximum number of ingest sources per minute per client (Task F2.3).
    /// Default: 60. Clients exceeding this are rejected with 429 Too Many
    /// Requests (the MCP surface surfaces this as a structured error — see
    /// `WikiError::RateLimited`). Enforced via a per-client sliding-window
    /// limiter keyed on the authenticated principal id.
    #[serde(default = "default_ingest_max_sources_per_minute")]
    pub ingest_max_sources_per_minute: u32,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            http: false,
            http_port: 8080,
            http_allowed_hosts: default_http_allowed_hosts(),
            http_bind_address: default_http_bind_address(),
            http_bind_all_interfaces: false,
            acp: false,
            max_restarts: 10,
            restart_backoff: 1,
            heartbeat_secs: 60,
            acp_max_sessions: default_acp_max_sessions(),
            acp_session_ttl_secs: default_acp_session_ttl_secs(),
            mcp_session_keep_alive_secs: default_mcp_session_keep_alive_secs(),
            mcp_init_timeout_secs: default_mcp_init_timeout_secs(),
            mcp_bearer_tokens_env: default_mcp_bearer_tokens_env(),
            mcp_completed_cache_ttl_secs: default_mcp_completed_cache_ttl_secs(),
            mcp_tool_call_timeout_secs: default_mcp_tool_call_timeout_secs(),
            mcp_stateful_mode: default_mcp_stateful_mode(),
            mcp_json_response: default_mcp_json_response(),
            console_dev_bootstrap_secret: None,
            console_dev_bootstrap_secret_file: None,
            console_dev_bootstrap_username_env: default_console_username_env(),
            console_dev_bootstrap_password_env: default_console_password_env(),
            console_static_dir: None,
            ingest_max_source_bytes: default_ingest_max_source_bytes(),
            ingest_max_sources_per_minute: default_ingest_max_sources_per_minute(),
        }
    }
}

impl ServeConfig {
    /// Resolves the effective Console bootstrap secret.
    ///
    /// Priority: `console_dev_bootstrap_secret_file` > `console_dev_bootstrap_secret`.
    ///
    /// **Fail-closed:** if `console_dev_bootstrap_secret_file` is `Some(_)` but
    /// the file cannot be read, this returns an error — the caller MUST NOT
    /// fall back to the direct string or to an empty secret. The file's
    /// contents are trimmed of surrounding ASCII whitespace (so a trailing
    /// newline from `echo $SECRET > file` or `printf` is dropped).
    ///
    /// Returns `Ok(None)` when neither field is set (Console API not mounted).
    /// Returns `Ok(Some(""))` only when the operator explicitly sets the
    /// direct string to `""` and no file is configured — the empty-secret
    /// check that suppresses the router mount happens at the call site.
    pub fn resolve_bootstrap_secret(&self) -> Result<Option<String>> {
        if let Some(path) = self.console_dev_bootstrap_secret_file.as_ref() {
            let raw = std::fs::read_to_string(path).with_context(|| {
                format!(
                    "failed to read console_dev_bootstrap_secret_file at {}",
                    path.display()
                )
            })?;
            let trimmed = raw.trim().to_string();
            return Ok(Some(trimmed));
        }
        Ok(self.console_dev_bootstrap_secret.clone())
    }

    /// The resolved Console credentials (Phase G: username/password
    /// migration, 2026-07-20). Replaces [`Self::resolve_bootstrap_secret`]
    /// for the production server-boot path while keeping the legacy method
    /// intact for tests and backward compatibility.
    ///
    /// Resolution priority:
    /// - **username**: `console_dev_bootstrap_username_env` → `std::env::var(name)`.
    ///   `None` if the env var is unset (legacy single-credential mode).
    /// - **password**:
    ///   1. `console_dev_bootstrap_password_env` → `std::env::var(name)` (wins)
    ///   2. `console_dev_bootstrap_secret_file` (legacy Docker-secret path)
    ///   3. `console_dev_bootstrap_secret` (legacy direct-string fallback)
    ///
    /// **Fail-closed:** if the password env var is set but empty, or if the
    /// legacy file path is set but unreadable, this returns an error. The
    /// caller MUST NOT fall back to an empty password.
    ///
    /// Returns `Ok(None)` when no credential is configured (Console API not
    /// mounted). Returns `Ok(Some(creds))` with a non-empty password
    /// otherwise — the username may be `None`.
    pub fn resolve_bootstrap_credentials(&self) -> Result<Option<BootstrapCredentials>> {
        // Username: optional. Unset env var → None (legacy mode).
        let username = std::env::var(&self.console_dev_bootstrap_username_env)
            .ok()
            .filter(|s| !s.is_empty());

        // Password: env var wins; otherwise fall back to legacy resolution.
        let password = match std::env::var(&self.console_dev_bootstrap_password_env) {
            Ok(p) if !p.is_empty() => Some(p),
            // Empty string is an explicit configuration error — we do NOT
            // silently fall back to legacy secrets when the operator named a
            // password env var that resolves to empty.
            Ok(_) => {
                anyhow::bail!(
                    "console_dev_bootstrap_password_env='{}' resolved to an empty value",
                    self.console_dev_bootstrap_password_env
                );
            }
            Err(_) => self.resolve_bootstrap_secret()?,
        };

        match (username, password) {
            (u, Some(p)) if !p.is_empty() => Ok(Some(BootstrapCredentials {
                username: u,
                password: p,
            })),
            // No password configured → Console API not mounted (fail-closed).
            _ => Ok(None),
        }
    }

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
}

/// `[provider]` section — optional AI provider wiring for `brain_extract` /
/// `brain_propose` inference. **Disabled by default** (`enabled = false`):
/// the server has zero AI dependency unless an operator explicitly opts in.
/// No provider is ever silently enabled from an empty/default config.
///
/// Mirrors the shape `ZaiHttpAdapter` + `ProviderConfig` expect (see
/// `src/provider.rs` and `tests/adversarial_corpus_live_v1.rs`): the API key
/// itself is never stored here, only the name of an environment variable the
/// deployment adapter resolves at call time (`resolve_api_key` only
/// implements the `env:` scheme today).
///
/// §8.2 requires a recorded compliance acknowledgement before any provider
/// call can happen. `compliance_user_decision` is the fail-closed gate for
/// that: if `enabled = true` but it is empty, [`ProviderSection::resolve`]
/// returns an error rather than defaulting or guessing consent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSection {
    /// Opt-in switch. Default: false (no AI provider wired).
    #[serde(default)]
    pub enabled: bool,
    /// OpenAI-compatible chat-completions endpoint (default: Z.ai's).
    #[serde(default = "default_provider_base_url")]
    pub base_url: String,
    /// Name of the environment variable holding the API key (resolved at
    /// call time via `env:<name>`, never read or stored here).
    #[serde(default = "default_provider_api_key_env")]
    pub api_key_env: String,
    /// Model used for routine extraction calls.
    #[serde(default = "default_provider_model")]
    pub routine_model: String,
    /// Model used for reasoning/synthesis calls.
    #[serde(default = "default_provider_model")]
    pub reasoning_model: String,
    /// `max_tokens` budget for a `brain_extract` provider call (default:
    /// `crate::extraction::DEFAULT_EXTRACTION_MAX_TOKENS`, 8192). Bump this
    /// higher if you see `brain_extract` fail with a JSON-parse error on an
    /// EMPTY response — that shape means the model hit `finish_reason:
    /// "length"` before writing any answer, typically because a reasoning
    /// model (glm-4.6/glm-5.2 and similar) spent the whole budget on
    /// `reasoning_content` chain-of-thought first. Confirmed live 2026-07-19.
    #[serde(default = "default_provider_extraction_max_tokens")]
    pub extraction_max_tokens: u32,
    /// Per-request HTTP timeout for provider chat-completion calls, in
    /// seconds (default: 600). Found live 2026-07-19: the previous
    /// hardcoded 60 s was too short for `brain_extract` against reasoning
    /// models — see `default_provider_timeout_secs`. Set to 0 to disable
    /// the timeout (a stuck connection then hangs until the OS kills it).
    #[serde(default = "default_provider_timeout_secs")]
    pub timeout_secs: u64,
    /// §8.2 compliance: what the operator decided (free text). **Required**
    /// (non-empty) when `enabled = true` — this is the fail-closed consent
    /// gate, not a default-filled placeholder.
    #[serde(default)]
    pub compliance_user_decision: String,
    /// §8.2 compliance: known terms-of-service risk acknowledged.
    #[serde(default)]
    pub compliance_known_terms_risk: String,
    /// §8.2 compliance: data-retention terms (confirmed/unconfirmed + detail).
    #[serde(default = "default_provider_compliance_unconfirmed")]
    pub compliance_retention_terms: String,
    /// §8.2 compliance: training-usage terms.
    #[serde(default = "default_provider_compliance_unconfirmed")]
    pub compliance_training_terms: String,
    /// §8.2 compliance: processing region.
    #[serde(default = "default_provider_compliance_unconfirmed")]
    pub compliance_processing_region: String,
}

impl Default for ProviderSection {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_provider_base_url(),
            api_key_env: default_provider_api_key_env(),
            routine_model: default_provider_model(),
            reasoning_model: default_provider_model(),
            extraction_max_tokens: default_provider_extraction_max_tokens(),
            timeout_secs: default_provider_timeout_secs(),
            compliance_user_decision: String::new(),
            compliance_known_terms_risk: String::new(),
            compliance_retention_terms: default_provider_compliance_unconfirmed(),
            compliance_training_terms: default_provider_compliance_unconfirmed(),
            compliance_processing_region: default_provider_compliance_unconfirmed(),
        }
    }
}

impl ProviderSection {
    /// Resolve this section into a `(ProviderConfig, ComplianceRecord)` pair
    /// ready to build a `ZaiHttpAdapter`, or `Ok(None)` when the provider is
    /// not enabled (the default — safe to call unconditionally at startup).
    ///
    /// **Fail-closed:** `enabled = true` with an empty
    /// `compliance_user_decision` is a startup-time error, not a silently
    /// disabled feature — the caller decides whether to abort startup or
    /// warn-and-continue-without-the-provider (`server.rs::serve` does the
    /// latter, matching how a failed `SemanticStore::open` is handled: an
    /// optional subsystem degrades, the core server still starts).
    pub fn resolve(
        &self,
    ) -> Result<
        Option<(
            crate::provider::ProviderConfig,
            crate::provider::ComplianceRecord,
        )>,
    > {
        if !self.enabled {
            return Ok(None);
        }
        if self.compliance_user_decision.trim().is_empty() {
            anyhow::bail!(
                "[provider] enabled = true but compliance_user_decision is empty — §8.2 requires \
                 a recorded acknowledgement before any provider call; set it in config.toml"
            );
        }
        let provider_config = crate::provider::ProviderConfig {
            base_url: self.base_url.clone(),
            api_key_ref: format!("env:{}", self.api_key_env),
            routine_model: self.routine_model.clone(),
            reasoning_model: self.reasoning_model.clone(),
            kill_switch: false,
        };
        let compliance = crate::provider::ComplianceRecord {
            user_decision: self.compliance_user_decision.clone(),
            endpoint: self.base_url.clone(),
            workload: "extraction".to_owned(),
            known_terms_risk: self.compliance_known_terms_risk.clone(),
            retention_terms: self.compliance_retention_terms.clone(),
            training_terms: self.compliance_training_terms.clone(),
            processing_region: self.compliance_processing_region.clone(),
            acknowledged_at: chrono::Utc::now().to_rfc3339(),
        };
        Ok(Some((provider_config, compliance)))
    }
}

fn default_provider_base_url() -> String {
    "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned()
}
fn default_provider_api_key_env() -> String {
    "ZAI_API_KEY".to_owned()
}
fn default_provider_model() -> String {
    "glm-4.6".to_owned()
}
fn default_provider_extraction_max_tokens() -> u32 {
    crate::extraction::DEFAULT_EXTRACTION_MAX_TOKENS
}

/// Default per-request HTTP timeout for provider chat-completion calls
/// (600 s / 10 min). Found live 2026-07-19: the previous hardcoded 60 s
/// was too short for `brain_extract` against reasoning models — a single
/// ~4 KB chunk measured 119 s end-to-end against Z.ai glm-4.6 (the
/// `reasoning_content` chain-of-thought alone consumed ~4640 tokens).
/// 600 s leaves ~5× headroom over the longest observed extraction and
/// bounds a stuck connection. Operators with a faster provider can lower
/// this in `[provider]`.
fn default_provider_timeout_secs() -> u64 {
    600
}
fn default_provider_compliance_unconfirmed() -> String {
    "unconfirmed".to_owned()
}

/// `[validation]` section — frontmatter validation strictness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// How strictly unknown types are treated: `"loose"` (warn) or `"strict"` (error) (default: `"loose"`).
    #[serde(default = "default_type_strictness")]
    pub type_strictness: String,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            type_strictness: "loose".into(),
        }
    }
}

/// `[logging]` section — structured log file configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Directory where log files are written (default: `~/.llm-wiki/logs`).
    #[serde(default = "default_log_path")]
    pub log_path: String,
    /// Log rotation policy: `"daily"` or `"never"` (default: `"daily"`).
    #[serde(default = "default_log_rotation")]
    pub log_rotation: String,
    /// Maximum number of log files to retain before pruning (default: 7).
    #[serde(default = "default_log_max_files")]
    pub log_max_files: u32,
    /// Log line format: `"text"` or `"json"` (default: `"text"`).
    #[serde(default = "default_log_format")]
    pub log_format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            log_path: default_log_path(),
            log_rotation: "daily".into(),
            log_max_files: 7,
            log_format: "text".into(),
        }
    }
}

/// `[ingest]` section — controls ingest commit behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestConfig {
    /// Automatically commit ingested files to git after validation (default: true).
    #[serde(default = "default_true")]
    pub auto_commit: bool,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self { auto_commit: true }
    }
}

/// `[history]` section — git log / history command defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Enable `--follow` rename tracking in git log (default: true).
    #[serde(default = "default_true")]
    pub follow: bool,
    /// Default maximum number of history entries to return (default: 10).
    #[serde(default = "default_history_limit")]
    pub default_limit: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            follow: true,
            default_limit: 10,
        }
    }
}

/// `[watch]` section — filesystem watcher configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    /// Debounce delay in milliseconds before triggering ingest after a file change (default: 500).
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u32,
    /// Watcher backend selection (default: auto — statfs-detect broken filesystems).
    #[serde(default)]
    pub backend: WatchBackendConfig,
    /// Poll interval in milliseconds when running in `poll` mode
    /// (default: 30000, matches notify upstream).
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u32,
}

/// Operator-selected watcher backend.
///
/// `Auto` (default) statfs-probes each `wiki_root` at startup and falls back
/// to `Poll` if any sits on a known-broken filesystem (V9FS, NFS, CIFS,
/// FUSE). `Native` forces the OS-native watcher
/// (inotify/FSEvents/ReadDirectoryChanges). `Poll` forces polling — required
/// for Docker Desktop bind mounts on Windows/macOS where inotify silently
/// fails across the V9FS / gRPC-FUSE virtualized bind mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WatchBackendConfig {
    #[default]
    Auto,
    Native,
    Poll,
}

fn default_poll_interval_ms() -> u32 {
    30000
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: default_debounce_ms(),
            backend: WatchBackendConfig::Auto,
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

/// `[suggest]` section — related-page suggestion defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestConfig {
    /// Default maximum number of suggestions to return (default: 5).
    #[serde(default = "default_suggest_limit")]
    pub default_limit: u32,
    /// Minimum relevance score for a suggestion to be included (default: 0.1).
    #[serde(default = "default_suggest_min_score")]
    pub min_score: f32,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            default_limit: 5,
            min_score: 0.1,
        }
    }
}

/// `[search]` section — BM25 score multipliers by page status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Map of status value → score multiplier applied to BM25 results.
    #[serde(default = "default_search_status")]
    pub status: std::collections::HashMap<String, f32>,
}

fn default_search_status() -> std::collections::HashMap<String, f32> {
    [
        ("active".into(), 1.0_f32),
        ("draft".into(), 0.8),
        ("archived".into(), 0.3),
        ("unknown".into(), 0.9),
    ]
    .into_iter()
    .collect()
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            status: default_search_status(),
        }
    }
}

/// Configuration for the `stale` lint rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintConfig {
    /// Pages not updated within this many days are candidates for the `stale` rule (default 90).
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
    /// `stale` only fires when `confidence` is also below this threshold (default 0.4).
    #[serde(default = "default_stale_confidence_threshold")]
    pub stale_confidence_threshold: f32,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            stale_days: default_stale_days(),
            stale_confidence_threshold: default_stale_confidence_threshold(),
        }
    }
}

/// A user-defined redaction rule added to the built-in patterns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomPattern {
    /// Unique name used in redaction reports.
    pub name: String,
    /// Regex pattern to match sensitive text.
    pub pattern: String,
    /// Replacement string substituted for matched text (e.g. `"[REDACTED]"`).
    pub replacement: String,
}

/// `[redact]` section — sensitive-data redaction configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedactConfig {
    /// Built-in pattern names to disable (e.g. `["aws-key"]`).
    #[serde(default)]
    pub disable: Vec<String>,
    /// Additional user-defined redaction patterns.
    #[serde(default)]
    pub patterns: Vec<CustomPattern>,
}

// ── Composite configs ─────────────────────────────────────────────────────────

/// Root structure for `~/.llm-wiki/config.toml` — the global configuration file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    /// `[global]` section.
    #[serde(default)]
    pub global: GlobalSection,
    /// `[[wikis]]` array — registered wiki spaces.
    #[serde(default)]
    pub wikis: Vec<WikiEntry>,
    /// `[defaults]` section — CLI flag defaults.
    #[serde(default)]
    pub defaults: Defaults,
    /// `[read]` section.
    #[serde(default)]
    pub read: ReadConfig,
    /// `[index]` section.
    #[serde(default)]
    pub index: IndexConfig,
    /// `[graph]` section.
    #[serde(default)]
    pub graph: GraphConfig,
    /// `[serve]` section.
    #[serde(default)]
    pub serve: ServeConfig,
    /// `[provider]` section — optional AI provider wiring for `brain_extract`
    /// / `brain_propose`. Absent from config = disabled (default).
    #[serde(default)]
    pub provider: ProviderSection,
    /// `[validation]` section.
    #[serde(default)]
    pub validation: ValidationConfig,
    /// `[ingest]` section.
    #[serde(default)]
    pub ingest: IngestConfig,
    /// `[history]` section.
    #[serde(default)]
    pub history: HistoryConfig,
    /// `[suggest]` section.
    #[serde(default)]
    pub suggest: SuggestConfig,
    /// `[search]` section.
    #[serde(default)]
    pub search: SearchConfig,
    /// `[lint]` section.
    #[serde(default)]
    pub lint: LintConfig,
    /// `[logging]` section.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// `[watch]` section.
    #[serde(default)]
    pub watch: WatchConfig,
    /// `[redact]` section.
    #[serde(default)]
    pub redact: RedactConfig,
}

/// A type entry in `[types.<name>]` of `wiki.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeEntry {
    /// Relative path to the JSON Schema file for this type.
    pub schema: String,
    /// Human-readable description of the type.
    pub description: String,
}

/// Per-wiki configuration loaded from `<wiki-root>/wiki.toml`.
///
/// Fields present here override the corresponding `GlobalConfig` sections.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WikiConfig {
    /// Wiki display name (informational; used in export headers).
    #[serde(default)]
    pub name: String,
    /// One-line description of the wiki.
    #[serde(default)]
    pub description: String,
    /// `[types.<name>]` custom type registrations for this wiki.
    #[serde(default)]
    pub types: std::collections::HashMap<String, TypeEntry>,
    /// Per-wiki override for `[defaults]`.
    #[serde(default)]
    pub defaults: Option<Defaults>,
    /// Per-wiki override for `[read]`.
    #[serde(default)]
    pub read: Option<ReadConfig>,
    /// Per-wiki override for `[validation]`.
    #[serde(default)]
    pub validation: Option<ValidationConfig>,
    /// Per-wiki override for `[ingest]`.
    #[serde(default)]
    pub ingest: Option<IngestConfig>,
    /// Per-wiki override for `[graph]`.
    #[serde(default)]
    pub graph: Option<GraphConfig>,
    /// Per-wiki override for `[history]`.
    #[serde(default)]
    pub history: Option<HistoryConfig>,
    /// Per-wiki override for `[suggest]`.
    #[serde(default)]
    pub suggest: Option<SuggestConfig>,
    /// Per-wiki override for `[search]`.
    #[serde(default)]
    pub search: Option<SearchConfig>,
    /// Per-wiki override for `[lint]`.
    #[serde(default)]
    pub lint: Option<LintConfig>,
    /// Per-wiki override for `[redact]`.
    #[serde(default)]
    pub redact: Option<RedactConfig>,
    /// Content directory relative to repo root. Default: `"wiki"`.
    #[serde(default = "default_wiki_root")]
    pub wiki_root: String,
}

/// Fully merged config for a specific wiki — global settings overlaid with per-wiki overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedConfig {
    /// Resolved defaults section.
    pub defaults: Defaults,
    /// Resolved read section.
    pub read: ReadConfig,
    /// Resolved index section (always from global).
    pub index: IndexConfig,
    /// Resolved graph section.
    pub graph: GraphConfig,
    /// Resolved serve section (always from global).
    pub serve: ServeConfig,
    /// Resolved ingest section.
    pub ingest: IngestConfig,
    /// Resolved validation section.
    pub validation: ValidationConfig,
    /// Resolved history section.
    pub history: HistoryConfig,
    /// Resolved suggest section.
    pub suggest: SuggestConfig,
    /// Resolved search section (merged: per-wiki entries override global entries).
    pub search: SearchConfig,
    /// Resolved lint section.
    pub lint: LintConfig,
    /// Resolved redact section.
    pub redact: RedactConfig,
}

// ── Default value helpers ─────────────────────────────────────────────────────

fn default_search_top_k() -> u32 {
    10
}
fn default_true() -> bool {
    true
}
fn default_page_mode() -> String {
    "flat".into()
}
fn default_list_page_size() -> u32 {
    20
}
fn default_output_format() -> String {
    "text".into()
}
fn default_facets_top_tags() -> u32 {
    10
}
fn default_memory_budget_mb() -> u32 {
    50
}
fn default_tokenizer() -> String {
    "en_stem".into()
}
fn default_graph_format() -> String {
    "mermaid".into()
}
fn default_graph_depth() -> u32 {
    3
}
fn default_http_port() -> u16 {
    8080
}
fn default_http_allowed_hosts() -> Vec<String> {
    vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]
}
fn default_http_bind_address() -> String {
    "127.0.0.1".to_owned()
}
/// Default per-source ingest byte cap: 10 MB. Sized to comfortably admit a
/// multi-megabyte document chunk while rejecting accidental giant payloads
/// (e.g. a binary blob a client tried to ingest as text). §13 Task 6.2
/// "size/time limits ป้องกัน runaway ingest".
fn default_ingest_max_source_bytes() -> usize {
    10 * 1024 * 1024
}
/// Default per-client ingest rate cap: 60 sources / minute. Matches the F2.3
/// task spec default; loose enough for a single interactive client, tight
/// enough that a runaway loop can't run up provider cost in the minute before
/// an operator notices.
fn default_ingest_max_sources_per_minute() -> u32 {
    60
}
fn default_max_restarts() -> u32 {
    10
}
fn default_restart_backoff() -> u32 {
    1
}
fn default_heartbeat_secs() -> u32 {
    60
}
fn default_type_strictness() -> String {
    "loose".into()
}
fn default_log_path() -> String {
    std::path::PathBuf::from(home_dir())
        .join(".llm-wiki")
        .join("logs")
        .to_string_lossy()
        .into()
}

/// Cross-platform home directory: `USERPROFILE` (Windows) → `HOME` (Unix) → `"."`.
pub fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into())
}
fn default_log_rotation() -> String {
    "daily".into()
}
fn default_log_max_files() -> u32 {
    7
}
fn default_log_format() -> String {
    "text".into()
}
fn default_history_limit() -> u32 {
    10
}
fn default_debounce_ms() -> u32 {
    500
}
fn default_suggest_limit() -> u32 {
    5
}
fn default_suggest_min_score() -> f32 {
    0.1
}
fn default_stale_days() -> u32 {
    90
}
fn default_stale_confidence_threshold() -> f32 {
    0.4
}
fn default_wiki_root() -> String {
    "wiki".to_string()
}
// ── Functions ─────────────────────────────────────────────────────────────────

/// Merge global and per-wiki config into a `ResolvedConfig` for a specific wiki.
pub fn resolve(global: &GlobalConfig, per_wiki: &WikiConfig) -> ResolvedConfig {
    ResolvedConfig {
        defaults: per_wiki
            .defaults
            .clone()
            .unwrap_or_else(|| global.defaults.clone()),
        read: per_wiki.read.clone().unwrap_or_else(|| global.read.clone()),
        index: global.index.clone(),
        graph: per_wiki
            .graph
            .clone()
            .unwrap_or_else(|| global.graph.clone()),
        serve: global.serve.clone(),
        ingest: per_wiki
            .ingest
            .clone()
            .unwrap_or_else(|| global.ingest.clone()),
        validation: per_wiki
            .validation
            .clone()
            .unwrap_or_else(|| global.validation.clone()),
        history: per_wiki
            .history
            .clone()
            .unwrap_or_else(|| global.history.clone()),
        suggest: per_wiki
            .suggest
            .clone()
            .unwrap_or_else(|| global.suggest.clone()),
        search: {
            let mut merged = global.search.status.clone();
            if let Some(wiki_search) = &per_wiki.search {
                for (k, v) in &wiki_search.status {
                    merged.insert(k.clone(), *v);
                }
            }
            SearchConfig { status: merged }
        },
        lint: per_wiki.lint.clone().unwrap_or_else(|| global.lint.clone()),
        redact: per_wiki
            .redact
            .clone()
            .unwrap_or_else(|| global.redact.clone()),
    }
}

/// Load the global config from a TOML file. Returns default config if the file is absent.
pub fn load_global(path: &Path) -> Result<GlobalConfig> {
    let mut config: GlobalConfig = if !path.exists() {
        GlobalConfig::default()
    } else {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?
    };

    // Watcher env overrides (watcher bind-mount reliability, 2026-07-25).
    // Env wins over config file, mirroring the [provider] env-var indirection
    // pattern. Operators on Docker Desktop can force `poll` without editing
    // the (often read-only) bind-mounted config file.
    if let Ok(val) = std::env::var("LLM_WIKI_WATCH_BACKEND") {
        match val.trim().to_ascii_lowercase().as_str() {
            "auto" => config.watch.backend = WatchBackendConfig::Auto,
            "native" => config.watch.backend = WatchBackendConfig::Native,
            "poll" => config.watch.backend = WatchBackendConfig::Poll,
            other => {
                tracing::warn!(
                    var = %other,
                    "LLM_WIKI_WATCH_BACKEND must be auto|native|poll; ignoring"
                );
            }
        }
    }
    if let Ok(val) = std::env::var("LLM_WIKI_WATCH_POLL_MS") {
        match val.trim().parse::<u32>() {
            Ok(ms) if ms >= 100 => config.watch.poll_interval_ms = ms,
            Ok(ms) => tracing::warn!(ms, "LLM_WIKI_WATCH_POLL_MS must be >= 100; ignoring"),
            Err(_) => tracing::warn!(
                raw = %val,
                "LLM_WIKI_WATCH_POLL_MS is not a valid u32; ignoring"
            ),
        }
    }

    Ok(config)
}

/// Load per-wiki config from `<wiki_root>/wiki.toml`. Returns default config if absent.
pub fn load_wiki(wiki_root: &Path) -> Result<WikiConfig> {
    let path = wiki_root.join("wiki.toml");
    if !path.exists() {
        return Ok(WikiConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: WikiConfig =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}

/// Serialize and write the global config to `path`, creating parent dirs if needed.
pub fn save_global(config: &GlobalConfig, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Serialize and write the per-wiki config to `<wiki_root>/wiki.toml`.
pub fn save_wiki(config: &WikiConfig, wiki_root: &Path) -> Result<()> {
    let path = wiki_root.join("wiki.toml");
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Set a dot-notation config key on a `GlobalConfig` in place. Errors on unknown keys.
pub fn set_global_config_value(global: &mut GlobalConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "global.default_wiki" => global.global.default_wiki = value.into(),
        "defaults.search_top_k" => global.defaults.search_top_k = value.parse()?,
        "defaults.search_excerpt" => global.defaults.search_excerpt = value.parse()?,
        "defaults.search_sections" => global.defaults.search_sections = value.parse()?,
        "defaults.page_mode" => global.defaults.page_mode = value.into(),
        "defaults.list_page_size" => global.defaults.list_page_size = value.parse()?,
        "defaults.output_format" => global.defaults.output_format = value.into(),
        "defaults.facets_top_tags" => global.defaults.facets_top_tags = value.parse()?,
        "read.no_frontmatter" => global.read.no_frontmatter = value.parse()?,
        "index.auto_rebuild" => global.index.auto_rebuild = value.parse()?,
        "index.auto_recovery" => global.index.auto_recovery = value.parse()?,
        "index.memory_budget_mb" => global.index.memory_budget_mb = value.parse()?,
        "index.tokenizer" => global.index.tokenizer = value.into(),
        "graph.format" => global.graph.format = value.into(),
        "graph.depth" => global.graph.depth = value.parse()?,
        "graph.output" => global.graph.output = value.into(),
        "graph.snapshot" => global.graph.snapshot = value.parse()?,
        "graph.snapshot_keep" => global.graph.snapshot_keep = value.parse()?,
        "graph.snapshot_format" => global.graph.snapshot_format = value.into(),
        "graph.structural_algorithms" => global.graph.structural_algorithms = value.parse()?,
        "graph.max_nodes_for_diameter" => global.graph.max_nodes_for_diameter = value.parse()?,
        "serve.http" => global.serve.http = value.parse()?,
        "serve.http_port" => global.serve.http_port = value.parse()?,
        "serve.http_allowed_hosts" => {
            global.serve.http_allowed_hosts =
                value.split(',').map(|s| s.trim().to_string()).collect();
        }
        "serve.acp" => global.serve.acp = value.parse()?,
        "serve.max_restarts" => global.serve.max_restarts = value.parse()?,
        "serve.restart_backoff" => global.serve.restart_backoff = value.parse()?,
        "serve.heartbeat_secs" => global.serve.heartbeat_secs = value.parse()?,
        "serve.acp_max_sessions" => global.serve.acp_max_sessions = value.parse()?,
        "serve.acp_session_ttl_secs" => global.serve.acp_session_ttl_secs = value.parse()?,
        "serve.mcp_session_keep_alive_secs" => {
            global.serve.mcp_session_keep_alive_secs = value.parse()?;
        }
        "serve.mcp_init_timeout_secs" => {
            global.serve.mcp_init_timeout_secs = value.parse()?;
        }
        "serve.mcp_completed_cache_ttl_secs" => {
            global.serve.mcp_completed_cache_ttl_secs = value.parse()?;
        }
        "serve.mcp_tool_call_timeout_secs" => {
            global.serve.mcp_tool_call_timeout_secs = value.parse()?;
        }
        "serve.mcp_stateful_mode" => global.serve.mcp_stateful_mode = value.parse()?,
        "serve.mcp_json_response" => global.serve.mcp_json_response = value.parse()?,
        "serve.ingest_max_source_bytes" => {
            global.serve.ingest_max_source_bytes = value.parse()?;
        }
        "serve.ingest_max_sources_per_minute" => {
            global.serve.ingest_max_sources_per_minute = value.parse()?;
        }
        "ingest.auto_commit" => global.ingest.auto_commit = value.parse()?,
        "history.follow" => global.history.follow = value.parse()?,
        "history.default_limit" => global.history.default_limit = value.parse()?,
        "suggest.default_limit" => global.suggest.default_limit = value.parse()?,
        "suggest.min_score" => global.suggest.min_score = value.parse()?,
        "validation.type_strictness" => global.validation.type_strictness = value.into(),
        "logging.log_path" => global.logging.log_path = value.into(),
        "logging.log_rotation" => global.logging.log_rotation = value.into(),
        "logging.log_max_files" => global.logging.log_max_files = value.parse()?,
        "logging.log_format" => global.logging.log_format = value.into(),
        "watch.debounce_ms" => global.watch.debounce_ms = value.parse()?,
        "provider.enabled" => global.provider.enabled = value.parse()?,
        "provider.base_url" => global.provider.base_url = value.into(),
        "provider.api_key_env" => global.provider.api_key_env = value.into(),
        "provider.routine_model" => global.provider.routine_model = value.into(),
        "provider.reasoning_model" => global.provider.reasoning_model = value.into(),
        "provider.extraction_max_tokens" => {
            global.provider.extraction_max_tokens = value.parse()?;
        }
        "provider.timeout_secs" => {
            global.provider.timeout_secs = value.parse()?;
        }
        "provider.compliance_user_decision" => {
            global.provider.compliance_user_decision = value.into();
        }
        "provider.compliance_known_terms_risk" => {
            global.provider.compliance_known_terms_risk = value.into();
        }
        "provider.compliance_retention_terms" => {
            global.provider.compliance_retention_terms = value.into();
        }
        "provider.compliance_training_terms" => {
            global.provider.compliance_training_terms = value.into();
        }
        "provider.compliance_processing_region" => {
            global.provider.compliance_processing_region = value.into();
        }
        _ => anyhow::bail!("unknown key: {key}"),
    }
    Ok(())
}

/// Read a dot-notation config key from `ResolvedConfig`/`GlobalConfig`. Returns `"unknown key"` for unrecognized keys.
pub fn get_config_value(resolved: &ResolvedConfig, global: &GlobalConfig, key: &str) -> String {
    match key {
        "global.default_wiki" => global.global.default_wiki.clone(),
        "defaults.search_top_k" => resolved.defaults.search_top_k.to_string(),
        "defaults.search_excerpt" => resolved.defaults.search_excerpt.to_string(),
        "defaults.search_sections" => resolved.defaults.search_sections.to_string(),
        "defaults.page_mode" => resolved.defaults.page_mode.clone(),
        "defaults.list_page_size" => resolved.defaults.list_page_size.to_string(),
        "defaults.output_format" => resolved.defaults.output_format.clone(),
        "defaults.facets_top_tags" => resolved.defaults.facets_top_tags.to_string(),
        "read.no_frontmatter" => resolved.read.no_frontmatter.to_string(),
        "index.auto_rebuild" => resolved.index.auto_rebuild.to_string(),
        "index.auto_recovery" => global.index.auto_recovery.to_string(),
        "index.memory_budget_mb" => global.index.memory_budget_mb.to_string(),
        "index.tokenizer" => global.index.tokenizer.clone(),
        "graph.format" => resolved.graph.format.clone(),
        "graph.depth" => resolved.graph.depth.to_string(),
        "graph.output" => resolved.graph.output.clone(),
        "graph.snapshot" => resolved.graph.snapshot.to_string(),
        "graph.snapshot_keep" => resolved.graph.snapshot_keep.to_string(),
        "graph.snapshot_format" => resolved.graph.snapshot_format.clone(),
        "graph.structural_algorithms" => resolved.graph.structural_algorithms.to_string(),
        "graph.max_nodes_for_diameter" => resolved.graph.max_nodes_for_diameter.to_string(),
        "serve.http" => resolved.serve.http.to_string(),
        "serve.http_port" => resolved.serve.http_port.to_string(),
        "serve.http_allowed_hosts" => resolved.serve.http_allowed_hosts.join(","),
        "serve.acp" => resolved.serve.acp.to_string(),
        "serve.max_restarts" => global.serve.max_restarts.to_string(),
        "serve.restart_backoff" => global.serve.restart_backoff.to_string(),
        "serve.heartbeat_secs" => global.serve.heartbeat_secs.to_string(),
        "serve.acp_max_sessions" => global.serve.acp_max_sessions.to_string(),
        "serve.acp_session_ttl_secs" => global.serve.acp_session_ttl_secs.to_string(),
        "serve.mcp_session_keep_alive_secs" => global.serve.mcp_session_keep_alive_secs.to_string(),
        "serve.mcp_init_timeout_secs" => global.serve.mcp_init_timeout_secs.to_string(),
        "serve.mcp_completed_cache_ttl_secs" => {
            global.serve.mcp_completed_cache_ttl_secs.to_string()
        }
        "serve.mcp_tool_call_timeout_secs" => global.serve.mcp_tool_call_timeout_secs.to_string(),
        "serve.mcp_stateful_mode" => global.serve.mcp_stateful_mode.to_string(),
        "serve.mcp_json_response" => global.serve.mcp_json_response.to_string(),
        "serve.ingest_max_source_bytes" => resolved.serve.ingest_max_source_bytes.to_string(),
        "serve.ingest_max_sources_per_minute" => {
            resolved.serve.ingest_max_sources_per_minute.to_string()
        }
        "validation.type_strictness" => resolved.validation.type_strictness.clone(),
        "logging.log_path" => global.logging.log_path.clone(),
        "logging.log_rotation" => global.logging.log_rotation.clone(),
        "logging.log_max_files" => global.logging.log_max_files.to_string(),
        "logging.log_format" => global.logging.log_format.clone(),
        "watch.debounce_ms" => global.watch.debounce_ms.to_string(),
        "ingest.auto_commit" => resolved.ingest.auto_commit.to_string(),
        "history.follow" => resolved.history.follow.to_string(),
        "history.default_limit" => resolved.history.default_limit.to_string(),
        "suggest.default_limit" => resolved.suggest.default_limit.to_string(),
        "suggest.min_score" => resolved.suggest.min_score.to_string(),
        "provider.enabled" => global.provider.enabled.to_string(),
        "provider.base_url" => global.provider.base_url.clone(),
        "provider.api_key_env" => global.provider.api_key_env.clone(),
        "provider.routine_model" => global.provider.routine_model.clone(),
        "provider.reasoning_model" => global.provider.reasoning_model.clone(),
        "provider.extraction_max_tokens" => global.provider.extraction_max_tokens.to_string(),
        "provider.timeout_secs" => global.provider.timeout_secs.to_string(),
        "provider.compliance_user_decision" => global.provider.compliance_user_decision.clone(),
        "provider.compliance_known_terms_risk" => {
            global.provider.compliance_known_terms_risk.clone()
        }
        "provider.compliance_retention_terms" => global.provider.compliance_retention_terms.clone(),
        "provider.compliance_training_terms" => global.provider.compliance_training_terms.clone(),
        "provider.compliance_processing_region" => {
            global.provider.compliance_processing_region.clone()
        }
        _ => format!("unknown key: {key}"),
    }
}

/// Set a dot-notation config key on a `WikiConfig` in place. Errors on global-only or unknown keys.
pub fn set_wiki_config_value(wiki_cfg: &mut WikiConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "defaults.search_top_k" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .search_top_k = value.parse()?;
        }
        "defaults.search_excerpt" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .search_excerpt = value.parse()?;
        }
        "defaults.search_sections" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .search_sections = value.parse()?;
        }
        "defaults.page_mode" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .page_mode = value.into();
        }
        "defaults.list_page_size" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .list_page_size = value.parse()?;
        }
        "defaults.output_format" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .output_format = value.into();
        }
        "defaults.facets_top_tags" => {
            wiki_cfg
                .defaults
                .get_or_insert_with(Defaults::default)
                .facets_top_tags = value.parse()?;
        }
        "read.no_frontmatter" => {
            wiki_cfg
                .read
                .get_or_insert_with(ReadConfig::default)
                .no_frontmatter = value.parse()?;
        }
        "validation.type_strictness" => {
            wiki_cfg
                .validation
                .get_or_insert_with(ValidationConfig::default)
                .type_strictness = value.into();
        }
        "ingest.auto_commit" => {
            wiki_cfg
                .ingest
                .get_or_insert_with(IngestConfig::default)
                .auto_commit = value.parse()?;
        }
        "history.follow" => {
            wiki_cfg
                .history
                .get_or_insert_with(HistoryConfig::default)
                .follow = value.parse()?;
        }
        "history.default_limit" => {
            wiki_cfg
                .history
                .get_or_insert_with(HistoryConfig::default)
                .default_limit = value.parse()?;
        }
        "suggest.default_limit" => {
            wiki_cfg
                .suggest
                .get_or_insert_with(SuggestConfig::default)
                .default_limit = value.parse()?;
        }
        "suggest.min_score" => {
            wiki_cfg
                .suggest
                .get_or_insert_with(SuggestConfig::default)
                .min_score = value.parse()?;
        }
        "graph.format" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .format = value.into();
        }
        "graph.depth" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .depth = value.parse()?;
        }
        "graph.output" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .output = value.into();
        }
        "graph.snapshot" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .snapshot = value.parse()?;
        }
        "graph.snapshot_keep" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .snapshot_keep = value.parse()?;
        }
        "graph.snapshot_format" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .snapshot_format = value.into();
        }
        "graph.structural_algorithms" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .structural_algorithms = value.parse()?;
        }
        "graph.max_nodes_for_diameter" => {
            wiki_cfg
                .graph
                .get_or_insert_with(GraphConfig::default)
                .max_nodes_for_diameter = value.parse()?;
        }
        "global.default_wiki"
        | "index.auto_rebuild"
        | "index.auto_recovery"
        | "index.memory_budget_mb"
        | "index.tokenizer"
        | "serve.http"
        | "serve.http_port"
        | "serve.http_allowed_hosts"
        | "serve.acp"
        | "serve.max_restarts"
        | "serve.restart_backoff"
        | "serve.heartbeat_secs"
        | "serve.acp_max_sessions"
        | "serve.acp_session_ttl_secs"
        | "serve.mcp_session_keep_alive_secs"
        | "serve.mcp_init_timeout_secs"
        | "serve.mcp_completed_cache_ttl_secs"
        | "serve.mcp_tool_call_timeout_secs"
        | "serve.mcp_stateful_mode"
        | "serve.mcp_json_response"
        | "serve.ingest_max_source_bytes"
        | "serve.ingest_max_sources_per_minute"
        | "logging.log_path"
        | "logging.log_rotation"
        | "logging.log_max_files"
        | "logging.log_format" => {
            anyhow::bail!("{key} is a global-only key \u{2014} use --global");
        }
        "watch.debounce_ms" => {
            anyhow::bail!("{key} is a global-only key \u{2014} use --global");
        }
        _ => anyhow::bail!("unknown key: {key}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: the `unsafe` blocks around env mutation are required by Rust 2024
    // edition — `set_var`/`remove_var` are unsafe because concurrent reads of
    // the env from another thread can race. These tests use unique-per-test
    // env var names so no other thread reads them; see the same pattern in
    // tests/secret_file_v1.rs.

    #[test]
    fn resolve_mcp_tokens_parses_comma_separated() {
        // SAFETY: TEST_MCP_TOKENS is unique to this test; no other thread reads it.
        unsafe { std::env::set_var("TEST_MCP_TOKENS", "tok-a, tok-b ,, tok-c") };
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS".into();
        cfg.http_bind_all_interfaces = false;
        let tokens = cfg.resolve_mcp_tokens().unwrap();
        assert_eq!(
            tokens,
            vec!["tok-a".to_string(), "tok-b".to_string(), "tok-c".to_string()]
        );
        // SAFETY: see above.
        unsafe { std::env::remove_var("TEST_MCP_TOKENS") };
    }

    #[test]
    fn resolve_mcp_tokens_fail_closed_when_public_and_empty() {
        // SAFETY: TEST_MCP_TOKENS_EMPTY is unique to this test.
        unsafe { std::env::remove_var("TEST_MCP_TOKENS_EMPTY") };
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS_EMPTY".into();
        cfg.http_bind_all_interfaces = true; // public bind + no tokens = refuse
        let err = cfg.resolve_mcp_tokens();
        assert!(err.is_err(), "public bind with no tokens must fail-closed");
    }

    #[test]
    fn resolve_mcp_tokens_loopback_allows_empty() {
        // SAFETY: TEST_MCP_TOKENS_EMPTY2 is unique to this test.
        unsafe { std::env::remove_var("TEST_MCP_TOKENS_EMPTY2") };
        let mut cfg = ServeConfig::default();
        cfg.mcp_bearer_tokens_env = "TEST_MCP_TOKENS_EMPTY2".into();
        cfg.http_bind_all_interfaces = false; // loopback + no tokens = backward compat
        let tokens = cfg.resolve_mcp_tokens().unwrap();
        assert!(
            tokens.is_empty(),
            "loopback with no tokens must return empty (no auth)"
        );
    }
}
