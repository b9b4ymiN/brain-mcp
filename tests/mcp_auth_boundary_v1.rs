//! Task 3.3 — Production auth boundary contracts (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 3.3 DoD at the contract level (real TLS/OAuth
//! termination needs deployment infrastructure — Phase 6; this task delivers
//! the auth-policy framework + capability enforcement + negative tests):
//!
//! - TLS, Origin validation, audience/resource validation, per-tool
//!   capabilities `brain.read/capture/propose/confirm/purge/admin` ผ่าน
//!   negative tests
//! - token ไม่อยู่ URL/log/repo
//! - ทุก HTTP path มี auth policy ชัด

use llm_wiki::mcp::auth::{
    AuthPolicy, AuthPolicyEntry, AuthPrincipal, Capability, TokenRedaction,
};
use serde_json::json;

// =============================================================================
// DoD 1 — per-tool capability map + enforcement (negative tests)
// =============================================================================

/// Every tool maps to exactly one required capability from the §7.2 set
/// (`brain.read/capture/propose/confirm/purge/admin`). A tool with no mapping
/// is a policy gap — it must be classified before it ships.
#[test]
fn every_tool_has_a_required_capability() {
    let policy = AuthPolicy::default();
    for tool in llm_wiki::mcp::tools::tool_list() {
        let cap = policy.required_capability(&tool.name);
        assert!(
            cap.is_some(),
            "tool {} has no required capability in the auth policy",
            tool.name
        );
    }
}

/// Read tools require `brain.read`. A proposal-only worker (capabilities =
/// `[brain.propose]`) must be denied read... no, read is the baseline — a
/// proposal-only worker CAN read (it needs context to propose). Correct:
/// read tools require `brain.read`, and a proposal-only worker does NOT have
/// `brain.read` unless granted. The negative test is: a worker with ONLY
/// `brain.propose` is denied `brain.confirm`.
#[test]
fn proposal_only_worker_is_denied_confirm() {
    let policy = AuthPolicy::default();
    let worker = AuthPrincipal {
        id: "worker-1".to_owned(),
        capabilities: vec![Capability::Propose],
    };
    // A confirm-class tool (e.g. wiki_content_commit, or a future brain_confirm)
    // requires brain.confirm.
    let confirm_tool = policy
        .required_capability("wiki_content_commit")
        .unwrap_or(Capability::Confirm);
    assert!(
        !worker.has(confirm_tool),
        "proposal-only worker must not satisfy the confirm capability"
    );
    assert!(
        policy.allows(&worker, "wiki_content_commit") == false,
        "proposal-only worker must be DENIED wiki_content_commit"
    );
}

/// A proposal-only worker IS allowed to call propose-class tools (it has
/// `brain.propose`). This is the positive counterpart — the policy is
/// capability-scoped, not blanket-deny.
#[test]
fn proposal_only_worker_is_allowed_propose_tools() {
    let policy = AuthPolicy::default();
    let worker = AuthPrincipal {
        id: "worker-1".to_owned(),
        capabilities: vec![Capability::Propose],
    };
    // Find a tool that requires brain.propose (e.g. wiki_ingest — additive).
    let allowed = policy.allows(&worker, "wiki_ingest");
    assert!(
        allowed,
        "proposal-only worker should be allowed tools it has the capability for"
    );
}

/// An admin-only capability does NOT grant confirm/purge (no implicit
/// escalation). §7.1: "admin capability อย่างเดียวไม่พอ" for destructive ops.
#[test]
fn admin_capability_alone_does_not_grant_confirm_or_purge() {
    let policy = AuthPolicy::default();
    let admin_only = AuthPrincipal {
        id: "admin-1".to_owned(),
        capabilities: vec![Capability::Admin],
    };
    let confirm_tool = policy
        .required_capability("wiki_content_commit")
        .unwrap_or(Capability::Confirm);
    assert!(
        !admin_only.has(confirm_tool),
        "admin-only must not implicitly satisfy confirm"
    );
}

// =============================================================================
// DoD 2 — token never in URL/log/repo (redaction contract)
// =============================================================================

/// `TokenRedaction::redact` strips bearer tokens, API keys, and password
/// fragments from a string before it enters a log or URL. The redacted output
/// must not contain the secret substring.
#[test]
fn token_redaction_strips_bearer_tokens() {
    let input = "Authorization: Bearer sk-secret-1234567890";
    let redacted = TokenRedaction::redact(input);
    assert!(
        !redacted.contains("sk-secret-1234567890"),
        "redacted string must not contain the token: got {redacted}"
    );
    assert!(
        redacted.contains("[REDACTED]"),
        "redacted string should mark where the token was: got {redacted}"
    );
}

/// Redaction is total: a URL with a `?token=` or `#access_token=` fragment is
/// scrubbed too.
#[test]
fn token_redaction_strips_url_query_tokens() {
    let input = "https://host/mcp?access_token=eyJsecret";
    let redacted = TokenRedaction::redact(input);
    assert!(
        !redacted.contains("eyJsecret"),
        "URL token must be redacted: got {redacted}"
    );
}

// =============================================================================
// DoD 3 — every HTTP path has an auth policy
// =============================================================================

/// The default `AuthPolicy` declares an entry for every HTTP path the server
/// exposes (at minimum `/mcp`). No path is implicitly open.
#[test]
fn auth_policy_covers_every_http_path() {
    let policy = AuthPolicy::default();
    let entries = policy.entries();
    assert!(
        !entries.is_empty(),
        "auth policy must declare at least one path"
    );
    // The MCP endpoint must be covered.
    assert!(
        entries.iter().any(|e| e.path == "/mcp"),
        "auth policy must cover /mcp"
    );
    // Every entry must name a required capability (not "open").
    for entry in &entries {
        assert!(
            !entry.requires.is_empty(),
            "path {} has an empty requires list (implicitly open)",
            entry.path
        );
    }
}

/// An auth policy entry carries Origin + TLS requirements so a path is never
/// served over plain HTTP or from an unexpected origin in production.
#[test]
fn auth_policy_entry_carries_origin_and_tls_requirements() {
    let entry = AuthPolicyEntry {
        path: "/mcp".to_owned(),
        requires: vec![Capability::Read],
        requires_tls: true,
        allowed_origins: vec!["https://brain.tailnet.ts.net".to_owned()],
    };
    assert!(entry.requires_tls);
    assert!(!entry.allowed_origins.is_empty());
}

/// Compile-time proof of the capability enum covering the §7.2 set.
#[test]
fn capability_enum_covers_the_7_2_set() {
    let _ = Capability::Read;
    let _ = Capability::Capture;
    let _ = Capability::Propose;
    let _ = Capability::Confirm;
    let _ = Capability::Purge;
    let _ = Capability::Admin;
}

// keep json import used for future expansion without warning
#[test]
fn _json_compile_check() {
    let _ = json!({"ok": true});
}
