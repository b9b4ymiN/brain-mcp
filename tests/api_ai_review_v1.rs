//! AI Pre-Review Phase 2.2 — `GET /inbox/{proposal_id}/ai-review` HTTP contract.
//!
//! Drives the real axum router over loopback with reqwest. Mirrors the
//! pattern in tests/api_console_v1.rs (make_store / spawn / login / seed_*).
//! Asserts 401 (no session), 404 (unknown id), 200 (dirty + clean), and
//! the response shape (ai_used=false, checker_version, RFC3339 checked_at).

use std::sync::Arc;

use llm_wiki::api::{ConsoleApiState, router};
use llm_wiki::quality::QUALITY_CHECKER_VERSION;
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticStore, TrustedContext,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

const SECRET: &str = "dev-bootstrap-secret";

// ── fixtures (copied + adapted from tests/api_console_v1.rs) ──────────────

fn make_store() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

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

#[allow(dead_code)] // kept symmetric with api_console_v1.rs; used by future tests
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

async fn spawn(state: ConsoleApiState) -> String {
    let app = axum::Router::new().nest("/api/v1", router(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn login(client: &reqwest::Client, base: &str, secret: &str) -> Option<(String, String)> {
    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "secret": secret }))
        .send()
        .await
        .unwrap();
    if resp.status() != 200 {
        return None;
    }
    let set_cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    let body: Value = resp.json().await.unwrap();
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    Some((cookie, csrf))
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ai_review_returns_401_without_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let fake_id = Uuid::new_v4();
    let resp = client
        .get(format!("{base}/api/v1/inbox/{fake_id}/ai-review"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ai_review_returns_404_for_unknown_id() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let fake_id = Uuid::new_v4();
    let resp = client
        .get(format!("{base}/api/v1/inbox/{fake_id}/ai-review"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn ai_review_returns_200_with_tags_for_dirty_proposal() {
    let (_parent, store, ctx) = make_store();
    // Seed a dirty CATL margin proposal: predicate "margin" is vague AND
    // domain=stocks is outside canon → multiple tags expected.
    let pid = seed_dirty_proposal(
        &store,
        &ctx,
        "p1",
        "evidence text",
        custom_draft(
            "CATL",
            "margin",
            json!("24%"),
            "external_fact",
            "stocks",
            8_000,
        ),
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let resp = client
        .get(format!("{base}/api/v1/inbox/{pid}/ai-review"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let tags = body["tags"].as_array().expect("tags is array");
    assert!(
        !tags.is_empty(),
        "dirty proposal must produce >=1 tag, got {tags:?}"
    );
    // Verify at least one tag is a known kind (round-trip the snake_case serde).
    let kinds: Vec<&str> = tags
        .iter()
        .map(|t| t["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(
        kinds.iter().any(|k| matches!(
            *k,
            "duplicate_predicate"
                | "packed_facts"
                | "vague_predicate"
                | "taxonomy_drift"
                | "confidence_too_high"
                | "double_bracket"
                | "kind_mismatch"
        )),
        "expected a known deterministic kind, got {kinds:?}"
    );
}

#[tokio::test]
async fn ai_review_response_shape() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_dirty_proposal(
        &store,
        &ctx,
        "p1",
        "evidence text",
        custom_draft(
            "CATL",
            "margin",
            json!("24%"),
            "external_fact",
            "stocks",
            8_000,
        ),
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let resp = client
        .get(format!("{base}/api/v1/inbox/{pid}/ai-review"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    // Shape checks
    assert_eq!(body["proposal_id"].as_str(), Some(pid.to_string().as_str()));
    assert_eq!(body["ai_used"].as_bool(), Some(false));
    assert_eq!(
        body["checker_version"].as_str(),
        Some(QUALITY_CHECKER_VERSION)
    );
    let checked_at = body["checked_at"].as_str().expect("checked_at is string");
    // RFC3339 parse check
    chrono::DateTime::parse_from_rfc3339(checked_at).expect("checked_at parses as RFC3339");
    // Each tag has kind+severity+message (evidence optional)
    if let Some(tags) = body["tags"].as_array() {
        for t in tags {
            assert!(t["kind"].is_string(), "tag.kind is string: {t:?}");
            assert!(t["severity"].is_string(), "tag.severity is string: {t:?}");
            assert!(t["message"].is_string(), "tag.message is string: {t:?}");
        }
    }
}

#[tokio::test]
async fn ai_review_returns_200_for_clean_fact_but_strict_canon_flag() {
    // The strict 4-domain canon (Phase 1) means a "clean fact" like
    // USDTHB-2026-07-20 has_rate 33.59 in domain "fx" STILL gets a
    // TaxonomyDrift tag. This test documents that behavior — Phase 1 is
    // not yet "clean fact -> no tags" until the canon relaxes.
    let (_parent, store, ctx) = make_store();
    let pid = seed_dirty_proposal(
        &store,
        &ctx,
        "p1",
        "FX rate evidence",
        custom_draft(
            "USDTHB-2026-07-20",
            "has_rate",
            json!("33.59"),
            "external_fact",
            "fx",
            8_000,
        ),
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let resp = client
        .get(format!("{base}/api/v1/inbox/{pid}/ai-review"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let kinds: Vec<&str> = body["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap_or(""))
        .collect();
    // The content rules should NOT fire on a clean rate value:
    assert!(
        !kinds.contains(&"packed_facts"),
        "no packed_facts: {kinds:?}"
    );
    assert!(
        !kinds.contains(&"vague_predicate"),
        "no vague_predicate: {kinds:?}"
    );
    assert!(
        !kinds.contains(&"double_bracket"),
        "no double_bracket: {kinds:?}"
    );
    // But taxonomy_drift WILL fire (fx is outside the strict canon):
    assert!(
        kinds.contains(&"taxonomy_drift"),
        "strict canon flags fx domain; this is the user-acknowledged trade-off: {kinds:?}"
    );
}
