//! AI Pre-Review Phase 3.3 — `AiQualityChecker` mock-provider tests.
//!
//! Drives the real `AiQualityChecker` (which calls the provider via the
//! `AiProvider` trait) with a hand-written mock that returns canned JSON.
//! Asserts the four postures from the Phase 3 spec:
//!   1. valid response → tags parsed, provider called once, `ran_to_completion=true`.
//!   2. local_only egress (user_assertion + empty excerpt) → no provider call.
//!   3. detected secret in evidence excerpt → no provider call.
//!   4. malformed JSON → provider called once, returns empty + `ran_to_completion=false`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use llm_wiki::provider::{AiProvider, ProviderRequest, ProviderResult};
use llm_wiki::quality::{AiQualityChecker, QualityCheckerInput, QualityTagKind};
use llm_wiki::semantic::{EvidenceSummary, ProposalSummary};

// ── MockProvider (duplicated from tests/api_ai_review_v1.rs by design — a
//     shared tests/common/mod.rs would force touching every existing test
//     file that already declares its own fixture module, more churn than
//     ~20 lines of duplication is worth) ──────────────────────────────────

/// Mock provider that returns a canned response and counts calls. Clonable
/// only via the outer `Arc<MockProvider>` the tests hold — the inner state
/// lives behind atomics so a `&self` `complete` can mutate the counter
/// without `Arc::get_mut`.
struct MockProvider {
    response: String,
    call_count: AtomicUsize,
}

impl MockProvider {
    fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            call_count: AtomicUsize::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl AiProvider for MockProvider {
    fn complete(&self, _request: &ProviderRequest) -> ProviderResult<String> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
    fn adapter_name(&self) -> &str {
        "mock"
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────

fn proposal_with_excerpt(
    excerpt: &str,
    provenance_kind: &str,
) -> (ProposalSummary, EvidenceSummary) {
    let p = ProposalSummary {
        proposal_id: Uuid::new_v4(),
        domain: "financial".to_string(),
        subject: "CATL".to_string(),
        predicate: "Q1 2026 gross margin".to_string(),
        value: json!("24%"),
        claim_kind: "financial_metric".to_string(),
        provenance_kind: provenance_kind.to_string(),
        submitted_at: Utc::now(),
        event_seq: 1,
    };
    let excerpt_opt = if excerpt.is_empty() {
        None
    } else {
        Some(excerpt.to_string())
    };
    let e = EvidenceSummary {
        provenance_kind: provenance_kind.to_string(),
        excerpt: excerpt_opt,
        source_id: None,
        quote_hash: None,
        value_located: false,
        value_offset: None,
        value_len: None,
        excerpt_truncated: false,
        additional_sources: Vec::new(),
    };
    (p, e)
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ai_check_parses_tags_from_provider_response() {
    let response = r#"{"tags": [{"kind": "source_claim_mismatch", "severity": "warning", "message": "26% not found in evidence"}]}"#;
    let mock = Arc::new(MockProvider::new(response));
    let checker = AiQualityChecker::new(mock.clone() as Arc<dyn AiProvider>);
    let (p, e) = proposal_with_excerpt("CATL margin was 24%", "inference");
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &e,
        existing_claims: &[],
    };
    let (tags, ran) = checker.check(&input).await;
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].kind, QualityTagKind::SourceClaimMismatch);
    assert!(ran, "provider ran to completion");
    assert_eq!(mock.calls(), 1, "provider should have been called once");
}

#[tokio::test]
async fn ai_check_local_only_returns_empty_no_provider_call() {
    // A user_assertion proposal with no excerpt → local_only heuristic trips
    // → egress denied → provider never called.
    let mock = Arc::new(MockProvider::new(
        r#"{"tags":[{"kind":"semantic_duplicate"}]}"#,
    ));
    let checker = AiQualityChecker::new(mock.clone() as Arc<dyn AiProvider>);
    let (p, e) = proposal_with_excerpt("", "user_assertion");
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &e,
        existing_claims: &[],
    };
    let (tags, ran) = checker.check(&input).await;
    assert!(tags.is_empty(), "denied egress should return no tags");
    assert!(!ran, "denied egress must report ran_to_completion=false");
    assert_eq!(
        mock.calls(),
        0,
        "provider must NOT be called when egress is denied"
    );
}

#[tokio::test]
async fn ai_check_detected_secret_in_evidence_returns_empty() {
    // Evidence excerpt containing a Bearer token → detect_secret trips →
    // egress denied. The `Bearer ` marker is one of detect_secret's patterns
    // (provider.rs:173), so this fixture deterministically denies.
    let mock = Arc::new(MockProvider::new(
        r#"{"tags":[{"kind":"provenance_loss"}]}"#,
    ));
    let checker = AiQualityChecker::new(mock.clone() as Arc<dyn AiProvider>);
    let (p, e) = proposal_with_excerpt("leaked creds: Bearer sk-ant-api03-abc123XYZ", "inference");
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &e,
        existing_claims: &[],
    };
    let (tags, ran) = checker.check(&input).await;
    assert!(
        tags.is_empty(),
        "secret-bearing evidence should be denied egress"
    );
    assert!(!ran);
    assert_eq!(mock.calls(), 0);
}

#[tokio::test]
async fn ai_check_provider_malformed_json_returns_empty() {
    let mock = Arc::new(MockProvider::new("not json at all"));
    let checker = AiQualityChecker::new(mock.clone() as Arc<dyn AiProvider>);
    let (p, e) = proposal_with_excerpt("clean evidence", "inference");
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &e,
        existing_claims: &[],
    };
    let (tags, ran) = checker.check(&input).await;
    assert!(tags.is_empty(), "malformed JSON should yield empty tags");
    assert!(
        !ran,
        "unparseable response must report ran_to_completion=false"
    );
    assert_eq!(
        mock.calls(),
        1,
        "provider WAS called but the response was unparseable"
    );
}
