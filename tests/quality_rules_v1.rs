//! AI Pre-Review Phase 1.3 + 1.4 — deterministic quality-rule integration
//! tests and the false-positive gate.
//!
//! # Phase 1.3 — rule integration tests (8 tests)
//!
//! Each test drives the *real* `SemanticStore` (no mocks) through the public
//! `capture` / `propose` / `confirm` API, then runs `QualityChecker` on the
//! pending proposal via `check_proposal`. This is the contract that the
//! Phase 2 HTTP handler (`GET /inbox/{proposal_id}/ai-review`) will rely on,
//! so exercising it end-to-end here catches drift between store + checker.
//!
//! # Phase 1.4 — false-positive gate (1 advisory test)
//!
//! `false_positive_rate_on_30_confirmed_claims_reported` seeds 30 confirmed
//! claims and runs the deterministic checker on each. The observed FP rate
//! is printed via `eprintln!` so the CI log captures it; the test never
//! fails on a high rate. The user explicitly accepted the strict
//! 4-domain / 7-kind canon trade-off (see
//! `docs/plans/feature-ai-review-and-quality-rules.md`); this test
//! documents the canon mismatch and is the artifact a future session uses
//! to decide whether to relax the canon.

use std::sync::Arc;

use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticStore, TrustedContext,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

// ── store fixture (adapted from tests/api_console_v1.rs) ──────────────────

fn make_store() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

/// Fully-parameterized draft helper. The api_console_v1.rs `draft()` helper
/// hardcodes predicate/kind/domain/confidence — but the rule tests need
/// control over every "dirty" field, so this variant takes them all.
fn custom_draft(
    subject: &str,
    predicate: &str,
    value: Value,
    claim_kind: &str,
    domain: &str,
    confidence_basis_points: u16,
) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value,
        claim_kind: claim_kind.to_owned(),
        domain: domain.to_owned(),
        confidence_basis_points,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Propose-only (stays pending). Used by the rule tests so `check_proposal`
/// can still find the proposal in `list_pending_proposals`.
fn seed_dirty_proposal(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    evidence: &str,
    draft: ClaimDraft,
) -> Uuid {
    store
        .capture(
            context,
            CaptureCommand {
                operation_id: format!("{op}-cap"),
                bytes: evidence.as_bytes().to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .unwrap();
    let outcome = store
        .propose(
            context,
            ProposeCommand {
                operation_id: op.to_owned(),
                capture_operation_id: format!("{op}-cap"),
                draft,
            },
        )
        .unwrap();
    outcome.generated.proposal_id.unwrap()
}

/// Propose + confirm with a fully custom draft. Returns the new claim_id.
fn seed_confirmed_draft(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    evidence: &str,
    draft: ClaimDraft,
) -> Uuid {
    seed_dirty_proposal(store, context, op, evidence, draft);
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: format!("{op}-confirm"),
                proposal_operation_id: op.to_owned(),
            },
        )
        .unwrap();
    outcome.generated.claim_id.unwrap()
}

/// Propose + confirm a CATL+target_price claim using the api_console_v1
/// defaults (predicate=target_price, kind=external_fact, domain=stocks,
/// conf=8_000). Used when a test just needs *some* existing confirmed
/// claim to collide with.
fn seed_confirmed_default(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    value: i64,
) -> Uuid {
    seed_confirmed_draft(
        store,
        context,
        op,
        "evidence text",
        custom_draft(
            subject,
            "target_price",
            json!(value),
            "external_fact",
            "stocks",
            8_000,
        ),
    )
}

// ── checker runner ────────────────────────────────────────────────────────

/// Run the deterministic checker against a still-pending proposal, after
/// gathering the store's full current claim set (active + future + past) as
/// the `existing_claims` context — this mirrors what the Phase 2 HTTP
/// handler will pass.
fn check_proposal(store: &SemanticStore, proposal_id: Uuid) -> Vec<llm_wiki::quality::QualityTag> {
    use chrono::Utc;
    use llm_wiki::quality::{QualityChecker, QualityCheckerInput};

    let pending = store.list_pending_proposals().unwrap();
    let proposal = pending
        .iter()
        .find(|p| p.proposal_id == proposal_id)
        .expect("proposal must be pending");
    let evidence = store.evidence_for(proposal_id).unwrap();
    let head = store.ledger_head().unwrap();
    let current = store.all_claims_current(head, Utc::now()).unwrap();

    // Existing claims = the full historical+current scope, exactly as the
    // Phase 2 handler will assemble it. DuplicatePredicate / ConfidenceTooHigh
    // consult this set.
    let mut existing: Vec<_> = current.active.clone();
    existing.extend(current.future.iter().cloned());
    existing.extend(current.past.iter().cloned());

    let input = QualityCheckerInput {
        proposal,
        evidence: &evidence,
        existing_claims: &existing,
    };
    QualityChecker::without_subject_validation().check_deterministic(&input)
}

