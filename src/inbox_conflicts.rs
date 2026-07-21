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
pub fn detect_conflicts(
    pending: &[crate::semantic::ProposalSummary],
    confirmed: &[crate::semantic::ClaimView],
) -> std::collections::HashMap<Uuid, Vec<ScopeConflict>> {
    use std::collections::HashMap;

    #[derive(Clone)]
    struct Entry {
        id: Uuid,
        status: PeerStatus,
        domain: String,
        subject: String,
        predicate: String,
        value: serde_json::Value,
        submitted_at: Option<DateTime<Utc>>,
    }

    let mut all: Vec<Entry> = Vec::with_capacity(pending.len() + confirmed.len());
    for p in pending {
        all.push(Entry {
            id: p.proposal_id,
            status: PeerStatus::Pending,
            domain: p.domain.clone(),
            subject: p.subject.clone(),
            predicate: p.predicate.clone(),
            value: p.value.clone(),
            submitted_at: Some(p.submitted_at),
        });
    }
    for c in confirmed {
        all.push(Entry {
            id: c.claim_id,
            status: PeerStatus::Confirmed,
            domain: c.domain.clone(),
            subject: c.subject.clone(),
            predicate: c.predicate.clone(),
            value: c.value.clone(),
            // ClaimView has no `submitted_at`; `valid_from` is the closest
            // creation-era timestamp on a confirmed claim.
            submitted_at: c.valid_from,
        });
    }

    // Bucket by (domain, subject, predicate).
    let mut buckets: HashMap<(String, String, String), Vec<usize>> = HashMap::new();
    for (i, e) in all.iter().enumerate() {
        buckets
            .entry((e.domain.clone(), e.subject.clone(), e.predicate.clone()))
            .or_default()
            .push(i);
    }

    let mut out: HashMap<Uuid, Vec<ScopeConflict>> = HashMap::new();
    let threshold = 0.001_f64; // 0.1% relative difference

    for (_, indices) in buckets {
        if indices.len() < 2 {
            continue;
        }
        for &i in &indices {
            let mut peer_list: Vec<ConflictPeer> = Vec::new();
            let mut has_hard_value = false;
            let mut has_duplicate = false;
            for &j in &indices {
                if i == j {
                    continue;
                }
                let a = &all[i];
                let b = &all[j];
                let (kind, rel_diff) = classify_pair(&a.value, &b.value, threshold);
                if let Some(k) = kind {
                    if k == ConflictKind::HardValue {
                        has_hard_value = true;
                    } else {
                        has_duplicate = true;
                    }
                    peer_list.push(ConflictPeer {
                        peer_id: b.id,
                        peer_status: b.status,
                        value: b.value.clone(),
                        submitted_at: b.submitted_at,
                        rel_diff_pct: rel_diff.map(|r| r * 100.0),
                    });
                }
            }
            if has_hard_value || has_duplicate {
                let final_kind = if has_hard_value {
                    ConflictKind::HardValue
                } else {
                    ConflictKind::Duplicate
                };
                out.entry(all[i].id).or_default().push(ScopeConflict {
                    proposal_id: all[i].id,
                    kind: final_kind,
                    peers: peer_list,
                });
            }
        }
    }
    out
}

/// Classify a pair of values as conflict or no-conflict.
fn classify_pair(
    a: &serde_json::Value,
    b: &serde_json::Value,
    threshold: f64,
) -> (Option<ConflictKind>, Option<f64>) {
    if a == b {
        return (Some(ConflictKind::Duplicate), None);
    }
    let (Some(a_f), Some(b_f)) = (to_f64(a), to_f64(b)) else {
        return (None, None);
    };
    let max_abs = a_f.abs().max(b_f.abs());
    if max_abs == 0.0 {
        return (Some(ConflictKind::Duplicate), None);
    }
    let rel_diff = ((a_f - b_f).abs()) / max_abs;
    if rel_diff > threshold {
        (Some(ConflictKind::HardValue), Some(rel_diff))
    } else {
        (Some(ConflictKind::Duplicate), None)
    }
}

use regex::Regex;
use std::sync::LazyLock;

static RE_NUMERIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(-?[\d,]+(?:\.\d+)?)\s*([BMKbmk])?(%)?").unwrap());

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
    let suffix = caps
        .get(2)
        .map(|m| m.as_str().chars().next().unwrap_or(' '));
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

#[cfg(test)]
mod tests_detect {
    use super::*;
    use crate::semantic::ProposalSummary;
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn proposal(
        domain: &str,
        subject: &str,
        predicate: &str,
        value: serde_json::Value,
    ) -> ProposalSummary {
        ProposalSummary {
            proposal_id: Uuid::new_v4(),
            domain: domain.to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            value,
            claim_kind: "financial_metric".to_string(),
            provenance_kind: "inference".to_string(),
            submitted_at: Utc::now(),
            event_seq: 1,
        }
    }

    #[test]
    fn detects_duplicate_same_value() {
        let a = proposal("finance", "CATL", "market_cap", json!("¥1,614B"));
        let b = proposal("finance", "CATL", "market_cap", json!("¥1,614B"));
        let out = detect_conflicts(&[a.clone(), b], &[]);
        let entry = out.get(&a.proposal_id).expect("should have conflict");
        assert_eq!(entry[0].kind, ConflictKind::Duplicate);
        assert_eq!(entry[0].peers.len(), 1);
    }

    #[test]
    fn detects_hard_value_numeric_conflict() {
        let a = proposal("finance", "CATL", "market_cap", json!("¥1,614B"));
        let b = proposal("finance", "CATL", "market_cap", json!("¥2,000.8B"));
        let out = detect_conflicts(&[a.clone(), b], &[]);
        let entry = out.get(&a.proposal_id).expect("should have conflict");
        assert_eq!(entry[0].kind, ConflictKind::HardValue);
        let rel = entry[0].peers[0]
            .rel_diff_pct
            .expect("should have rel_diff");
        assert!(rel > 10.0, "expected > 10%, got {rel}");
    }

    #[test]
    fn no_conflict_different_predicate() {
        let a = proposal("finance", "CATL", "current_case_price", json!("¥361"));
        let b = proposal("finance", "CATL", "dcf_price_per_share", json!("¥447.6"));
        let out = detect_conflicts(&[a, b], &[]);
        assert!(out.is_empty(), "different predicates → no conflict");
    }

    #[test]
    fn tiny_diff_below_threshold_no_hard_value() {
        let a = proposal("finance", "X", "y", json!("1.750"));
        let b = proposal("finance", "X", "y", json!("1.751"));
        let out = detect_conflicts(&[a.clone(), b], &[]);
        let entry = out.get(&a.proposal_id).expect("should still appear");
        assert_eq!(entry[0].kind, ConflictKind::Duplicate);
    }

    #[test]
    fn unparseable_values_no_conflict() {
        let a = proposal("finance", "X", "y", json!("expensive"));
        let b = proposal("finance", "X", "y", json!("cheap"));
        let out = detect_conflicts(&[a, b], &[]);
        assert!(out.is_empty());
    }
}
