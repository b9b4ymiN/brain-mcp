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

// =============================================================================
// Phase E Task E3.1 — producer behaviour (GREEN stage)
// -----------------------------------------------------------------------------
// Each test exercises a real `SemanticStore` and asserts the contract type
// produced by the new scanner/method is correct against a seeded scenario.
// These complement the type-only RED-stage tests above.
// =============================================================================

use std::sync::Arc;

use chrono::Utc;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PredicateAssignment, PrivacyLabel, ProposeCommand,
    ProposeUserAssertionCommand, SemanticConfig, SemanticStore, SplitCommand, SplitOutcome,
    TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;

fn fixture() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

fn draft_with(subject: &str, predicate: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Seed a confirmed claim by capturing evidence, proposing, and confirming.
/// Returns the new claim_id.
fn seed_confirmed(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    predicate: &str,
    value: i64,
) -> uuid::Uuid {
    store
        .capture(
            context,
            CaptureCommand {
                operation_id: format!("{op}-cap"),
                bytes: b"evidence text".to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .expect("capture");
    store
        .propose(
            context,
            ProposeCommand {
                operation_id: op.to_owned(),
                capture_operation_id: format!("{op}-cap"),
                draft: draft_with(subject, predicate, value),
            },
        )
        .expect("propose");
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: format!("{op}-confirm"),
                proposal_operation_id: op.to_owned(),
            },
        )
        .expect("confirm");
    outcome.generated.claim_id.expect("claim id")
}

/// Seed a confirmed claim owned by a NAMED client (so `list_clients` sees a
/// mutation attributed to a non-bootstrap label).
fn seed_confirmed_as(
    store: &SemanticStore,
    label: &str,
    op: &str,
    subject: &str,
    predicate: &str,
    value: i64,
) -> uuid::Uuid {
    let context = store.register_client(label).expect("register client");
    let draft = ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    };
    store
        .capture(
            &context,
            CaptureCommand {
                operation_id: format!("{op}-cap"),
                bytes: b"evidence text".to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .expect("capture");
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: op.to_owned(),
                capture_operation_id: format!("{op}-cap"),
                draft,
            },
        )
        .expect("propose");
    let outcome = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: format!("{op}-confirm"),
                proposal_operation_id: op.to_owned(),
            },
        )
        .expect("confirm");
    outcome.generated.claim_id.expect("claim id")
}

/// Two active claims in the same `(domain, subject, predicate)` scope with
/// DIFFERENT values surface exactly one `Contradiction` flag naming both.
/// Equal values in the same scope (the supersede case) do NOT flag.
#[test]
fn contradictions_detects_divergent_values() {
    let (_parent, store, context) = fixture();
    // Two distinct values for the same scope.
    seed_confirmed(&store, &context, "c-a", "GULF", "target_price", 58);
    seed_confirmed(&store, &context, "c-b", "GULF", "target_price", 61);
    let head = store.ledger_head().expect("ledger head");
    let flags = store
        .contradictions(head, Utc::now())
        .expect("contradictions");
    let contradictions: Vec<_> = flags
        .iter()
        .filter(|flag| matches!(flag, TrustFlag::Contradiction { .. }))
        .collect();
    assert_eq!(
        contradictions.len(),
        1,
        "expected exactly one Contradiction flag, got {flags:?}"
    );
    if let TrustFlag::Contradiction { claim_ids } = contradictions[0] {
        assert_eq!(claim_ids.len(), 2, "flag should name both divergent claims");
    } else {
        panic!("filtered flag was not a Contradiction");
    }

    // Sanity: a single-claim scope produces no flags.
    seed_confirmed(&store, &context, "c-c", "PTT", "target_price", 40);
    let head = store.ledger_head().expect("ledger head");
    let flags = store
        .contradictions(head, Utc::now())
        .expect("contradictions");
    assert!(
        flags
            .iter()
            .all(|flag| matches!(flag, TrustFlag::Contradiction { .. })),
        "only Contradiction flags should be emitted"
    );
    let ptt_flags: Vec<_> = flags
        .iter()
        .filter_map(|flag| match flag {
            TrustFlag::Contradiction { claim_ids } if claim_ids.len() >= 2 => Some(claim_ids),
            _ => None,
        })
        .collect();
    // PTT has only one claim; the only multi-claim scope is GULF.
    assert_eq!(ptt_flags.len(), 1);
}

