//! Task 5.3 — Trust + operations views contract (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 5.3 DoD at the contract level. The user can
//! answer four core questions and see trust/operations surfaces; destructive
//! ops carry preview + irreversible warnings.

use llm_wiki::trust::{
    BackupHealth, ClientActivity, DestructiveAction, DestructivePreviewItem, DestructiveWarning,
    EvalSummary, JobSummary, ProvenanceAnswer, ProvenanceQuestion, RetrievalTrace, TrustFlag,
    TrustView,
};

// =============================================================================
// DoD: trust view — contradictions/staleness/trace visible
// =============================================================================

/// `TrustView` surfaces every trust flag (contradiction, stale, orphan) so the
/// user sees contested/expired knowledge. §5.3 "contradictions, staleness".
#[test]
fn trust_view_collects_flags() {
    let view = TrustView {
        flags: vec![
            TrustFlag::Contradiction {
                claim_ids: vec!["c-1".into(), "c-2".into()],
            },
            TrustFlag::Stale {
                claim_id: "c-3".into(),
                days_since_modified: 400,
            },
        ],
    };
    assert_eq!(view.flags.len(), 2);
}

/// `RetrievalTrace` explains WHY a result was selected (or excluded) — the
/// user can audit the system's reasoning, not just accept an answer.
#[test]
fn retrieval_trace_explains_selection() {
    let trace = RetrievalTrace {
        included_claim_ids: vec!["c-1".into()],
        excluded_claim_ids: vec!["c-2".into()],
        reason: "c-2 superseded by c-1 at event_seq 42".into(),
    };
    assert!(!trace.reason.is_empty());
    assert_eq!(trace.excluded_claim_ids.len(), 1);
}

// =============================================================================
// DoD: operations view — jobs/evals/backup health
// =============================================================================

/// `JobSummary` reports the async-job queue state (extraction/consolidation).
#[test]
fn job_summary_reports_queue() {
    let job = JobSummary {
        active: 2,
        queued: 5,
        failed: 1,
    };
    assert_eq!(job.active + job.queued + job.failed, 8);
}

/// `BackupHealth` reports the last backup time + whether restore was verified.
#[test]
fn backup_health_reports_status() {
    let health = BackupHealth {
        last_backup_at: "2026-07-16T00:00:00Z".into(),
        last_restore_drill_ok: true,
    };
    assert!(health.last_restore_drill_ok);
}

// =============================================================================
// DoD: four core questions the user can answer
// =============================================================================

/// The user can ask "what do I know about X" and get a structured answer
/// covering the four dimensions: what / source / when-true / connections.
/// §5.3 "ผู้ใช้ตอบคำถามหลัก รู้อะไร/มาจากไหน/จริงเมื่อไร/เชื่อมอะไร".
#[test]
fn provenance_question_has_four_dimensions() {
    let q = ProvenanceQuestion::about("GULF");
    assert_eq!(q.subject, "GULF");
    // The four question facets are enumerable.
    let _ = ProvenanceQuestion::WHAT;
    let _ = ProvenanceQuestion::SOURCE;
    let _ = ProvenanceQuestion::WHEN;
    let _ = ProvenanceQuestion::CONNECTIONS;
}

/// `ProvenanceAnswer` answers the four questions with evidence.
#[test]
fn provenance_answer_covers_four_dimensions() {
    let answer = ProvenanceAnswer {
        what: "target_price = 58".into(),
        source: "analyst note, rendition rend-1 bytes 10-40".into(),
        when_true: "valid 2026-06-01 to 2026-09-01".into(),
        connections: "related to GULF-EQ (merged); sources/analyst-2026".into(),
        client_that_edited: "claude-desktop (client_id cli-1)".into(),
    };
    assert!(!answer.what.is_empty());
    assert!(!answer.source.is_empty());
    assert!(!answer.when_true.is_empty());
    assert!(!answer.connections.is_empty());
    // §5.3: "client/channel ใดแก้" — audit field present.
    assert!(!answer.client_that_edited.is_empty());
}