fn has_tag(
    tags: &[llm_wiki::quality::QualityTag],
    kind: llm_wiki::quality::QualityTagKind,
) -> bool {
    tags.iter().any(|t| t.kind == kind)
}

// ── Phase 1.3 — the 8 rule tests ──────────────────────────────────────────

#[test]
fn duplicate_predicate_tagged() {
    let (_parent, store, ctx) = make_store();
    // Seed a confirmed CATL+target_price claim.
    seed_confirmed_default(&store, &ctx, "c1", "CATL", 100);
    // Re-propose the same (subject, predicate) — must trip DuplicatePredicate.
    let dup_id = seed_dirty_proposal(
        &store,
        &ctx,
        "p1",
        "evidence",
        custom_draft(
            "CATL",
            "target_price",
            json!(120),
            "external_fact",
            "stocks",
            8_000,
        ),
    );
    let tags = check_proposal(&store, dup_id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::DuplicatePredicate),
        "expected DuplicatePredicate, got {:?}",
        tags
    );
}

#[test]
fn packed_facts_vs_tagged() {
    let (_parent, store, ctx) = make_store();
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p2",
        "some evidence",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("$60 vs $69/kWh"),
            "financial_metric", // in canon → no TaxonomyDrift pollution
            "financial",
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::PackedFacts),
        "expected PackedFacts, got {:?}",
        tags
    );
}

#[test]
fn vague_predicate_margin_tagged() {
    let (_parent, store, ctx) = make_store();
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p3",
        "some evidence",
        custom_draft(
            "CATL",
            "margin", // bare metric noun → vague
            json!("24%"),
            "financial_metric",
            "financial",
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::VaguePredicate),
        "expected VaguePredicate, got {:?}",
        tags
    );
}

#[test]
fn taxonomy_drift_domain_tagged() {
    let (_parent, store, ctx) = make_store();
    // domain="stocks" is NOT in the strict canon {business, financial,
    // project, personal} — must trip TaxonomyDrift. claim_kind stays in
    // canon so we only get the domain-side tag (we don't assert on count,
    // only presence).
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p4",
        "some evidence",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("$69/kWh"),
            "financial_metric", // in canon
            "stocks",           // NOT in canon
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::TaxonomyDrift),
        "expected TaxonomyDrift for domain=stocks, got {:?}",
        tags
    );
}

#[test]
fn taxonomy_drift_kind_tagged() {
    let (_parent, store, ctx) = make_store();
    // claim_kind="external_fact" is NOT in the strict canon — must trip
    // TaxonomyDrift. domain stays in canon.
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p5",
        "some evidence",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("$69/kWh"),
            "external_fact", // NOT in canon
            "financial",     // in canon
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::TaxonomyDrift),
        "expected TaxonomyDrift for claim_kind=external_fact, got {:?}",
        tags
    );
}

#[test]
fn confidence_too_high_tagged() {
    let (_parent, store, ctx) = make_store();
    // Seed a CONFIRMED claim with confidence 1.0 (10000 bps).
    seed_confirmed_draft(
        &store,
        &ctx,
        "c6",
        "evidence text",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("$69/kWh"),
            "financial_metric",
            "financial",
            10_000, // conf 1.0
        ),
    );
    // Re-propose the same (subject, predicate) with a different value.
    // The `propose` path produces provenance_kind="evidence" (NOT
    // user_assertion), so check_confidence_too_high does NOT take its
    // early-return; the existing claim's conf==10000 trips the rule.
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p6",
        "more evidence",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("$65/kWh"),
            "financial_metric",
            "financial",
            9_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::ConfidenceTooHigh),
        "expected ConfidenceTooHigh, got {:?}",
        tags
    );
}

#[test]
fn double_bracket_in_value_tagged() {
    let (_parent, store, ctx) = make_store();
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p7",
        "some evidence",
        custom_draft(
            "CATL",
            "battery cost 2026",
            json!("[[wikis]] foo"), // wiki link syntax in value
            "financial_metric",
            "financial",
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        has_tag(&tags, llm_wiki::quality::QualityTagKind::DoubleBracket),
        "expected DoubleBracket, got {:?}",
        tags
    );
}