/// A claim whose confirm event was older than `threshold_days` surfaces a
/// `Stale` flag. With `threshold_days = 0`, EVERY active claim is stale
/// (days_since >= 1 > 0 — actually days_since is 0 on the same day, so we
/// verify the producer surfaces stale claims when days_since > threshold by
/// asserting the seeded claim appears in the result and carries a non-zero
/// days_since when threshold is 0... but 0 > 0 is false). The robust path:
/// seed a claim, then call staleness with threshold=0; a freshly-confirmed
/// claim has days_since=0 (today) and is NOT stale; we assert the producer
/// runs without error and returns a Vec. Then we use a NEGATIVE threshold
/// (treat as "always stale") is not possible — instead, this test asserts
/// staleness is well-defined and returns the right shape.
#[test]
fn staleness_flags_old_claims() {
    let (_parent, store, context) = fixture();
    seed_confirmed(&store, &context, "s-a", "GULF", "target_price", 58);
    let head = store.ledger_head().expect("ledger head");

    // threshold=0: a claim confirmed "now" has days_since=0, so it is NOT
    // strictly greater than 0 → not flagged. Confirms the producer runs and
    // returns the correct empty result for a fresh claim.
    let flags = store.staleness(head, 0, Utc::now()).expect("staleness");
    assert!(
        flags.is_empty(),
        "a fresh claim (days_since=0) is not stale at threshold=0: {flags:?}"
    );

    // A future-dated "now" simulates the passage of time: the claim's
    // recorded_at is in the past relative to the requested world_time, so
    // days_since > threshold fires.
    let future = Utc::now() + chrono::Duration::days(400);
    let flags = store.staleness(head, 365, future).expect("staleness");
    let stale: Vec<_> = flags
        .iter()
        .filter(|flag| matches!(flag, TrustFlag::Stale { .. }))
        .collect();
    assert_eq!(
        stale.len(),
        1,
        "expected exactly one Stale flag for the old claim, got {flags:?}"
    );
    if let TrustFlag::Stale {
        claim_id: _,
        days_since_modified,
    } = stale[0]
    {
        assert!(
            *days_since_modified > 365,
            "days_since should exceed the 365-day threshold: got {days_since_modified}"
        );
    }
}

/// `retrieval_trace` partitions active matches into included (top_k) and
/// excluded (beyond top_k) with a stable reason string.
#[test]
fn retrieval_trace_captures_included_excluded() {
    let (_parent, store, context) = fixture();
    // Three claims whose subject matches "GULF".
    seed_confirmed(&store, &context, "r-a", "GULF", "target_price", 1);
    seed_confirmed(&store, &context, "r-b", "GULF", "pe_ratio", 2);
    seed_confirmed(&store, &context, "r-c", "GULF", "eps", 3);
    // One unrelated claim.
    seed_confirmed(&store, &context, "r-d", "PTT", "target_price", 4);
    let head = store.ledger_head().expect("ledger head");

    let trace = store
        .retrieval_trace("gulf", 2, head, Utc::now())
        .expect("retrieval_trace");
    assert_eq!(
        trace.included_claim_ids.len(),
        2,
        "top_k=2 → 2 included: {trace:?}"
    );
    assert_eq!(
        trace.excluded_claim_ids.len(),
        1,
        "1 matched claim beyond top_k → 1 excluded: {trace:?}"
    );
    assert!(
        trace.reason.contains("top_k=2"),
        "reason must record the cutoff: {}",
        trace.reason
    );
    assert!(
        trace.reason.contains("matched 3"),
        "reason must record the match count: {}",
        trace.reason
    );
}

/// `list_clients` returns every registered client with at least one mutation
/// attributed to it. The bootstrap context plus a named client that confirms
/// a claim both appear; the named client's mutation_count >= 1.
#[test]
fn list_clients_returns_registered() {
    let (_parent, store, _context) = fixture();
    // Register a named client + drive one confirm through it.
    seed_confirmed_as(&store, "console-test", "lc-a", "GULF", "target_price", 58);

    let clients = store.list_clients().expect("list_clients");
    let named = clients
        .iter()
        .find(|client| client.label == "console-test")
        .expect("named client must appear");
    assert!(
        named.mutation_count >= 1,
        "named client must have at least one mutation: {named:?}"
    );
    assert!(
        !named.last_active_at.is_empty(),
        "named client must have a last_active_at: {named:?}"
    );
    // No person-identity field on the contract.
}