// =============================================================================
// DoD: destructive ops — preview + irreversible warning + two-step nonce
// =============================================================================

/// A `DestructiveWarning` for hard purge carries the irreversible flag + the
/// two-step nonce requirement. The UI MUST display this before proceeding.
/// §5.3 "hard purge มี preview, recent re-auth, two-step nonce และคำเตือนว่า
/// irreversible โดยไม่มี undo".
#[test]
fn hard_purge_warning_is_irreversible_and_requires_nonce() {
    let warning = DestructiveWarning::for_action(DestructiveAction::HardPurge);
    assert!(
        warning.irreversible,
        "hard purge must be flagged irreversible"
    );
    assert!(
        warning.requires_two_step_nonce,
        "hard purge requires a two-step nonce"
    );
    // The warning must state there is NO undo (it warns the user, not claims one exists).
    assert!(
        warning.message.to_lowercase().contains("no undo")
            || warning
                .message
                .to_lowercase()
                .contains("cannot be recovered"),
        "warning must state there is no undo / cannot be recovered: got {}",
        warning.message
    );
}

/// Hard purge requires BOTH recent re-auth AND a two-step nonce — they are
/// separate controls (F3). Re-auth = authentication freshness; nonce =
/// operation-confirmation token. §5.3 + §10 SELECTED PURGE POLICY.
#[test]
fn hard_purge_requires_recent_reauth_distinct_from_nonce() {
    let warning = DestructiveWarning::for_action(DestructiveAction::HardPurge);
    assert!(
        warning.requires_recent_reauth,
        "hard purge must require recent re-auth (freshness gate)"
    );
    assert!(
        warning.requires_two_step_nonce,
        "hard purge must require a two-step nonce (confirmation token)"
    );
    // Merge/split do NOT require re-auth (they are reversible audited events).
    let merge = DestructiveWarning::for_action(DestructiveAction::EntityMerge);
    assert!(!merge.requires_recent_reauth);
    assert!(!merge.requires_two_step_nonce);
}

/// The destructive warning carries a structured `preview` list of affected
/// targets (F2), not just a message string. The Console populates it before
/// showing the warning so the user sees exactly what will be lost.
#[test]
fn destructive_warning_carries_structured_preview() {
    let mut warning = DestructiveWarning::for_action(DestructiveAction::HardPurge);
    warning.preview.push(DestructivePreviewItem {
        target_kind: "object".to_owned(),
        target_id: "sha256:abc123".to_owned(),
        effect: "content key destroyed — bytes unreadable".to_owned(),
    });
    assert_eq!(warning.preview.len(), 1);
    assert_eq!(warning.preview[0].target_id, "sha256:abc123");
}

/// `ClientActivity` surfaces client/token audit (F1 — DoD bullet 1 "client
/// activity"). §9.1 Operations item 7. NOT person inference (TM-024).
#[test]
fn client_activity_surfaces_audit_not_person() {
    let activity = ClientActivity {
        client_id: "cli-1".to_owned(),
        label: "claude-desktop".to_owned(),
        capabilities: vec!["brain.read".into(), "brain.confirm".into()],
        last_active_at: "2026-07-16T00:00:00Z".to_owned(),
        mutation_count: 42,
    };
    assert_eq!(activity.label, "claude-desktop");
    assert_eq!(activity.mutation_count, 42);
    // No person field — only client/channel audit.
}

/// `EvalSummary` surfaces eval health (F1 — DoD bullet 1 "evals").
#[test]
fn eval_summary_reports_health() {
    let eval = EvalSummary {
        case_count: 126,
        passed: 126,
        abstention_passed: true,
        run_at: "2026-07-16T00:00:00Z".to_owned(),
    };
    assert_eq!(eval.passed, 126);
    assert!(eval.abstention_passed);
}
