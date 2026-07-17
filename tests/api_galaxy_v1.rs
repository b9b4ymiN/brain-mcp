//! Phase E Task E2.1 — `GET /api/v1/galaxy` integration tests.
//!
//! Drives the real axum router over a loopback OS-assigned port with
//! `reqwest`. The endpoint must (a) require a session, (b) honor the LOD
//! cap (≤300 / ≤2000), (c) support ego mode with `zoom=close&focus=…`,
//! (d) honor the domain filter, (e) return a payload whose aggregated edge
//! count equals the raw fixture edge count (§9.2 "aggregated edges ตรงกับ
//! raw fixture 100%"). The edge-inference heuristic is the value-string
//! reference: a claim whose `value` is a JSON string naming another
//! entity's canonical subject emits one `Related` edge — see
//! [`GalaxyGraph::from_claims`] in `src/galaxy.rs` for the rationale.

use std::sync::Arc;

use llm_wiki::api::{ConsoleApiState, router};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticStore, TrustedContext,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

const SECRET: &str = "dev-bootstrap-secret";

// ── fixtures ──────────────────────────────────────────────────────────────

fn make_store() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

/// Seed a confirmed claim with a fully-parameterized draft so we can build
/// value-reference fixtures (value as a JSON string naming another entity).
fn seed_claim(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    predicate: &str,
    value: Value,
    domain: &str,
) {
    store
        .capture(
            context,
            CaptureCommand {
                operation_id: format!("{op}-cap"),
                bytes: b"evidence".to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .unwrap();
    store
        .propose(
            context,
            ProposeCommand {
                operation_id: op.to_owned(),
                capture_operation_id: format!("{op}-cap"),
                draft: ClaimDraft {
                    subject: subject.to_owned(),
                    predicate: predicate.to_owned(),
                    value,
                    claim_kind: "external_fact".to_owned(),
                    domain: domain.to_owned(),
                    confidence_basis_points: 8_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .unwrap();
    store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: format!("{op}-confirm"),
                proposal_operation_id: op.to_owned(),
            },
        )
        .unwrap();
}

/// Resolve the stable entity_id behind a `(domain, subject)` after seeding.
fn entity_id_for(
    store: &SemanticStore,
    context: &TrustedContext,
    domain: &str,
    subject: &str,
) -> Uuid {
    store.resolve_entity(context, domain, subject).unwrap()
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

/// Logs in with the given secret. Returns `(session_cookie, csrf_token)`.
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
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .expect("login sets a cookie")
        .to_str()
        .unwrap();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    let body: Value = resp.json().await.unwrap();
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    Some((cookie, csrf))
}

// ── 1. session required (DoD #3) ──────────────────────────────────────────

#[tokio::test]
async fn galaxy_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/galaxy"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "GET /galaxy without session must be 401"
    );
}

// ── 2. far zoom returns bounded nodes (DoD #1) ────────────────────────────

#[tokio::test]
async fn galaxy_far_returns_bounded_nodes() {
    let (_parent, store, ctx) = make_store();
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "target_price",
        json!(58),
        "stocks",
    );
    seed_claim(
        &store,
        &ctx,
        "p2",
        "PTT",
        "target_price",
        json!(70),
        "stocks",
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=far"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["lod"], "community_supernodes");
    assert_eq!(body["max_nodes"], 300);
    assert!(
        body["node_count"].as_u64().unwrap() <= 300,
        "node_count must respect the LOD cap"
    );
    // Each node carries id/label/kind/domain (color is not the only signal).
    let nodes = body["nodes"].as_array().expect("nodes is array");
    assert!(!nodes.is_empty(), "seeded entities must appear");
    for n in nodes {
        assert!(n["id"].is_string(), "node has id");
        assert!(n["label"].is_string(), "node has label");
        assert!(n["kind"].is_string(), "node has kind");
        assert!(n["domain"].is_string(), "node has domain");
    }
}

// ── 3. mid zoom cap (DoD #1) ──────────────────────────────────────────────

#[tokio::test]
async fn galaxy_mid_caps_at_2000() {
    let (_parent, store, ctx) = make_store();
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "target_price",
        json!(58),
        "stocks",
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=mid"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["lod"], "visible_nodes");
    assert_eq!(body["max_nodes"], 2000, "mid LOD cap is 2000");
    assert!(
        body["node_count"].as_u64().unwrap() <= 2000,
        "node_count must respect the LOD cap"
    );
}

// ── 4. close zoom requires focus (DoD #1) ─────────────────────────────────

#[tokio::test]
async fn galaxy_close_ego_requires_focus() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=close"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "zoom=close without focus must be 400 invalid_request"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

// ── 5. close zoom with focus returns the focus entity ─────────────────────

#[tokio::test]
async fn galaxy_close_ego_returns_focus_entity() {
    let (_parent, store, ctx) = make_store();
    // GULF references PTT via a string value → PTT is a depth-1 neighbor.
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "rival_of",
        json!("PTT"),
        "stocks",
    );
    seed_claim(
        &store,
        &ctx,
        "p2",
        "PTT",
        "target_price",
        json!(70),
        "stocks",
    );
    let gulf_id = entity_id_for(&store, &ctx, "stocks", "GULF");
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=close&focus={gulf_id}"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["lod"], "ego_neighborhood");
    let nodes = body["nodes"].as_array().expect("nodes is array");
    let ids: Vec<&str> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert!(
        ids.contains(&gulf_id.to_string().as_str()),
        "focus entity must be in nodes: {ids:?}"
    );
}

