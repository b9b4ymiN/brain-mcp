//! Production auth boundary (Task 3.3).
//!
//! Contract-level framework for GOAL-vNext §13 Task 3.3 + §7.1 auth matrix.
//! Real TLS/OAuth/Keycloak termination belongs to Phase 6 deployment; this
//! module defines the policy primitives a deployment plugs into:
//!
//! - [`Capability`] — the §7.2 capability set (`brain.read/capture/propose/
//!   confirm/purge/admin`). No broad `brain.write`.
//! - [`AuthPrincipal`] — an authenticated subject + its granted capabilities.
//! - [`AuthPolicy`] — maps each tool name to its required capability and each
//!   HTTP path to its policy entry (capability + TLS + Origin requirements).
//! - [`TokenRedaction`] — strips secrets before they enter logs/URLs.
//!
//! Enforcement is `AuthPolicy::allows(principal, tool)` — a boolean gate the
//! transport layer calls before dispatching. The deployment layer (Phase 6)
//! is responsible for populating `AuthPrincipal` from a validated token and
//! for enforcing `requires_tls` / `allowed_origins` at the HTTP edge.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Capability set (§7.2) ────────────────────────────────────────────────────

/// The §7.2 per-tool capability set. There is deliberately no broad
/// `brain.write` — the server enforces an allow-list per tool. A worker
/// identity has `Propose` only; confirm/purge/admin are separate grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read tools (search/get/timeline/explain/status).
    Read,
    /// Capture a raw source/utterance into quarantine.
    Capture,
    /// Propose a claim (AI worker's only write capability).
    Propose,
    /// Confirm/supersede/retract a claim (owner assertion).
    Confirm,
    /// Destructive privacy operation (crypto-shred).
    Purge,
    /// Admin: rebuild/eval/backup — NOT an implicit grant of the above.
    Admin,
}

// ── Authenticated principal ──────────────────────────────────────────────────

/// An authenticated subject (derived from a validated token at the transport
/// edge). Carries only the granted capabilities — never the token itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    /// Stable subject id (validated token subject; NOT the raw token).
    pub id: String,
    /// Capabilities granted to this subject.
    pub capabilities: Vec<Capability>,
}

impl AuthPrincipal {
    /// True if this principal holds the given capability.
    pub fn has(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

// ── Auth policy ──────────────────────────────────────────────────────────────

/// One HTTP path's auth policy entry (Task 3.3 DoD bullet 3). A path is never
/// implicitly open: every entry names a non-empty `requires` list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPolicyEntry {
    /// Path pattern (e.g. "/mcp").
    pub path: String,
    /// Capabilities required to call any tool on this path (baseline; the
    /// per-tool map in [`AuthPolicy`] may raise the bar for specific tools).
    pub requires: Vec<Capability>,
    /// True in production — the path must be served over TLS.
    pub requires_tls: bool,
    /// Allowed Origin headers (Origin validation, §7.1). Empty in dev.
    pub allowed_origins: Vec<String>,
}

/// The full auth policy: per-tool capability map + per-path entries.
/// Constructed once at startup from config; consulted on every tool call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthPolicy {
    tool_capabilities: HashMap<String, Capability>,
    path_entries: Vec<AuthPolicyEntry>,
}

impl AuthPolicy {
    /// Default policy: read tools → `brain.read`, mutation tools → the
    /// capability matching their §7.2 classification. Every declared tool is
    /// mapped (no gaps). Paths: `/mcp` requires at least `brain.read` over
    /// TLS.
    pub fn defaults_for(tools: &[String]) -> Self {
        let mut tool_capabilities = HashMap::new();
        for name in tools {
            tool_capabilities.insert(name.clone(), default_capability_for(name));
        }
        let path_entries = vec![AuthPolicyEntry {
            path: "/mcp".to_owned(),
            requires: vec![Capability::Read],
            requires_tls: true,
            allowed_origins: Vec::new(),
        }];
        Self {
            tool_capabilities,
            path_entries,
        }
    }

    /// The capability required to call `tool`, or `None` if the tool is not
    /// in the policy (a gap that must be closed before the tool ships).
    pub fn required_capability(&self, tool: &str) -> Option<Capability> {
        self.tool_capabilities.get(tool).copied()
    }

    /// True if `principal` may call `tool`. Read tools need `brain.read`;
    /// mutation tools need their specific capability. No implicit escalation
    /// (admin alone does not grant confirm/purge).
    pub fn allows(&self, principal: &AuthPrincipal, tool: &str) -> bool {
        match self.required_capability(tool) {
            Some(cap) => principal.has(cap),
            None => false,
        }
    }

