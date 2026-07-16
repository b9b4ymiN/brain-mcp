//! Task 7.1 + 7.2 — Eval-driven evolution contract (RED stage).
//!
//! GOAL-vNext §13 Phase 7: retrieval experiments (7.1) + safe automation (7.2).

use llm_wiki::evolution::{
    AutomationBudget, AutomationPolicy, DriftReport, PromoteDecision, RetrievalBaseline,
    RetrievalCandidate,
};

// =============================================================================
// Task 7.1: retrieval experiments
// =============================================================================

#[test]
fn bm25_baseline_is_frozen() {
    let baseline = RetrievalBaseline::bm25_frozen();
    assert_eq!(baseline.name, "bm25");
    assert!(baseline.frozen, "BM25 baseline must be frozen");
}

#[test]
fn candidate_promotion_requires_threshold() {
    let candidate = RetrievalCandidate {
        name: "vector-bge-m3".to_owned(),
        recall_at_10_delta: 0.04,
        ndcg_at_10_delta: 0.02,
        abstention_delta: 0.0,
        latency_regression_pct: 10.0,
    };
    // ≥0.03 improvement on recall + no metric regression >0.01 + latency <20%
    let decision = PromoteDecision::evaluate(&candidate, 0.03, 0.01, 20.0);
    assert!(decision.promote, "0.04 recall gain ≥0.03 threshold → promote");
}

#[test]
fn candidate_rejected_on_latency_regression() {
    let candidate = RetrievalCandidate {
        name: "vector-heavy".to_owned(),
        recall_at_10_delta: 0.05,
        ndcg_at_10_delta: 0.03,
        abstention_delta: 0.0,
        latency_regression_pct: 25.0, // >20% → needs user approval
    };
    let decision = PromoteDecision::evaluate(&candidate, 0.03, 0.01, 20.0);
    assert!(
        !decision.promote,
        "25% latency regression >20% must block auto-promote"
    );
    assert!(
        decision.needs_user_approval,
        "latency regression >20% requires user approval"
    );
}

// =============================================================================
// Task 7.2: safe automation
// =============================================================================

#[test]
fn automation_policy_has_kill_switch() {
    let policy = AutomationPolicy {
        kill_switch: true,
        budget: AutomationBudget::default(),
    };
    assert!(policy.is_disabled(), "kill switch engaged = disabled");
}

#[test]
fn automation_budget_caps_runs() {
    let budget = AutomationBudget {
        max_consolidation_runs_per_week: 3,
        max_auto_approves_per_day: 50,
    };
    assert!(budget.is_within_consolidation_budget(2));
    assert!(!budget.is_within_consolidation_budget(5));
}

#[test]
fn drift_report_records_quality_cost_samples() {
    let report = DriftReport {
        quality_score: 0.82,
        cost_usd: 1.50,
        failure_samples: vec!["stock-001 abstention missed".to_owned()],
    };
    assert!(report.quality_score > 0.0);
    assert!(!report.failure_samples.is_empty());
}
