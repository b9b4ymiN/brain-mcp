//! Subject Validator v1 — integration tests on real SemanticStore + the
//! production rules/*.toml files.
//!
//! These tests are the contract the Phase 1.5 DoD enforces. See
//! `docs/plans/subject-validator-v1-spec.md` §8.1.

use std::sync::Arc;

use llm_wiki::quality::{QualityChecker, QualityCheckerInput, QualitySeverity, QualityTagKind};
use llm_wiki::semantic::{EvidenceSummary, ProposalSummary};
use llm_wiki::subject_validator::{
    SUBJECT_VALIDATOR_VERSION, SubjectShape, SubjectValidator, SubjectVerdict,
};

fn production_validator() -> Arc<SubjectValidator> {
    let rules = include_str!("../rules/subject_rules.toml");
    let allow = include_str!("../rules/subject_allowlist.toml");
    let deny = include_str!("../rules/subject_denylist.toml");
    SubjectValidator::from_strings(rules, allow, deny, Default::default())
        .expect("production validator")
}

#[test]
fn validator_version_is_stamped() {
    assert_eq!(SUBJECT_VALIDATOR_VERSION, "subject-validator-v1");
}

#[test]
fn production_rules_load_without_error() {
    let _ = production_validator();
}

// ── Real-world subjects observed in inbox (from rootcause doc) ──────────

#[test]
fn lowercase_metric_subjects_rejected_critical() {
    let v = production_validator();
    for bad in [
        "risk-free rate",
        "beta",
        "terminal growth",
        "current case price",
        "equity value",
    ] {
        let r = v.validate(bad);
        assert_eq!(
            r.verdict,
            SubjectVerdict::Reject,
            "subject `{bad}` should be rejected, got shape={:?}",
            r.shape
        );
        assert!(r.quality_tags.iter().any(|t| {
            t.kind == QualityTagKind::BadSubjectShape && t.severity == QualitySeverity::Critical
        }));
    }
}

#[test]
fn slug_subjects_rejected_or_not_accepted() {
    let v = production_validator();
    // Note: "international-peers-deep_has_peer_data" mixes - and _ so
    // won't match pure Slug regex — but must NOT be accepted as a valid entity.
    for bad in [
        "international-peers-deep_has_peer_data",
        "thai-shipping-bf-report",
    ] {
        let r = v.validate(bad);
        assert_ne!(
            r.verdict,
            SubjectVerdict::Accept,
            "subject `{bad}` should not be accepted, got shape={:?}",
            r.shape
        );
    }
}

#[test]
fn real_entity_subjects_silent_accept() {
    let v = production_validator();
    // Per spec §7: CATL/BYD (not in allowlist, no digit) → Acronym →
    // `accept_info` → AcceptWithInfo. TSLA/NVDA are in the NASDAQ ticker
    // allowlist (rules/subject_allowlist.toml) → Ticker → `accept` → Accept.
    // Both are "silent accept" — no Critical tag.
    for good in ["CATL", "BYD", "TSLA", "NVDA"] {
        let r = v.validate(good);
        assert!(
            matches!(
                r.verdict,
                SubjectVerdict::Accept | SubjectVerdict::AcceptWithInfo
            ),
            "subject `{good}` got shape={:?}, verdict={:?}",
            r.shape,
            r.verdict
        );
        assert!(
            r.quality_tags
                .iter()
                .all(|t| t.severity != QualitySeverity::Critical)
        );
    }
}

#[test]
fn thai_entity_accept_info() {
    let v = production_validator();
    let r = v.validate("บมจ. ปตท.");
    assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
    assert_eq!(r.shape, SubjectShape::ThaiPure);
}

#[test]
fn ambiguous_acronym_flagged_info() {
    let v = production_validator();
    let r = v.validate("BAT");
    assert!(
        r.quality_tags
            .iter()
            .any(|t| t.kind == QualityTagKind::SubjectAmbiguousAcronym
                && t.severity == QualitySeverity::Info)
    );
}

#[test]
fn llm_bleed_placeholders_rejected() {
    let v = production_validator();
    for bad in ["<entity>", "[SUBJECT]", "TBD", "unknown", "unspecified"] {
        let r = v.validate(bad);
        assert_eq!(
            r.verdict,
            SubjectVerdict::Reject,
            "subject `{bad}` should be rejected (LLM bleed)"
        );
    }
}

#[test]
fn section_headings_rejected_even_when_title_case() {
    let v = production_validator();
    for bad in ["DCF Assumptions", "Risk Factors", "Executive Summary"] {
        let r = v.validate(bad);
        assert_eq!(
            r.verdict,
            SubjectVerdict::Reject,
            "subject `{bad}` (section heading) should be rejected"
        );
    }
}

#[test]
fn possessive_subjects_soft_flag() {
    let v = production_validator();
    let r = v.validate("Tesla's CFO");
    assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
}

#[test]
fn multi_entity_subjects_soft_flag() {
    let v = production_validator();
    let r = v.validate("CATL, BYD, LG");
    assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
}

// ── Adversarial inputs ───────────────────────────────────────────────────

#[test]
fn html_injection_rejected() {
    let v = production_validator();
    let r = v.validate("<script>alert(1)</script>");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
    assert!(
        r.quality_tags
            .iter()
            .any(|t| t.kind == QualityTagKind::BadSubjectAdversarial)
    );
}

#[test]
fn template_injection_rejected() {
    let v = production_validator();
    let r = v.validate("${evil}");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

#[test]
fn rtl_override_rejected() {
    let v = production_validator();
    let r = v.validate("\u{202E}CATL");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

#[test]
fn emoji_rejected() {
    let v = production_validator();
    let r = v.validate("CATL 🚀");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

// ── Normalize behavior ──────────────────────────────────────────────────

#[test]
fn zero_width_char_stripped_before_classification() {
    let v = production_validator();
    let r = v.validate("CATL\u{200B}");
    assert_eq!(r.normalized, "CATL");
    assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
}

// ── QualityChecker integration ──────────────────────────────────────────

#[test]
fn quality_checker_with_validator_flags_bad_subject() {
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    let p = ProposalSummary {
        proposal_id: Uuid::new_v4(),
        domain: "financial".to_string(),
        subject: "risk-free rate".to_string(),
        predicate: "is".to_string(),
        value: json!("1.75%"),
        claim_kind: "financial_metric".to_string(),
        provenance_kind: "inference".to_string(),
        submitted_at: Utc::now(),
        event_seq: 1,
    };
    let ev = EvidenceSummary {
        provenance_kind: "inference".to_string(),
        excerpt: Some("...".to_string()),
        source_id: None,
        quote_hash: None,
    };
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &ev,
        existing_claims: &[],
    };
    let checker = QualityChecker::new(production_validator());
    let tags = checker.check_deterministic(&input);
    assert!(
        tags.iter()
            .any(|t| t.kind == QualityTagKind::BadSubjectShape),
        "expected BadSubjectShape tag, got: {:?}",
        tags
    );
}
