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

// ── Destructive-action warning ───────────────────────────────────────────────

/// The destructive actions that require a warning + nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestructiveAction {
    HardPurge,
    EntityMerge,
    EntitySplit,
}

/// A warning the UI MUST display before a destructive action. §5.3 "hard purge
/// มี preview, recent re-auth, two-step nonce และคำเตือนว่า irreversible โดยไม่มี
/// undo".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestructiveWarning {
    pub action: DestructiveAction,
    pub irreversible: bool,
    pub requires_two_step_nonce: bool,
    /// The human-readable warning message (must state "no undo" for hard purge).
    pub message: String,
}

impl DestructiveWarning {
    /// Build the warning for a given action.
    pub fn for_action(action: DestructiveAction) -> Self {
        match action {
            DestructiveAction::HardPurge => Self {
                action,
                irreversible: true,
                requires_two_step_nonce: true,
                message: "Hard purge is IRREVERSIBLE: the content key is destroyed and \
                          the data cannot be recovered. There is no undo."
                    .to_owned(),
            },
            DestructiveAction::EntityMerge | DestructiveAction::EntitySplit => Self {
                action,
                irreversible: false, // merge/split are audited events with undo via retract
                requires_two_step_nonce: false,
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