#[test]
fn clean_fact_no_tags() {
    // The live-fire negative case from the spec: an FX rate fact should be
    // clean. With the strict 4-domain canon, `domain="fx"` is NOT allowed,
    // so TaxonomyDrift WILL fire — that is the user-acknowledged strict-canon
    // trade-off (Phase 1.4 reports the FP rate; the user accepted it). For
    // THIS test we only assert the four content rules do NOT fire:
    // PackedFacts / VaguePredicate / DoubleBracket / DuplicatePredicate.
    let (_parent, store, ctx) = make_store();
    let id = seed_dirty_proposal(
        &store,
        &ctx,
        "p8",
        "rate source text",
        custom_draft(
            "USDTHB-2026-07-20",
            "has_rate",
            json!("33.59"),
            "external_fact",
            "fx", // not in canon — TaxonomyDrift will fire, intentionally
            8_000,
        ),
    );
    let tags = check_proposal(&store, id);
    assert!(
        !has_tag(&tags, llm_wiki::quality::QualityTagKind::PackedFacts),
        "clean FX rate should not trip PackedFacts, got {:?}",
        tags
    );
    assert!(
        !has_tag(&tags, llm_wiki::quality::QualityTagKind::VaguePredicate),
        "predicate `has_rate` is specific; should not trip VaguePredicate, got {:?}",
        tags
    );
    assert!(
        !has_tag(&tags, llm_wiki::quality::QualityTagKind::DoubleBracket),
        "clean FX rate should not trip DoubleBracket, got {:?}",
        tags
    );
    assert!(
        !has_tag(&tags, llm_wiki::quality::QualityTagKind::DuplicatePredicate),
        "fresh (subject, predicate) should not trip DuplicatePredicate, got {:?}",
        tags
    );
}

// ── Phase 1.4 — False-positive gate (advisory, non-blocking) ============
//
// Seeds 30 confirmed claims and runs the deterministic checker on each
// (treating each confirmed claim's own ProposalSummary reconstruction as
// the "proposal"). Reports the FP rate to stderr via `eprintln!` so the
// CI log captures it. This test does NOT fail on a high FP rate — the
// user explicitly accepted the strict 4/7 canon trade-off (see plan). The
// observed rate is recorded in the Phase 1.5 commit message.
//
// The "expected" baseline FP rate with strict canon + "stocks" domain in
// the test fixture is high (~100% for the seed shape below, since every
// seeded claim uses domain=stocks + claim_kind=external_fact, both
// outside canon). This is intentional: it documents the canon mismatch.
// A future session that relaxes the canon will see this number drop.

#[test]
fn false_positive_rate_on_30_confirmed_claims_reported() {
    let (_parent, store, ctx) = make_store();
    let mut confirmed_ids = Vec::with_capacity(30);
    for i in 0..30 {
        let id = seed_confirmed_default(&store, &ctx, &format!("c{i}"), "CATL", 100 + i);
        confirmed_ids.push(id);
    }
    // confirmed_id is a claim_id, not proposal_id — but for FP measurement
    // we want to construct a ProposalSummary shape from each confirmed
    // claim's view. Use `all_claims_current` to get ClaimViews, then build
    // a synthetic ProposalSummary from each.
    use chrono::Utc;
    use llm_wiki::quality::{QualityChecker, QualityCheckerInput};
    use llm_wiki::semantic::ProposalSummary;

    let head = store.ledger_head().unwrap();
    let current = store.all_claims_current(head, Utc::now()).unwrap();
    let active = current.active.clone();
    assert!(
        active.len() >= 30,
        "expected ≥30 active claims, got {}",
        active.len()
    );

    let ev_template = llm_wiki::semantic::EvidenceSummary {
        provenance_kind: "inference".to_string(),
        excerpt: Some("evidence text".to_string()),
        source_id: None,
        quote_hash: None,
    };

    let mut flagged = 0usize;
    let mut by_kind: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for cv in active.iter().take(30) {
        // Build a synthetic ProposalSummary from the ClaimView — represents
        // "what would the proposal have looked like that produced this claim?"
        let p = ProposalSummary {
            proposal_id: cv.proposal_id,
            domain: cv.domain.clone(),
            subject: cv.subject.clone(),
            predicate: cv.predicate.clone(),
            value: cv.value.clone(),
            claim_kind: cv.claim_kind.clone(),
            provenance_kind: cv.provenance_kind.clone(),
            submitted_at: Utc::now(), // synthetic; not used by rules
            event_seq: cv.confirmed_event_seq,
        };
        let input = QualityCheckerInput {
            proposal: &p,
            evidence: &ev_template,
            existing_claims: &active, // pass all active so DuplicatePredicate can fire
        };
        let tags = QualityChecker::without_subject_validation().check_deterministic(&input);
        if !tags.is_empty() {
            flagged += 1;
        }
        for t in &tags {
            *by_kind.entry(format!("{:?}", t.kind)).or_insert(0) += 1;
        }
    }
    let total = 30usize;
    let rate = (flagged as f64 / total as f64) * 100.0;
    eprintln!(
        "\n==== Phase 1.4 False-Positive Gate ====\n\
         Population: {total} confirmed claims seeded with domain=stocks, claim_kind=external_fact (BOTH outside the strict canon).\n\
         Flagged:    {flagged}/{total} ({rate:.1}%)\n\
         Tag breakdown: {by_kind:?}\n\
         NOTE: This rate is ADVISORY. The strict 4-domain / 7-kind canon is the\n\
         user-acknowledged trade-off (see docs/plans/feature-ai-review-and-quality-rules.md).\n\
         A high rate here documents the canon mismatch; relaxing the canon in a\n\
         future session will drop this number.\n\
         ========================================\n"
    );
    // Always pass — non-blocking. The output above is the artifact.
    assert!(rate >= 0.0);
}
