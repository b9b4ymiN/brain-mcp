//! Task 4.3 — Consolidation + domain evals (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 4.3 DoD at the contract level:
//! - duplicate/contradiction/stale detection ส่งเข้าคิว review
//! - stocks/projects/knowledge golden sets ทดสอบ update/time/provenance/abstention
//! - auto-approve ปิด default เปิดได้เฉพาะ memory type/source ที่ผ่าน threshold
//! - regression report เปรียบเทียบ model/prompt version ก่อน promote
//!
//! Contract-level: no real LLM run. Detectors are deterministic stubs the
//! integration layer swaps for LLM-backed ones.

use llm_wiki::consolidation::{
    AutoApprovePolicy, ConsolidationCandidate, ConsolidationReport, ConsolidationKind,
    DomainEvalReport, RegressionDelta, RegressionReport,
};
use chrono::Utc;

// =============================================================================
// DoD: duplicate/contradiction/stale detection → review queue
// =============================================================================

/// `ConsolidationReport` classifies candidates into dedupe/contradiction/stale
/// buckets. §8.1 Stage A/B/C. NONE are auto-applied — every candidate goes to
/// the review queue (§8.1 "ห้าม auto-merge").
#[test]
fn consolidation_report_classifies_candidates_without_auto_applying() {
    let report = ConsolidationReport {
        candidates: vec![
            ConsolidationCandidate {
                kind: ConsolidationKind::Duplicate,
                claim_ids: vec!["c-1".into(), "c-2".into()],
                detail: "title cosine ≥0.85".into(),
            },
            ConsolidationCandidate {
                kind: ConsolidationKind::Contradiction,
                claim_ids: vec!["c-3".into(), "c-4".into()],
                detail: "GULF target_price 58 vs 60".into(),
            },
            ConsolidationCandidate {
                kind: ConsolidationKind::Stale,
                claim_ids: vec!["c-5".into()],
                detail: "last_modified > 12 months, no inbound links".into(),
            },
        ],
    };
    assert_eq!(report.candidates.len(), 3);
    // Every candidate is a REVIEW item — the report carries no "applied" list.
    // Compile-time proof: ConsolidationReport has no `applied` field.
}

/// The three consolidation kinds are distinct — a detector never conflates
/// a duplicate with a contradiction.
#[test]
fn consolidation_kinds_are_distinct() {
    assert_ne!(ConsolidationKind::Duplicate, ConsolidationKind::Contradiction);
    assert_ne!(ConsolidationKind::Contradiction, ConsolidationKind::Stale);
    assert_ne!(ConsolidationKind::Duplicate, ConsolidationKind::Stale);
}

// =============================================================================
// DoD: auto-approve OFF by default; per-type threshold gating
// =============================================================================

/// The default `AutoApprovePolicy` has auto-approve OFF for every memory
/// type. §8.3 "auto-approve ปิดเป็น default".
#[test]
fn auto_approve_is_off_by_default() {
    let policy = AutoApprovePolicy::default();
    assert!(!policy.is_auto_approved("external_fact", "web"));
    assert!(!policy.is_auto_approved("user_assertion", "console"));
    assert!(!policy.is_auto_approved("preference", "import"));
}

/// Auto-approve can be enabled per memory type + source, but ONLY when the
/// source's confidence passes the type's threshold. Below threshold = denied.
#[test]
fn auto_approve_respects_per_type_threshold() {
    let mut policy = AutoApprovePolicy::default();
    // Enable auto-approve for external_fact from "trusted_feed" at 0.9.
    policy.enable("external_fact", "trusted_feed", 0.9);
    assert!(
        policy.is_auto_approved_with_confidence("external_fact", "trusted_feed", 0.95),
        "above threshold should auto-approve"
    );
    assert!(
        !policy.is_auto_approved_with_confidence("external_fact", "trusted_feed", 0.8),
        "below threshold must NOT auto-approve"
    );
    // A different source is not covered by the enable.
    assert!(
        !policy.is_auto_approved_with_confidence("external_fact", "untrusted", 0.99),
        "unapproved source must not auto-approve even at high confidence"
    );
}

// =============================================================================
// DoD: domain eval — stocks/projects/knowledge golden sets
// =============================================================================

/// `DomainEvalReport` covers the three domains + the four eval dimensions
/// (update / time / provenance / abstention). §11 eval contract.
#[test]
fn domain_eval_covers_three_domains_and_four_dimensions() {
    let report = DomainEvalReport {
        stocks_update_pass: 28,
        stocks_update_total: 30,
        projects_time_pass: 29,
        projects_time_total: 30,
        knowledge_provenance_pass: 27,
        knowledge_provenance_total: 30,
        adversarial_abstention_pass: 30,
        adversarial_abstention_total: 30,
    };
    assert!(report.stocks_update_total == 30);
    assert!(report.projects_time_total == 30);
    assert!(report.knowledge_provenance_total == 30);
    assert!(report.adversarial_abstention_total == 30);
}

/// Abstention must pass 100% — a system that answers when it should abstain
/// is a hard-invariant failure (§11).
#[test]
fn abstention_is_a_hard_invariant() {
    let report = DomainEvalReport {
        stocks_update_pass: 30,
        stocks_update_total: 30,
        projects_time_pass: 30,
        projects_time_total: 30,
        knowledge_provenance_pass: 30,
        knowledge_provenance_total: 30,
        adversarial_abstention_pass: 29, // one failure
        adversarial_abstention_total: 30,
    };
    assert!(
        !report.abstention_passed_100(),
        "29/30 abstention is a hard-invariant failure"
    );
}

// =============================================================================
// DoD: regression report — model/prompt version comparison before promote
// =============================================================================

/// `RegressionReport` compares a candidate model/prompt version against the
/// incumbent on the eval corpus. A candidate that regresses any metric beyond
/// the threshold must NOT be promoted. §8.3 "regression report เปรียบเทียบ
/// model/prompt version ก่อน promote".
#[test]
fn regression_report_blocks_promotion_on_regression() {
    let delta = RegressionDelta {
        candidate_model: "glm-coding-v2".into(),
        candidate_prompt: "extract-v2".into(),
        incumbent_model: "glm-coding-v1".into(),
        incumbent_prompt: "extract-v1".into(),
        recall_at_10_delta: -0.05, // regression
        ndcg_at_10_delta: 0.01,
        abstention_delta: 0.0,
    };
    let report = RegressionReport {
        deltas: vec![delta.clone()],
        max_allowed_regression: 0.02,
    };
    assert!(
        !report.is_promotable(),
        "a -0.05 recall regression exceeds the 0.02 threshold → block promote"
    );
}

/// A candidate that improves (or holds) on every metric IS promotable.
#[test]
fn regression_report_allows_promotion_on_improvement() {
    let delta = RegressionDelta {
        candidate_model: "glm-coding-v2".into(),
        candidate_prompt: "extract-v2".into(),
        incumbent_model: "glm-coding-v1".into(),
        incumbent_prompt: "extract-v1".into(),
        recall_at_10_delta: 0.04,
        ndcg_at_10_delta: 0.03,
        abstention_delta: 0.0,
    };
    let report = RegressionReport {
        deltas: vec![delta],
        max_allowed_regression: 0.02,
    };
    assert!(
        report.is_promotable(),
        "all-positive deltas → promotable"
    );
}

// keep Utc import alive
#[test]
fn _utc_compile_check() {
    let _ = Utc::now();
}
