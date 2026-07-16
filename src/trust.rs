//! Trust + operations views contract (Task 5.3).
//!
//! Domain types for GOAL-vNext §13 Task 5.3 + §5.3 + §10. Surfaces:
//!
//! - [`TrustView`] + [`TrustFlag`] — contradictions/stale/orphan so the user
//!   sees contested/expired knowledge.
//! - [`RetrievalTrace`] — why a result was selected/excluded (auditability).
//! - [`ProvenanceQuestion`] + [`ProvenanceAnswer`] — the four core questions:
//!   what / source / when-true / connections (+ which client edited).
//! - [`JobSummary`] + [`BackupHealth`] — operations dashboard.
//! - [`DestructiveWarning`] — hard-purge preview + irreversible flag + nonce.

use serde::{Deserialize, Serialize};

// ── Trust view ───────────────────────────────────────────────────────────────

/// One trust flag on a claim or pair of claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrustFlag {
    /// Two claims in the same scope with conflicting values.
    Contradiction { claim_ids: Vec<String> },
    /// A claim not modified in a long time.
    Stale {
        claim_id: String,
        days_since_modified: u32,
    },
    /// A claim with no inbound links.
    Orphan { claim_id: String },
}

/// The trust surface: every flag the user needs to review.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustView {
    pub flags: Vec<TrustFlag>,
}

/// A retrieval trace: why a result was included or excluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalTrace {
    pub included_claim_ids: Vec<String>,
    pub excluded_claim_ids: Vec<String>,
    pub reason: String,
}

// ── Provenance — the four core questions ─────────────────────────────────────

/// The four question facets a user can ask about any subject. §5.3
/// "รู้อะไร/มาจากไหน/จริงเมื่อไร/เชื่อมอะไร".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceFacet {
    What,
    Source,
    When,
    Connections,
}

/// A provenance query about a subject.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceQuestion {
    pub subject: String,
}

impl ProvenanceQuestion {
    /// Construct a question about `subject`.
    pub fn about(subject: &str) -> Self {
        Self {
            subject: subject.to_owned(),
        }
    }

    // Constants for the four facets (used as compile-time proof of coverage).
    pub const WHAT: ProvenanceFacet = ProvenanceFacet::What;
    pub const SOURCE: ProvenanceFacet = ProvenanceFacet::Source;
    pub const WHEN: ProvenanceFacet = ProvenanceFacet::When;
    pub const CONNECTIONS: ProvenanceFacet = ProvenanceFacet::Connections;
}

/// The structured answer to the four provenance questions. §5.3 also requires
/// "client/channel ใดแก้" — captured in `client_that_edited`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceAnswer {
    /// What the system knows (predicate + value).
    pub what: String,
    /// Where it came from (source + evidence span).
    pub source: String,
    /// When it is/was true (valid time interval).
    pub when_true: String,
    /// What it connects to (related entities/sources).
    pub connections: String,
    /// Which client/channel last edited it (audit — NOT person inference, §5.3).
    pub client_that_edited: String,
}

// ── Operations view ──────────────────────────────────────────────────────────

/// Async-job queue summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub active: u32,
    pub queued: u32,
    pub failed: u32,
}

/// Backup health for the operations dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHealth {
    /// ISO-8601 UTC of the last successful backup.
    pub last_backup_at: String,
    /// True if the last clean-host restore drill passed.
    pub last_restore_drill_ok: bool,
}

/// Client/token activity audit for the operations dashboard (Task 5.3 F1).
/// §9.1 Operations item 7: "clients/tokens". Records which clients are
/// registered, their last activity, and token grant counts — NOT person
/// inference (TM-024).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientActivity {
    pub client_id: String,
    pub label: String,
    /// Capabilities granted to this client.
    pub capabilities: Vec<String>,
    /// ISO-8601 UTC of the client's last mutation.
    pub last_active_at: String,
    /// Total mutations this client has committed (audit counter).
    pub mutation_count: u64,
}

/// Eval health for the operations dashboard (Task 5.3 F1). §9.1 Operations
/// item 7: "evals". Summarizes the last domain-eval run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSummary {
    /// Total cases in the last run.
    pub case_count: u32,
    /// Cases that passed.
    pub passed: u32,
    /// True if the hard abstention invariant passed 100%.
    pub abstention_passed: bool,
    /// ISO-8601 UTC of the run.
    pub run_at: String,
}

// ── Destructive-action warning ───────────────────────────────────────────────

/// The destructive actions that require a warning + nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestructiveAction {
    HardPurge,
    EntityMerge,
    EntitySplit,
}

/// One item that a destructive action will affect (Task 5.3 F2). The preview
/// enumerates the exact targets so the user sees what will be lost/changed
/// before confirming — not just a warning string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestructivePreviewItem {
    /// The kind of target (claim, entity, object, etc.).
    pub target_kind: String,
    /// The target id.
    pub target_id: String,
    /// Human-readable description of the effect.
    pub effect: String,
}

/// A warning the UI MUST display before a destructive action. §5.3 "hard purge
/// มี preview, recent re-auth, two-step nonce และคำเตือนว่า irreversible โดยไม่มี
/// undo". `requires_recent_reauth` (F3) and `requires_two_step_nonce` are
/// SEPARATE controls: re-auth = authentication-freshness gate; nonce =
/// operation-confirmation token. `preview` (F2) enumerates the exact targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestructiveWarning {
    pub action: DestructiveAction,
    pub irreversible: bool,
    /// True if the user must have re-authenticated recently (freshness gate).
    pub requires_recent_reauth: bool,
    /// True if the action requires a single-use confirmation nonce.
    pub requires_two_step_nonce: bool,
    /// The exact targets the action will affect (F2 structured preview).
    pub preview: Vec<DestructivePreviewItem>,
    /// The human-readable warning message (must state "no undo" for hard purge).
    pub message: String,
}

impl DestructiveWarning {
    /// Build the warning for a given action. The `preview` list is empty by
    /// default — the caller populates it with the actual targets before
    /// showing the warning to the user.
    pub fn for_action(action: DestructiveAction) -> Self {
        match action {
            DestructiveAction::HardPurge => Self {
                action,
                irreversible: true,
                requires_recent_reauth: true,
                requires_two_step_nonce: true,
                preview: Vec::new(),
                message: "Hard purge is IRREVERSIBLE: the content key is destroyed and \
                          the data cannot be recovered. There is no undo."
                    .to_owned(),
            },
            DestructiveAction::EntityMerge | DestructiveAction::EntitySplit => Self {
                action,
                irreversible: false, // merge/split are audited events with undo via retract
                requires_recent_reauth: false,
                requires_two_step_nonce: false,
                preview: Vec::new(),
                message: "This operation is an audited event; it can be reversed via \
                          retract."
                    .to_owned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_purge_warning() {
        let w = DestructiveWarning::for_action(DestructiveAction::HardPurge);
        assert!(w.irreversible);
        assert!(w.requires_two_step_nonce);
        assert!(
            w.message.to_lowercase().contains("no undo")
                || w.message.to_lowercase().contains("cannot be recovered"),
            "warning must state no undo / cannot be recovered"
        );
    }
}