// ── 6. domain filter ──────────────────────────────────────────────────────

#[tokio::test]
async fn galaxy_domain_filter() {
    let (_parent, store, ctx) = make_store();
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "target_price",
        json!(58),
        "stocks",
    );
    seed_claim(
        &store,
        &ctx,
        "p2",
        "BTC",
        "target_price",
        json!(50_000),
        "crypto",
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?domain=stocks"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "only the stocks entity survives the filter");
    assert_eq!(nodes[0]["domain"], "stocks");
}

// ── 7. invalid zoom → 400 ─────────────────────────────────────────────────

#[tokio::test]
async fn galaxy_invalid_zoom_400() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=invalid"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

// ── 8. parity: aggregated edge count = raw fixture count (DoD #2) ─────────
//
// Fixture: GULF → PTT → ADVANC (3 claims, 3 distinct value-reference edges).
// The payload's `edges.len()` must equal 3, the count predicted by direct
// inspection of the claims. No fabrication, no loss.

#[tokio::test]
async fn galaxy_parity_edge_count_matches_raw_fixture() {
    let (_parent, store, ctx) = make_store();
    // GULF's value names PTT; PTT's value names ADVANC; ADVANC's value names
    // GULF (closes the triangle). Three distinct edges, deterministically.
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "rival_of",
        json!("PTT"),
        "stocks",
    );
    seed_claim(
        &store,
        &ctx,
        "p2",
        "PTT",
        "owns_stake_in",
        json!("ADVANC"),
        "stocks",
    );
    seed_claim(
        &store,
        &ctx,
        "p3",
        "ADVANC",
        "supplier_to",
        json!("GULF"),
        "stocks",
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=mid"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        3,
        "parity: 3 value-reference edges predicted, 3 emitted"
    );
    // All edges are `Related` (the only kind E2.1 infers from ClaimView).
    for e in edges {
        assert_eq!(e["kind"], "related", "E2.1 edges are Related");
    }
}

// ── 9.ego with unknown focus returns empty (200, not 404) ─────────────────

#[tokio::test]
async fn galaxy_close_ego_unknown_focus_returns_empty() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let unknown = Uuid::new_v4();

    let resp = client
        .get(format!("{base}/api/v1/galaxy?zoom=close&focus={unknown}"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["node_count"], 0, "unknown focus → empty graph");
    assert_eq!(body["lod"], "ego_neighborhood");
}

// ── 10. default zoom is far ───────────────────────────────────────────────

#[tokio::test]
async fn galaxy_default_zoom_is_far() {
    let (_parent, store, ctx) = make_store();
    seed_claim(
        &store,
        &ctx,
        "p1",
        "GULF",
        "target_price",
        json!(58),
        "stocks",
    );
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/galaxy"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["lod"], "community_supernodes", "default zoom is far");
    assert_eq!(body["max_nodes"], 300);
}
