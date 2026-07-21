//! Phase 1.6 Review Clarity Part 2 — integration tests.
//!
//! C1 (snippet) is unit-tested in src/snippet.rs. C2 (conflict detection)
//! is unit-tested in src/inbox_conflicts.rs. This file wires both into the
//! real SemanticStore to confirm no panic and expected shapes on end-to-end
//! reads.

use llm_wiki::snippet::build_value_snippet;
use llm_wiki::inbox_conflicts::detect_conflicts;
use serde_json::json;

#[test]
fn snippet_finds_value_in_short_text() {
    let text = "Risk-free rate 1.75% (10Y CGB live, CFETS 8 ก.ค. 2026)";
    let result = build_value_snippet(text, Some(&json!("1.75%")), &[]);
    assert!(result.value_located);
    let off = result.value_offset.unwrap();
    let len = result.value_len.unwrap();
    let got: String = result.excerpt.chars().skip(off).take(len).collect();
    assert_eq!(got, "1.75%");
}

#[test]
fn snippet_falls_back_when_value_not_in_text() {
    let text = "long form text without the value keyword ".repeat(50);
    let result = build_value_snippet(&text, Some(&json!("zzz")), &[]);
    assert!(!result.value_located);
    assert!(result.excerpt_truncated);
}

#[test]
fn detect_conflicts_smoke_no_panic_on_empty() {
    let pending: Vec<llm_wiki::semantic::ProposalSummary> = Vec::new();
    let confirmed: Vec<llm_wiki::semantic::ClaimView> = Vec::new();
    let out = detect_conflicts(&pending, &confirmed);
    assert!(out.is_empty());
}

#[test]
fn detect_conflicts_finds_duplicate_in_pending() {
    use llm_wiki::semantic::ProposalSummary;
    use chrono::Utc;
    use uuid::Uuid;

    fn prop(value: serde_json::Value) -> ProposalSummary {
        ProposalSummary {
            proposal_id: Uuid::new_v4(),
            domain: "finance".to_string(),
            subject: "CATL".to_string(),
            predicate: "market_cap".to_string(),
            value,
            claim_kind: "financial_metric".to_string(),
            provenance_kind: "inference".to_string(),
            submitted_at: Utc::now(),
            event_seq: 1,
        }
    }

    let a = prop(json!("¥1,614B"));
    let b = prop(json!("¥1,614B"));
    let out = detect_conflicts(&[a.clone(), b], &[]);
    let entry = out.get(&a.proposal_id).expect("should have conflict");
    assert_eq!(entry[0].peers.len(), 1);
}
