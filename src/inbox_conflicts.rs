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

use regex::Regex;
use std::sync::LazyLock;

static RE_NUMERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(-?[\d,]+(?:\.\d+)?)\s*([BMKbmk])?(%)?").unwrap()
});

/// Parse a JSON value (number or string) into f64, normalizing currency
/// prefixes (¥/$/€/£/฿), percent suffix, and magnitude suffixes (B/M/K).
/// Returns None for non-numeric values, arrays, objects, or null.
pub(crate) fn to_f64(v: &serde_json::Value) -> Option<f64> {
    use serde_json::Value;
    let s = match v {
        Value::Number(n) => return n.as_f64(),
        Value::String(s) => s.trim().to_string(),
        _ => return None,
    };
    let stripped = s.trim_start_matches(['¥', '$', '€', '£', '฿']);
    let caps = RE_NUMERIC.captures(stripped)?;
    let num_str = caps.get(1)?.as_str();
    let suffix = caps.get(2).map(|m| m.as_str().chars().next().unwrap_or(' '));
    let base: f64 = num_str.replace(',', "").parse().ok()?;
    let mult = match suffix {
        Some('B') | Some('b') => 1_000_000_000.0,
        Some('M') | Some('m') => 1_000_000.0,
        Some('K') | Some('k') => 1_000.0,
        _ => 1.0,
    };
    Some(base * mult)
}

#[cfg(test)]
mod tests_to_f64 {
    use super::to_f64;
    use serde_json::json;

    #[test]
    fn integer_json_number() {
        assert_eq!(to_f64(&json!(361)), Some(361.0));
    }
    #[test]
    fn float_json_number() {
        assert_eq!(to_f64(&json!(1.75)), Some(1.75));
    }
    #[test]
    fn plain_string_number() {
        assert_eq!(to_f64(&json!("361")), Some(361.0));
    }
    #[test]
    fn strip_yen() {
        assert_eq!(to_f64(&json!("¥361")), Some(361.0));
    }
    #[test]
    fn strip_dollar() {
        assert_eq!(to_f64(&json!("$1.75")), Some(1.75));
    }
    #[test]
    fn strip_percent() {
        assert_eq!(to_f64(&json!("1.75%")), Some(1.75));
    }
    #[test]
    fn strip_thousands_commas() {
        assert_eq!(to_f64(&json!("2,000.8")), Some(2000.8));
    }
    #[test]
    fn suffix_b_billion() {
        assert_eq!(to_f64(&json!("2,000.8B")), Some(2_000_800_000_000.0));
    }
    #[test]
    fn suffix_m_million() {
        assert_eq!(to_f64(&json!("5M")), Some(5_000_000.0));
    }
    #[test]
    fn suffix_k_thousand() {
        assert_eq!(to_f64(&json!("4.470K")), Some(4470.0));
    }
    #[test]
    fn case_insensitive_suffix() {
        assert_eq!(to_f64(&json!("5b")), Some(5_000_000_000.0));
    }
    #[test]
    fn non_numeric_string_returns_none() {
        assert_eq!(to_f64(&json!("expensive")), None);
    }
    #[test]
    fn null_returns_none() {
        assert_eq!(to_f64(&json!(null)), None);
    }
    #[test]
    fn array_returns_none() {
        assert_eq!(to_f64(&json!([1, 2])), None);
    }
}