/// A fresh store reports an empty job queue: `{active:0, queued:0, failed:0}`.
#[test]
fn job_summary_starts_zero() {
    let (_parent, store, _context) = fixture();
    let summary = store.job_summary().expect("job_summary");
    assert_eq!(summary.active, 0);
    assert_eq!(summary.queued, 0);
    assert_eq!(summary.failed, 0);
}

/// Registering + failing a job surfaces in `job_summary` as failed=1, while
/// queued jobs surface as queued=1.
#[test]
fn job_summary_tracks_registry_transitions() {
    let (_parent, store, _context) = fixture();
    let queued_id = store.register_job("extraction").expect("register");
    let failed_id = store.register_job("consolidation").expect("register");
    store.fail_job(&failed_id).expect("fail");

    let summary = store.job_summary().expect("job_summary");
    assert_eq!(summary.queued, 1, "queued: {summary:?}");
    assert_eq!(summary.failed, 1, "failed: {summary:?}");
    assert_eq!(summary.active, 0);

    // Completing the queued job clears it from the active queue.
    store.complete_job(&queued_id).expect("complete");
    let summary = store.job_summary().expect("job_summary");
    assert_eq!(summary.queued, 0, "after complete: {summary:?}");
}

/// A fresh store has never been backed up and never run a restore drill.
#[test]
fn backup_health_starts_never_false() {
    let (_parent, store, _context) = fixture();
    let health = store.backup_health().expect("backup_health");
    assert_eq!(
        health.last_backup_at, "never",
        "fresh store has no backups: {health:?}"
    );
    assert!(
        !health.last_restore_drill_ok,
        "fresh store has never run a restore drill: {health:?}"
    );
}

/// A fresh store has no eval runs; the producer surfaces the zero/"never"
/// contract state rather than an error.
#[test]
fn eval_summary_no_run_returns_zero() {
    let (_parent, store, _context) = fixture();
    let summary = store.eval_summary("stocks").expect("eval_summary");
    assert_eq!(summary.case_count, 0);
    assert_eq!(summary.passed, 0);
    assert!(!summary.abstention_passed);
    assert_eq!(summary.run_at, "never");
}

/// Recording an eval run surfaces through `eval_summary` (publisher helper
/// for Phase 4.3 — exercised here so the read+write pair is covered).
#[test]
fn eval_summary_reflects_recorded_run() {
    let (_parent, store, _context) = fixture();
    let run_at = "2026-07-17T00:00:00Z"
        .parse::<chrono::DateTime<Utc>>()
        .unwrap();
    store
        .record_eval_run("stocks", 30, 28, true, run_at)
        .expect("record_eval_run");
    let summary = store.eval_summary("stocks").expect("eval_summary");
    assert_eq!(summary.case_count, 30);
    assert_eq!(summary.passed, 28);
    assert!(summary.abstention_passed);
    assert_eq!(summary.run_at, run_at.to_rfc3339());
}

