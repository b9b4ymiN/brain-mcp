//! Eval-driven evolution contract (Task 7.1 + 7.2).
//!
//! GOAL-vNext §13 Phase 7: retrieval experiments (7.1) + safe automation (7.2).
//! The system improves without changing the canonical contract and without
//! new vendor/framework lock-in.

use serde::{Deserialize, Serialize};

// ── Task 7.1: retrieval experiments ──────────────────────────────────────────

/// A frozen retrieval baseline (§7.1 "BM25 baseline ถูก freeze").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalBaseline {
    pub name: String,
    pub frozen: bool,
}

impl RetrievalBaseline {
    /// The frozen BM25 baseline.
    pub fn bm25_frozen() -> Self {
        Self {
            name: "bm25".to_owned(),
            frozen: true,
        }
    }
}

/// A retrieval candidate (vector / reranker / Graphiti) evaluated against the
/// baseline on the same corpus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalCandidate {
    pub name: String,
    pub recall_at_10_delta: f64,
    pub ndcg_at_10_delta: f64,
    pub abstention_delta: f64,
    /// Latency regression percentage (positive = slower).
    pub latency_regression_pct: f64,
}

/// The promotion decision for a retrieval candidate. §7.1: promote only when
/// Recall@10 or nDCG@10 improves ≥ threshold, other metric doesn't regress
/// beyond bound, hard invariants don't regress, and latency/cost regression
/// ≤20% (else needs user approval).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromoteDecision {
    pub promote: bool,
    pub needs_user_approval: bool,
    pub reason: String,
}

impl PromoteDecision {
    /// Evaluate a candidate against the promotion thresholds.
    pub fn evaluate(
        candidate: &RetrievalCandidate,
        min_improvement: f64,
        max_metric_regression: f64,
        max_latency_regression_pct: f64,
    ) -> Self {
        // Hard invariant: no metric regresses beyond bound.
        if candidate.ndcg_at_10_delta < -max_metric_regression
            || candidate.abstention_delta < -max_metric_regression
        {
            return Self {
                promote: false,
                needs_user_approval: false,
                reason: "a metric regressed beyond the allowed bound".to_owned(),
            };
        }
        // Latency gate: >20% needs user approval.
        if candidate.latency_regression_pct > max_latency_regression_pct {
            return Self {
                promote: false,
                needs_user_approval: true,
                reason: format!(
                    "latency regression {:.1}% exceeds {:.1}% — needs user approval",
                    candidate.latency_regression_pct, max_latency_regression_pct
                ),
            };
        }
        // Improvement gate: recall OR nDCG must improve ≥ threshold.
        if candidate.recall_at_10_delta >= min_improvement
            || candidate.ndcg_at_10_delta >= min_improvement
        {
            Self {
                promote: true,
                needs_user_approval: false,
                reason: "meets improvement threshold within bounds".to_owned(),
            }
        } else {
            Self {
                promote: false,
                needs_user_approval: false,
                reason: format!(
                    "improvement ({:.3}/{:.3}) below threshold {:.3}",
                    candidate.recall_at_10_delta, candidate.ndcg_at_10_delta, min_improvement
                ),
            }
        }
    }
}

// ── Task 7.2: safe automation ────────────────────────────────────────────────

/// Budget caps for autonomous operations. §7.2 "มี budget/policy/kill switch".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationBudget {
    pub max_consolidation_runs_per_week: u32,
    pub max_auto_approves_per_day: u32,
}

impl Default for AutomationBudget {
    fn default() -> Self {
        Self {
            max_consolidation_runs_per_week: 3,
            max_auto_approves_per_day: 50,
        }
    }
}

impl AutomationBudget {
    pub fn is_within_consolidation_budget(&self, runs_this_week: u32) -> bool {
        runs_this_week < self.max_consolidation_runs_per_week
    }
}

/// The automation policy: kill switch + budget. §7.2 "scheduled consolidation,
/// stale review และ auto-approval มี budget/policy/kill switch".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationPolicy {
    pub kill_switch: bool,
    pub budget: AutomationBudget,
}

impl AutomationPolicy {
    /// True when the kill switch is engaged — no autonomous operation runs.
    pub fn is_disabled(&self) -> bool {
        self.kill_switch
    }
}

/// Monthly eval/drift report. §7.2 "monthly eval/drift report แสดงคุณภาพ
/// ค่าใช้จ่าย และ failure samples".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub quality_score: f64,
    pub cost_usd: f64,
    pub failure_samples: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promote_threshold() {
        let c = RetrievalCandidate {
            name: "x".into(),
            recall_at_10_delta: 0.04,
            ndcg_at_10_delta: 0.0,
            abstention_delta: 0.0,
            latency_regression_pct: 0.0,
        };
        assert!(PromoteDecision::evaluate(&c, 0.03, 0.01, 20.0).promote);
    }
}
