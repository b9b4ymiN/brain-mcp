//! Smart Console contract (Task 5.1).
//!
//! Domain types for GOAL-vNext §13 Task 5.1 + §9 Console. The Console is a
//! first-party authenticated app calling the same application API as MCP —
//! it must NEVER write SQLite/Git/index directly (§9). This module defines:
//!
//! - [`ConsolePage`] — the five primary pages (all API-backed, no mock path).
//! - [`ReviewItem`] + [`ReviewState`] + [`ReviewAction`] — the inbox/review
//!   workflow with a state machine that shows evidence/diff BEFORE commit.
//! - [`DiffPreview`] — before/after capture for diff rendering.
//! - [`SafeText`] — XSS-safe HTML escaping (§9.2, §10 stored-XSS).
//!
//! Contract-level: the React app + E2E harness are deployment artifacts; the
//! types here are the API contract the app consumes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Pages (§9.1) ─────────────────────────────────────────────────────────────

/// The Console's five primary pages. Every page is API-backed — there is no
/// `Mock` or `Todo` variant. §9.1 Home/Search/Inbox/Entity/Operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolePage {
    Home,
    Today,
    Search,
    Inbox,
    Entity,
    Operations,
    Activity,
    Status,
    Config,
}

// ── Review workflow (§9.1 Inbox/Review) ──────────────────────────────────────

/// The state of a review item in the inbox workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Awaiting user decision (approve/reject/edit).
    Pending,
    /// User approved — the application service committed the proposal.
    Approved,
    /// User rejected — the proposal is discarded (history preserved).
    Rejected,
}

/// A user action on a review item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Approve,
    Reject,
}

/// Transition error — the state machine rejects invalid transitions (no silent
/// overwrite, §4 rule 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewTransitionError {
    AlreadyDecided,
    InvalidAction,
}

impl ReviewState {
    /// Apply a review action, returning the new state or an error if the
    /// transition is invalid. A Pending item may be approved or rejected; an
    /// already-decided item cannot be re-decided (§4 rule 1 no silent
    /// overwrite).
    pub fn transition(
        from: ReviewState,
        action: ReviewAction,
    ) -> Result<ReviewState, ReviewTransitionError> {
        match (from, action) {
            (ReviewState::Pending, ReviewAction::Approve) => Ok(ReviewState::Approved),
            (ReviewState::Pending, ReviewAction::Reject) => Ok(ReviewState::Rejected),
            (ReviewState::Approved | ReviewState::Rejected, _) => {
                Err(ReviewTransitionError::AlreadyDecided)
            }
        }
    }
}

/// One review item in the inbox: the proposal + an evidence summary so the
/// Console renders the diff BEFORE the user commits. §9.1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub proposal_id: String,
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    /// Human-readable evidence summary for the diff view (NOT raw bytes).
    pub evidence_summary: String,
    pub state: ReviewState,
}

/// A before/after field diff for the review UI. The Console must display this
/// before calling commit. §9.1 "แสดง evidence/diff ก่อน commit".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffPreview {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

// ── XSS-safe text rendering (§9.2, §10 stored-XSS) ───────────────────────────

/// XSS-safe HTML escaping for Console rendering. The Console must use this
/// (or DOM `textContent`) — never raw `innerHTML` with untrusted content.
/// §9.2 "ห้ามส่งข้อมูลไม่วาใจเข้า HTML-capable nodeLabel โดยตรง".
///
/// Escapes: `&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`, `"` → `&quot;`,
/// `'` → `&#x27;`. One-time render concern: escape EXACTLY ONCE at render
/// time; double-escaping produces visible `&amp;lt;` corruption.
pub struct SafeText;

impl SafeText {
    /// Escape a raw string for safe DOM insertion. Call exactly once per
    /// render; do NOT re-escape an already-escaped string.
    pub fn escape(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        for c in raw.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#x27;"),
                _ => out.push(c),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_basics() {
        assert_eq!(SafeText::escape("<b>"), "&lt;b&gt;");
        assert_eq!(SafeText::escape("\"hi\""), "&quot;hi&quot;");
    }

    #[test]
    fn transition_pending_approve() {
        assert_eq!(
            ReviewState::transition(ReviewState::Pending, ReviewAction::Approve),
            Ok(ReviewState::Approved)
        );
    }

    #[test]
    fn transition_already_decided() {
        assert_eq!(
            ReviewState::transition(ReviewState::Approved, ReviewAction::Approve),
            Err(ReviewTransitionError::AlreadyDecided)
        );
    }
}