/// `split_entities` moves claims by predicate: claims whose predicate matches
/// an assignment rewrite onto the target; claims whose predicate is NOT in
/// any assignment stay on the source. Source is preserved (not deleted).
#[test]
fn split_entities_moves_claims_by_predicate() {
    let (_parent, store, context) = fixture();

    // Create source + target entities explicitly.
    let source = store
        .resolve_or_create_entity(&context, "stocks", "GULF-combined")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&context, "stocks", "GULF-prices")
        .expect("target entity");

    // Two claims on the source: one whose predicate we move, one we leave.
    store
        .propose_user_assertion(
            &context,
            ProposeUserAssertionCommand {
                operation_id: "split-source-1".to_owned(),
                utterance: b"price claim".to_vec(),
                draft: ClaimDraft {
                    subject: "GULF-combined".to_owned(),
                    predicate: "target_price".to_owned(),
                    value: json!(58),
                    claim_kind: "user_assertion".to_owned(),
                    domain: "stocks".to_owned(),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose 1");
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "split-confirm-1".to_owned(),
                proposal_operation_id: "split-source-1".to_owned(),
            },
        )
        .expect("confirm 1");
    store
        .propose_user_assertion(
            &context,
            ProposeUserAssertionCommand {
                operation_id: "split-source-2".to_owned(),
                utterance: b"sector claim".to_vec(),
                draft: ClaimDraft {
                    subject: "GULF-combined".to_owned(),
                    predicate: "sector".to_owned(),
                    value: json!("energy"),
                    claim_kind: "user_assertion".to_owned(),
                    domain: "stocks".to_owned(),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose 2");
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "split-confirm-2".to_owned(),
                proposal_operation_id: "split-source-2".to_owned(),
            },
        )
        .expect("confirm 2");

    // Split target_price → target. sector stays on source.
    let outcome = store
        .split_entities(
            &context,
            SplitCommand {
                operation_id: "split-1".to_owned(),
                source_entity_id: source,
                assignments: vec![PredicateAssignment {
                    predicate: "target_price".to_owned(),
                    target_entity_id: target,
                }],
            },
        )
        .expect("split");
    assert_eq!(outcome.event.event_type, "entity_split");

    // Re-derive the structured view.
    let view: SplitOutcome = store
        .last_split_outcome_for(&context, source)
        .expect("last split outcome")
        .expect("a split happened");
    assert_eq!(view.moved_claims.len(), 1, "one claim moved");
    assert_eq!(
        view.moved_claims[0].predicate, "target_price",
        "the moved predicate is target_price"
    );
    assert_eq!(
        view.moved_claims[0].to_entity_id, target,
        "moved onto the target entity"
    );
    assert_eq!(
        view.source_remaining_claim_count, 1,
        "one claim (sector) remains on source"
    );

    // The source entity still exists and still holds the sector claim.
    let source_claims = store
        .claims_for_entity(&context, source)
        .expect("source claims after split");
    assert!(
        source_claims.iter().any(|c| c.predicate == "sector"),
        "sector claim must remain on source: {source_claims:?}"
    );
    assert!(
        !source_claims.iter().any(|c| c.predicate == "target_price"),
        "target_price must have been moved off source: {source_claims:?}"
    );

    // The target entity now holds the target_price claim.
    let target_claims = store
        .claims_for_entity(&context, target)
        .expect("target claims after split");
    assert!(
        target_claims.iter().any(|c| c.predicate == "target_price"),
        "target_price claim must be on target: {target_claims:?}"
    );
}

/// `split_entities` is idempotent on `operation_id`: replaying the same
/// operation returns the same outcome and does NOT move any new claims
/// (no duplicate writes).
#[test]
fn split_entities_is_idempotent_on_operation_id() {
    let (_parent, store, context) = fixture();
    let source = store
        .resolve_or_create_entity(&context, "stocks", "GULF-idem")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&context, "stocks", "GULF-idem-out")
        .expect("target entity");
    store
        .propose_user_assertion(
            &context,
            ProposeUserAssertionCommand {
                operation_id: "idem-src".to_owned(),
                utterance: b"price".to_vec(),
                draft: ClaimDraft {
                    subject: "GULF-idem".to_owned(),
                    predicate: "target_price".to_owned(),
                    value: json!(58),
                    claim_kind: "user_assertion".to_owned(),
                    domain: "stocks".to_owned(),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose");
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "idem-confirm".to_owned(),
                proposal_operation_id: "idem-src".to_owned(),
            },
        )
        .expect("confirm");

    let cmd = SplitCommand {
        operation_id: "split-idem".to_owned(),
        source_entity_id: source,
        assignments: vec![PredicateAssignment {
            predicate: "target_price".to_owned(),
            target_entity_id: target,
        }],
    };
    let first = store.split_entities(&context, cmd.clone()).expect("first");
    let second = store
        .split_entities(&context, cmd)
        .expect("second (replay)");
    assert_eq!(
        first.event.event_seq, second.event.event_seq,
        "idempotent replay returns the same event_seq"
    );
}

/// Splitting an entity across domains is rejected (mirror of merge_entities'
/// cross-domain guard).
#[test]
fn split_entities_rejects_cross_domain() {
    let (_parent, store, context) = fixture();
    let stocks_source = store
        .resolve_or_create_entity(&context, "stocks", "GULF-x")
        .expect("source");
    let projects_target = store
        .resolve_or_create_entity(&context, "projects", "GULF-x-prj")
        .expect("target");

    let err = store
        .split_entities(
            &context,
            SplitCommand {
                operation_id: "split-x".to_owned(),
                source_entity_id: stocks_source,
                assignments: vec![PredicateAssignment {
                    predicate: "target_price".to_owned(),
                    target_entity_id: projects_target,
                }],
            },
        )
        .expect_err("cross-domain split must fail");
    assert!(
        matches!(err, llm_wiki::semantic::SemanticError::InvalidTransition(_)),
        "expected InvalidTransition, got {err:?}"
    );
}
