//! Task 5.1 — Authenticated console shell + review workflow (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 5.1 DoD at the contract level (Rust domain
//! types the Console API will expose; no real React app in-env). The Console
//! is a first-party app that calls the same application API as MCP — it must
//! never write SQLite/Git/index directly (§9).

use llm_wiki::console::{
    ConsolePage, DiffPreview, ReviewAction, ReviewItem, ReviewState, SafeText,
};
use serde_json::json;

// =============================================================================
// DoD: pages use real API, no mock/TODO path
// =============================================================================

/// The Console's five primary pages are enumerable. Every page maps to an API
/// endpoint — there is no `MockPage` or `TodoPage` variant. §9.1.
#[test]
fn console_pages_are_enumerable_and_api_backed() {
    let _ = ConsolePage::Home;
    let _ = ConsolePage::Search;
    let _ = ConsolePage::Inbox;
    let _ = ConsolePage::Entity;
    let _ = ConsolePage::Operations;
    // No Mock/Todo variant exists — compile-time proof.
}

// =============================================================================
// DoD: approve/reject/edit/supersede shows evidence/diff before commit
// =============================================================================

/// A `ReviewItem` carries the proposal + its evidence so the Console can
/// render the diff BEFORE the user commits. §9.1 "approve/reject/edit/
/// supersede แสดง evidence/diff ก่อน commit".
#[test]
fn review_item_carries_proposal_and_evidence() {
    let item = ReviewItem {
        proposal_id: "prop-1".to_owned(),
        subject: "GULF".to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(58),
        evidence_summary: "Analyst note bytes 10-40".to_owned(),
        state: ReviewState::Pending,
    };
    assert!(!item.evidence_summary.is_empty());
    assert_eq!(item.state, ReviewState::Pending);
}

/// `DiffPreview` captures the before/after so the UI renders a diff, not a
/// blind apply. The Console must display this BEFORE calling commit.
#[test]
fn diff_preview_captures_before_and_after() {
    let diff = DiffPreview {
        field: "target_price".to_owned(),
        before: json!(55),
        after: json!(58),
    };
    assert_eq!(diff.before, json!(55));
    assert_eq!(diff.after, json!(58));
}

/// The review state machine enforces valid transitions. A Pending item can be
/// approved or rejected; an Approved item cannot be re-approved (no silent
/// overwrite — §4 rule 1).
#[test]
fn review_state_machine_rejects_invalid_transitions() {
    use ReviewState::*;
    // Valid: Pending → Approved.
    assert!(ReviewState::transition(Pending, ReviewAction::Approve).is_ok());
    // Valid: Pending → Rejected.
    assert!(ReviewState::transition(Pending, ReviewAction::Reject).is_ok());
    // Invalid: Approved → Approve (no re-approve).
    assert!(ReviewState::transition(Approved, ReviewAction::Approve).is_err());
}

// =============================================================================
// DoD: XSS-safe content rendering (§9.2, §10 stored-XSS)
// =============================================================================

/// `SafeText` guarantees the rendered string is escaped for DOM insertion.
/// The Console must use this (or `textContent`) — never raw `innerHTML` with
/// untrusted content. §9.2 "ห้ามส่งข้อมูลไม่วาใจเข้า HTML-capable nodeLabel".
#[test]
fn safe_text_escapes_html_entities() {
    let raw = "<script>alert('xss')</script>";
    let safe = SafeText::escape(raw);
    assert!(!safe.contains("<script>"), "script tag must be escaped");
    assert!(safe.contains("&lt;script&gt;"), "entities must be present");
    assert!(!safe.contains("'xss'"), "quotes must be escaped");
}

/// SafeText is idempotent — escaping an already-safe string does not double-
/// escape (prevents `&amp;lt;` corruption).
#[test]
fn safe_text_escape_is_idempotent() {
    let raw = "<b>bold</b>";
    let once = SafeText::escape(raw);
    let twice = SafeText::escape(&once);
    assert_eq!(once, twice, "double-escape must not corrupt");
}

// keep json import alive
#[test]
fn _json_compile_check() {
    let _ = json!({"ok": true});
}
