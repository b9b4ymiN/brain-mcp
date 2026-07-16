//! Consolidation + domain evals (Task 4.3).
//!
//! Domain contract for GOAL-vNext §13 Task 4.3 + §8 consolidation cycle +
//! §11 eval contract. Three surfaces:
//!
//! - [`ConsolidationReport`] — duplicate/contradiction/stale candidates sent
//!   to the review queue. NEVER auto-applied (§8.1 "ห้าม auto-merge").
//! - [`AutoApprovePolicy`] — off by default; per `(memory_type, source)`
//!   enablement with a confidence threshold. §8.3.
//! - [`DomainEvalReport`] + [`RegressionReport`] — the eval/regression gates
//!   a model/prompt version must pass before promotion. §11 + §8.3.
//!
//! Contract-level: detectors are deterministic stubs the integration layer
//! swaps for LLM-backed ones. No real LLM run here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Consolidation candidates (review queue) ──────────────────────────────────

/// The kind of consolidation candidate. §8.1 Stage A/B/C.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationKind {
    /// Two claims with high title/content overlap. §8.1 Stage A.
    Duplicate,
    /// Two claims in the same scope with conflicting values. §8.1 Stage B.
    Contradiction,
    /// A claim not modified/linked in a long time. §8.1 Stage C.
    Stale,
}

/// One consolidation candidate: a kind + the claim ids involved + a
/// human-readable detail string for the review queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationCandidate {
    pub kind: ConsolidationKind,
    pub claim_ids: Vec<String>,
    pub detail: String,
}

/// The output of a consolidation pass: a list of candidates for the review
/// queue. There is deliberately NO `applied` field — §8.1 forbids auto-merge;
/// a human reviews every candidate before `consolidate_apply` runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub candidates: Vec<ConsolidationCandidate>,
}

// ── Auto-approve policy (off by default, §8.3) ───────────────────────────────

/// A `(memory_type, source)` enablement key for auto-approve.
type TypeSource = (String, String);

/// The auto-approve policy. Off by default for every type/source. When enabled
/// for a `(type, source)`, a claim still must pass the configured confidence
/// threshold before auto-approve fires. §8.3 "auto-approve ปิดเป็น default".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutoApprovePolicy {
    /// Enabled `(type, source)` → required confidence threshold (0.0–1.0).
    enabled: HashMap<TypeSource, f64>,
}

impl AutoApprovePolicy {
    /// Enable auto-approve for `(memory_type, source)` at the given confidence
    /// threshold. A claim of this type from this source auto-approves only if
    /// its confidence ≥ threshold.
    pub fn enable(&mut self, memory_type: &str, source: &str, threshold: f64) {
        self.enabled
            .insert((memory_type.to_owned(), source.to_owned()), threshold);
    }

    /// True if `(type, source)` is enabled in the policy (regardless of
    /// confidence). This is a PRE-CHECK only — it is NOT an approval. The
    /// actual auto-approve gate is [`Self::is_auto_approved_with_confidence`],
    /// which additionally requires the claim's confidence to meet the
    /// configured threshold. Do not treat a `true` here as permission to
    /// auto-approve.
    pub fn is_auto_approved(&self, memory_type: &str, source: &str) -> bool {
        self.enabled
            .contains_key(&(memory_type.to_owned(), source.to_owned()))
    }

    /// True if `(type, source)` is enabled AND `confidence ≥ threshold`. The
    /// full gate a proposal must pass before auto-approve fires.
    pub fn is_auto_approved_with_confidence(
        &self,
        memory_type: &str,
        source: &str,
        confidence: f64,
    ) -> bool {
        match self
            .enabled
            .get(&(memory_type.to_owned(), source.to_owned()))
        {
            Some(threshold) => confidence >= *threshold,
            None => false,
        }
    }
}

// ── Domain eval report (§11) ─────────────────────────────────────────────────

/// Eval results across the three domains (stocks/projects/knowledge) + the
/// adversarial abstention hard-invariant. §11: golden sets of ≥30 cases each;
/// abstention must pass 100%.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainEvalReport {
    pub stocks_update_pass: u32,
    pub stocks_update_total: u32,
    pub projects_time_pass: u32,
    pub projects_time_total: u32,
    pub knowledge_provenance_pass: u32,
    pub knowledge_provenance_total: u32,
    pub adversarial_abstention_pass: u32,
    pub adversarial_abstention_total: u32,
}

impl DomainEvalReport {
    /// True only if abstention passes 100% (a hard invariant — §11: a system
    /// that answers when it should abstain is a hard-invariant failure).
    pub fn abstention_passed_100(&self) -> bool {
        self.adversarial_abstention_total > 0
            && self.adversarial_abstention_pass == self.adversarial_abstention_total
    }
}

// ── Regression report (§8.3) ─────────────────────────────────────────────────

/// The metric delta between a candidate (model, prompt) version and the
/// incumbent on the eval corpus. Positive = improvement; negative = regression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionDelta {
    pub candidate_model: String,
    pub candidate_prompt: String,
    pub incumbent_model: String,
    pub incumbent_prompt: String,
    /// Recall@10 change (positive = better).
    pub recall_at_10_delta: f64,
    /// nDCG@10 change.
    pub ndcg_at_10_delta: f64,
    /// Abstention rate change (positive = more correct abstentions).
    pub abstention_delta: f64,
}

/// The regression gate: a candidate version is promotable only if NO metric
/// regresses beyond `max_allowed_regression`. §8.3 "regression report
/// เปรียบเทียบ model/prompt version ก่อน promote".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegressionReport {
    pub deltas: Vec<RegressionDelta>,
    /// The maximum allowed regression on any single metric (absolute). A delta
    /// more negative than `-max_allowed_regression` blocks promotion.
    pub max_allowed_regression: f64,
}

impl RegressionReport {
    /// True if every delta is within the allowed regression bound on all three
    /// metrics. A single out-of-bound metric blocks promotion.
    pub fn is_promotable(&self) -> bool {
        let bound = self.max_allowed_regression;
        self.deltas.iter().all(|d| {
            d.recall_at_10_delta >= -bound
                && d.ndcg_at_10_delta >= -bound
                && d.abstention_delta >= -bound
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approve_default_off() {
        let policy = AutoApprovePolicy::default();
        assert!(!policy.is_auto_approved("external_fact", "web"));
    }

    #[test]
    fn regression_blocks_on_regression() {
        let report = RegressionReport {
            deltas: vec![RegressionDelta {
                candidate_model: "v2".into(),
                candidate_prompt: "p2".into(),
                incumbent_model: "v1".into(),
                incumbent_prompt: "p1".into(),
                recall_at_10_delta: -0.05,
                ndcg_at_10_delta: 0.0,
                abstention_delta: 0.0,
            }],
            max_allowed_regression: 0.02,
        };
        assert!(!report.is_promotable());
    }
}