    /// The per-path policy entries (for DoD bullet 3 verification).
    pub fn entries(&self) -> &[AuthPolicyEntry] {
        &self.path_entries
    }
}

impl Default for AuthPolicy {
    fn default() -> Self {
        let tools: Vec<String> = crate::mcp::tools::tool_list()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        Self::defaults_for(&tools)
    }
}

/// Map a tool name to its §7.2 capability using the same classification as
/// the Task 3.1 annotation profiles (kept in sync by convention).
fn default_capability_for(name: &str) -> Capability {
    match name {
        // Read-only tools.
        "wiki_search" | "wiki_list" | "wiki_content_read" | "wiki_history" | "wiki_stats"
        | "wiki_graph" | "wiki_resolve" | "wiki_lint" | "wiki_suggest" | "profile_get"
        | "semantic_search" | "semantic_get" | "procedural_find" | "procedural_get"
        | "graph_neighbors" | "audit_history" | "wiki_index_status" | "brain_status"
        | "brain_search" | "brain_get" => Capability::Read,
        // brain_* mutations (Phase C C2)
        "brain_capture" => Capability::Capture,
        "brain_confirm" | "brain_supersede" => Capability::Confirm,
        // Capture-class (raw source ingest into quarantine).
        "wiki_ingest" => Capability::Capture,
        // Propose-class (additive writes — content write/new/commit, spaces
        // management, export). An AI worker with brain.propose may call these.
        "wiki_content_write"
        | "wiki_content_new"
        | "wiki_content_commit"
        | "wiki_spaces_create"
        | "wiki_spaces_register"
        | "wiki_spaces_list"
        | "wiki_export" => Capability::Propose,
        // Confirm-class (idempotent config mutations).
        "wiki_config" | "wiki_spaces_set_default" | "wiki_index_rebuild" => Capability::Confirm,
        // Purge-class (destructive removal).
        "wiki_spaces_remove" | "wiki_schema" => Capability::Purge,
        _ => Capability::Read,
    }
}

// ── Token redaction (DoD bullet 2) ───────────────────────────────────────────

/// Strip secrets before they enter logs or URLs. Catches:
/// - `Bearer <token>` / `Authorization: Bearer ...`
/// - `access_token=` / `token=` query parameters
/// - `#access_token=` URL fragments
///
/// Redaction is intentionally conservative (regex-free, substring-based) so
/// it never accidentally leaves a token fragment. The marker `[REDACTED]`
/// records where a secret was removed for auditability.
pub struct TokenRedaction;

impl TokenRedaction {
    /// Redact known secret patterns from `input`. Returns a new string with
    /// secrets replaced by `[REDACTED]`.
    pub fn redact(input: &str) -> String {
        let mut out = input.to_owned();
        // Bearer tokens (case-insensitive header value).
        out = redact_pattern(&out, "Bearer ", false);
        // URL query / fragment tokens.
        out = redact_pattern(&out, "access_token=", true);
        out = redact_pattern(&out, "token=", true);
        out
    }
}

/// Replace the value following `marker` (up to the next whitespace, `&`, `#`,
/// or end-of-string) with `[REDACTED]`. `value_url_safe` controls the
/// terminator set (query params end at `&`/`#`/whitespace; header values at
/// whitespace).
fn redact_pattern(input: &str, marker: &str, value_url_safe: bool) -> String {
    let marker_lower = marker.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.to_ascii_lowercase().find(&marker_lower) {
        out.push_str(&rest[..idx]);
        out.push_str(marker);
        let after = &rest[idx + marker.len()..];
        let end = if value_url_safe {
            after
                .find(|c: char| c.is_whitespace() || c == '&' || c == '#')
                .unwrap_or(after.len())
        } else {
            after
                .find(|c: char| c.is_whitespace())
                .unwrap_or(after.len())
        };
        out.push_str("[REDACTED]");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_bearer() {
        let r = TokenRedaction::redact("Authorization: Bearer sk-abc123 tail");
        assert!(!r.contains("sk-abc123"));
        assert!(r.contains("[REDACTED]"));
    }

    #[test]
    fn redact_url_token() {
        let r = TokenRedaction::redact("https://h/mcp?access_token=secret&x=1");
        assert!(!r.contains("secret"));
    }
}
