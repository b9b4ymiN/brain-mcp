//! Read-only projection of `GlobalConfig` for the `/config` Console endpoint.
//!
//! Phase 1 (2026-07-25 Console expansion): projects a safe subset of fields
//! for the Config page's read-only display. `GlobalConfig` stores secrets as
//! ENV VAR NAMES (`provider.api_key_env`, `serve.console_dev_bootstrap_*_env`)
//! rather than raw values, so the projection is naturally safe — but we still
//! write an explicit projection (rather than serializing `GlobalConfig`
//! directly) so future additions don't leak to the wire without an explicit
//! decision. The regression test in `tests/ops/config_view.rs` is the
//! trip-wire.

use serde::Serialize;

use crate::config::GlobalConfig;

/// Placeholder for any future field that needs masking. Currently unused —
/// kept here so the masking convention is obvious to anyone adding a new
/// secret-bearing field to `GlobalConfig`.
#[allow(dead_code)]
const MASKED: &str = "****";

/// Top-level Config view returned by `GET /api/v1/config`.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView {
    /// Registered wiki spaces (`[[wikis]]` in `config.toml`).
    pub wiki_spaces: Vec<WikiSpaceView>,
    /// `[serve]` section projection.
    pub server: ServerView,
    /// `[provider]` section projection. Renamed so the Config page can show
    /// it as "Extraction" without confusion (the on-disk TOML key stays
    /// `[provider]`).
    pub extraction: ProviderView,
    /// `[logging]` section projection.
    pub logging: LoggingView,
    /// `[index]` section projection.
    pub index: IndexView,
}

/// A single wiki space row.
#[derive(Debug, Clone, Serialize)]
pub struct WikiSpaceView {
    pub name: String,
    /// Filesystem path, stringified for the wire.
    pub path: String,
    /// Optional description, surfaced when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional git remote URL, surfaced when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// Server/transport section. No resolved secret values are included — only
/// the env-var NAMES that hold them, which are safe to display.
#[derive(Debug, Clone, Serialize)]
pub struct ServerView {
    pub http_enabled: bool,
    pub http_port: u16,
    pub bind: String,
    pub bind_all_interfaces: bool,
    pub acp_enabled: bool,
    /// Env var NAME holding the Console dev USERNAME (not the value).
    pub bootstrap_username_env: Option<String>,
    /// Env var NAME holding the Console dev PASSWORD (not the value).
    pub bootstrap_password_env: Option<String>,
}

/// AI provider ("Extraction") section.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub enabled: bool,
    pub base_url: String,
    /// Env var NAME (e.g. `OPENAI_API_KEY`) — safe, not the value.
    pub api_key_env: String,
    pub routine_model: String,
    pub reasoning_model: String,
}

/// Logging section.
#[derive(Debug, Clone, Serialize)]
pub struct LoggingView {
    /// `"text"` or `"json"`.
    pub format: String,
    /// `"daily"` or `"never"`.
    pub rotation: String,
}

/// Index section.
#[derive(Debug, Clone, Serialize)]
pub struct IndexView {
    pub tokenizer: String,
    pub auto_rebuild: bool,
}

/// Project `GlobalConfig` → `ConfigView`.
///
/// Walks the config struct and emits ONLY the fields listed above. Adding a
/// new field to `GlobalConfig` does NOT automatically expose it — the
/// projection must be updated explicitly. This is the whole point of having
/// a projection rather than serializing `GlobalConfig` directly.
pub fn config_view(config: &GlobalConfig) -> ConfigView {
    ConfigView {
        wiki_spaces: config
            .wikis
            .iter()
            .map(|w| WikiSpaceView {
                name: w.name.clone(),
                path: w.path.clone(),
                description: w.description.clone(),
                remote: w.remote.clone(),
            })
            .collect(),
        server: ServerView {
            http_enabled: config.serve.http,
            http_port: config.serve.http_port,
            bind: config.serve.http_bind_address.clone(),
            bind_all_interfaces: config.serve.http_bind_all_interfaces,
            acp_enabled: config.serve.acp,
            // Both `_env` fields are strings in `GlobalConfig` (default to
            // "USERNAME" / "PASSWORD"). Surface them as `Option<String>` so
            // the frontend can treat `None` as "not customized", and skip
            // the field entirely on the wire when not customized.
            bootstrap_username_env: customized_env(&config.serve.console_dev_bootstrap_username_env),
            bootstrap_password_env: customized_env(&config.serve.console_dev_bootstrap_password_env),
        },
        extraction: ProviderView {
            enabled: config.provider.enabled,
            base_url: config.provider.base_url.clone(),
            api_key_env: config.provider.api_key_env.clone(),
            routine_model: config.provider.routine_model.clone(),
            reasoning_model: config.provider.reasoning_model.clone(),
        },
        logging: LoggingView {
            format: config.logging.log_format.clone(),
            rotation: config.logging.log_rotation.clone(),
        },
        index: IndexView {
            tokenizer: config.index.tokenizer.clone(),
            auto_rebuild: config.index.auto_rebuild,
        },
    }
}

/// Return `Some(name)` only when the env-var name differs from its default.
/// This keeps the wire response clean (no `null`s for un-customized fields)
/// while still surfacing the name when an operator has customized it.
fn customized_env(name: &str) -> Option<String> {
    match name {
        "" | "USERNAME" | "PASSWORD" => None,
        other => Some(other.to_owned()),
    }
}
