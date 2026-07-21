//! Same-predicate conflict detection (Phase 1.6 Review Clarity Part 2, C2).
//!
//! Replaces the "N current confirmed claims in scope" text (which always
//! returned 0 because the filter was confirmed-only and the typical review
//! queue is all-pending) with deterministic conflict detection across
//! pending + confirmed claims.
//!
//! # Conflict kinds (Phase 1.6)
//!
//! - **C1 Hard value conflict**: same `(domain, subject, predicate)`, different
//!   scalar value, relative difference > 0.1%. Maps to QualitySeverity::Warning.
//! - **C2 Duplicate**: same `(domain, subject, predicate)`, same value.
//!   Maps to QualitySeverity::Info.
//!
//! Kinds C3-C6 (type mismatch, cross-predicate tension, semantic, temporal)
//! are deferred to later phases — see spec §6.
//!
//! # Algorithm
//!
//! O(M·K) bucket-and-pair: bucket all claims by (domain, subject, predicate),
//! pairwise compare within each bucket. With 182 pending + few confirmed and
//! average bucket size K≈2, this is ~91 comparisons — trivially cheap.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    HardValue,
    Duplicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Pending,
    Confirmed,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictPeer {
    pub peer_id: Uuid,
    pub peer_status: PeerStatus,
    pub value: serde_json::Value,
    pub submitted_at: Option<DateTime<Utc>>,
    /// Only set for `ConflictKind::HardValue`. Relative difference in percent.
    pub rel_diff_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScopeConflict {
    pub proposal_id: Uuid,
    pub kind: ConflictKind,
    pub peers: Vec<ConflictPeer>,
}

/// Detect C1 (HardValue) + C2 (Duplicate) conflicts across pending and
/// confirmed claims. Returns a map from proposal_id → list of conflicts
/// that proposal participates in.
///
/// Implementation in Task 10.
pub fn detect_conflicts(
    pending: &[crate::semantic::ProposalSummary],
    confirmed: &[crate::semantic::ClaimView],
) -> std::collections::HashMap<Uuid, Vec<ScopeConflict>> {
    let _ = (pending, confirmed);
    std::collections::HashMap::new()
}
