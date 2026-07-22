//! Phase E Task E3.2 — Console API routes for trust / operations / entity
//! mutations / hard purge / re-auth. Drives the real axum router over a
//! loopback OS-assigned port with `reqwest`, exercising every new route plus
//! the re-auth freshness gate that hard purge requires (§5.3 "recent re-auth").
//!
//! These tests assert the observable HTTP contract (status codes, JSON shapes,
//! auth/CSRF/reauth gating). The underlying producers, entity mutations, and
//! the purge saga are already covered by `trust_operations_contract_v1.rs`,
//! `semantic_ownership_v1.rs`, and `semantic_purge_v1.rs` — this file's job is
//! the API surface that wires them into `/api/v1/*`.

use std::sync::Arc;

use chrono::Duration;
use llm_wiki::api::{ConsoleApiState, router};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand,
    ProposeUserAssertionCommand, SemanticConfig, SemanticStore, TrustedContext,
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

/// Like [`make_store`] but configures two purge-registry target dirs so the
/// hard-purge saga can reach `completed` (the registry quorum requires >=2
/// targets). Used by the `/purge/execute` tests.
fn make_store_with_purge_targets() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let target_a = parent.path().join("target-a");
    let target_b = parent.path().join("target-b");
    std::fs::create_dir_all(&target_a).expect("mkdir target-a");
    std::fs::create_dir_all(&target_b).expect("mkdir target-b");
    let root = parent.path().join("semantic-store");
    let config = SemanticConfig::enabled_for(parent.path())
        .with_purge_registry_targets(vec![target_a, target_b]);
    let (store, _admin) = SemanticStore::create(&root, config).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

fn draft(subject: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: Some("stocks".to_owned()),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn seed_proposal(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    evidence: &str,
    subject: &str,
    value: i64,
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
                draft: draft(subject, value),
            },
        )
        .unwrap();
    outcome.generated.proposal_id.unwrap()
}

/// Seed a confirmed claim; returns `(claim_id, confirm_operation_id)`. The
/// confirm operation_id is what `/claim/{claim_operation_id}/retract` expects.
fn seed_confirmed(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    value: i64,
) -> (Uuid, String) {
    seed_proposal(store, context, op, "evidence text", subject, value);
    let confirm_op = format!("{op}-confirm");
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: confirm_op.clone(),
                proposal_operation_id: op.to_owned(),
            },
        )
        .unwrap();
    (outcome.generated.claim_id.unwrap(), confirm_op)
}

/// Seed a confirmed claim via user-assertion (needed for split/merge fixtures
/// where the subject is a non-default entity canonical subject). Returns the
/// confirm operation_id (for retract) and the claim_id.
fn seed_user_assertion(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    predicate: &str,
    value: i64,
) -> (Uuid, String) {
    store
        .propose_user_assertion(
            context,
            ProposeUserAssertionCommand {
                operation_id: op.to_owned(),
                utterance: b"assertion text".to_vec(),
                draft: ClaimDraft {
                    subject: subject.to_owned(),
                    predicate: predicate.to_owned(),
                    value: json!(value),
                    claim_kind: "user_assertion".to_owned(),
                    domain: Some("stocks".to_owned()),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose_user_assertion");
    let confirm_op = format!("{op}-confirm");
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: confirm_op.clone(),
                proposal_operation_id: op.to_owned(),
            },
        )
        .expect("confirm");
    (outcome.generated.claim_id.unwrap(), confirm_op)
}

/// Confirms a claim end-to-end and returns its object_id — the content key
/// `/purge/preview` and `/purge/execute` operate on.
fn confirm_a_claim_object_id(store: &SemanticStore, context: &TrustedContext) -> String {
    store
        .capture(
            context,
            CaptureCommand {
                operation_id: "purge-cap".to_owned(),
                bytes: b"evidence bytes".to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .unwrap();
    store
        .propose(
            context,
            ProposeCommand {
                operation_id: "purge-prop".to_owned(),
                capture_operation_id: "purge-cap".to_owned(),
                draft: ClaimDraft {
                    subject: "project:brain".to_owned(),
                    predicate: "deployment".to_owned(),
                    value: json!("kubernetes"),
                    claim_kind: "decision".to_owned(),
                    domain: Some("projects".to_owned()),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .unwrap();
    let confirmed = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: "purge-confirm".to_owned(),
                proposal_operation_id: "purge-prop".to_owned(),
            },
        )
        .unwrap();
    confirmed.event.payload.object_id
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

/// Logs in with the given secret. Returns `(session_cookie, csrf_token)` where
/// `session_cookie` is the ready-to-send `Cookie:` header value.
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

/// Re-authenticates an existing session. `cookie` is the session cookie from a
/// prior `login`. Returns the response JSON on success.
async fn reauth(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    secret: &str,
) -> reqwest::Response {
    client
        .post(format!("{base}/api/v1/auth/reauth"))
        .header("Cookie", cookie)
        .json(&json!({ "secret": secret }))
        .send()
        .await
        .unwrap()
}

// ── /trust ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn trust_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/trust"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn trust_returns_empty_for_fresh_store() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/trust"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["contradictions"].as_array().unwrap().len(), 0);
    assert_eq!(body["stale"].as_array().unwrap().len(), 0);
    assert!(body["retrieval_trace"].is_null());
}

#[tokio::test]
async fn trust_returns_contradiction_after_seed() {
    let (_parent, store, ctx) = make_store();
    // Two divergent values for the same (domain, subject, predicate) scope.
    seed_confirmed(&store, &ctx, "p1", "GULF", 58);
    seed_confirmed(&store, &ctx, "p2", "GULF", 61);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/trust"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let contradictions = body["contradictions"].as_array().unwrap();
    assert_eq!(
        contradictions.len(),
        1,
        "expected one contradiction flag, got {contradictions:?}"
    );
    assert_eq!(contradictions[0]["kind"], "contradiction");
}

// ── /ops/clients ──────────────────────────────────────────────────────────

#[tokio::test]
async fn ops_clients_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/ops/clients"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ops_clients_returns_console_after_login() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/ops/clients"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("clients is a bare array");
    assert!(
        arr.iter().any(|entry| entry["label"] == "console"),
        "login should have registered a 'console' client: {arr:?}"
    );
}

// ── /ops/jobs ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn ops_jobs_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/ops/jobs"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ops_jobs_returns_zero_initially() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/ops/jobs"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["active"], 0);
    assert_eq!(body["queued"], 0);
    assert_eq!(body["failed"], 0);
}

// ── /ops/evals ────────────────────────────────────────────────────────────

#[tokio::test]
async fn ops_evals_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/ops/evals?domain=stocks"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ops_evals_requires_domain() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/ops/evals"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn ops_evals_returns_never_initially() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/ops/evals?domain=stocks"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["run_at"], "never");
}

// ── /ops/backup-health ────────────────────────────────────────────────────

#[tokio::test]
async fn ops_backup_health_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/ops/backup-health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn ops_backup_health_returns_never_false_initially() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/ops/backup-health"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["last_backup_at"], "never");
    assert_eq!(body["last_restore_drill_ok"], false);
}

// ── /entity/merge ─────────────────────────────────────────────────────────

#[tokio::test]
async fn entity_merge_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/entity/merge"))
        .json(&json!({ "source": Uuid::new_v4(), "target": Uuid::new_v4() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn entity_merge_requires_csrf() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/entity/merge"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", "wrong-token")
        .json(&json!({ "source": Uuid::new_v4(), "target": Uuid::new_v4() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn entity_merge_happy_path() {
    let (_parent, store, ctx) = make_store();
    let source = store
        .resolve_or_create_entity(&ctx, "GULF-dup")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&ctx, "GULF")
        .expect("target entity");
    // Seed a claim on the source so the merge has something to move.
    seed_user_assertion(&store, &ctx, "merge-src", "GULF-dup", "target_price", 58);

    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/entity/merge"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "source": source, "target": target }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "merged");
    assert!(body["event_seq"].as_u64().is_some());
}

// ── /entity/split ─────────────────────────────────────────────────────────

#[tokio::test]
async fn entity_split_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/entity/split"))
        .json(&json!({ "source": Uuid::new_v4(), "assignments": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn entity_split_requires_csrf() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/entity/split"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", "wrong-token")
        .json(&json!({ "source": Uuid::new_v4(), "assignments": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn entity_split_happy_path() {
    let (_parent, store, ctx) = make_store();
    let source = store
        .resolve_or_create_entity(&ctx, "GULF-combined")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&ctx, "GULF-prices")
        .expect("target entity");
    // Two claims on the source: one whose predicate we move, one we leave.
    seed_user_assertion(&store, &ctx, "split-1", "GULF-combined", "target_price", 58);
    seed_user_assertion(&store, &ctx, "split-2", "GULF-combined", "sector", 1);

    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/entity/split"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({
            "source": source,
            "assignments": [
                { "predicate": "target_price", "target_entity_id": target }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "split");
    assert!(body["event_seq"].as_u64().is_some());
    assert_eq!(body["moved_claim_count"], 1);
    assert_eq!(body["source_remaining_claim_count"], 1);
}

/// The divergent-duplicate-predicate guard (carry-over fix from E3.1 review)
/// surfaces through the API as a 409 `conflict` (InvalidTransition).
#[tokio::test]
async fn entity_split_rejects_duplicate_predicate() {
    let (_parent, store, ctx) = make_store();
    let source = store
        .resolve_or_create_entity(&ctx, "GULF-dup-src")
        .expect("source entity");
    let target_a = store
        .resolve_or_create_entity(&ctx, "GULF-dup-a")
        .expect("target A");
    let target_b = store
        .resolve_or_create_entity(&ctx, "GULF-dup-b")
        .expect("target B");

    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/entity/split"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({
            "source": source,
            "assignments": [
                { "predicate": "target_price", "target_entity_id": target_a },
                { "predicate": "target_price", "target_entity_id": target_b }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "conflict");
}

// ── /claim/{id}/retract ───────────────────────────────────────────────────

#[tokio::test]
async fn entity_retract_happy_path() {
    let (_parent, store, _ctx) = make_store();
    // retract resolves the claim by `(client_id, operation_id)`, so the claim
    // must be confirmed by the SAME client the Console session uses. login()
    // registers the "console" client; seeding via that same label shares the
    // client_id, making the confirm operation_id resolvable from the session.
    let console_ctx = store.register_client("console").expect("register console");
    let (_claim_id, confirm_op) = seed_confirmed(&store, &console_ctx, "p1", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/claim/{confirm_op}/retract"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "retracted");
    assert!(body["event_seq"].as_u64().is_some());
}

// ── /destructive/warning ──────────────────────────────────────────────────

#[tokio::test]
async fn destructive_warning_returns_hard_purge_message() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!(
            "{base}/api/v1/destructive/warning?action=hard_purge"
        ))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let message = body["message"].as_str().unwrap().to_lowercase();
    assert!(
        message.contains("no undo") || message.contains("cannot be recovered"),
        "warning must state no undo / cannot be recovered: {message}"
    );
    assert_eq!(body["irreversible"], true);
    assert_eq!(body["requires_recent_reauth"], true);
    assert_eq!(body["requires_two_step_nonce"], true);
    assert_eq!(body["action"], "hard_purge");
}

// ── /purge/preview ────────────────────────────────────────────────────────

#[tokio::test]
async fn purge_preview_requires_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .json(&json!({ "object_ids": ["sha256:abc"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn purge_preview_returns_preview_with_nonce() {
    let (_parent, store, _ctx) = make_store();
    // Seed a real object so the preview has a well-formed target.
    let object_id = confirm_a_claim_object_id(&store, &store.trusted_context());
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "object_ids": [object_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let preview = &body["preview"];
    assert!(preview["preview_hash"].as_str().is_some());
    assert!(preview["nonce"].as_str().is_some());
    assert!(preview["expires_at"].as_str().is_some());
    // The warning must travel alongside the preview (Task E3.2 DoD #2).
    let message = body["warning"]["message"]
        .as_str()
        .expect("warning.message present")
        .to_lowercase();
    assert!(
        message.contains("no undo") || message.contains("cannot be recovered"),
        "preview must carry the hard-purge warning: {message}"
    );
    assert_eq!(body["warning"]["requires_recent_reauth"], true);
}

// ── /purge/execute ────────────────────────────────────────────────────────

#[tokio::test]
async fn purge_execute_requires_recent_reauth() {
    let (_parent, store, _ctx) = make_store_with_purge_targets();
    let object_id = confirm_a_claim_object_id(&store, &store.trusted_context());
    // A reauth-freshness window of ZERO means every session is immediately
    // stale unless it re-authed in the same clock tick — login happened
    // before the gate, so the first execute attempt must be rejected.
    let state = ConsoleApiState::with_session_ttl_and_reauth_freshness(
        store,
        SECRET.to_owned(),
        false,
        Duration::hours(24),
        Duration::zero(),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    // Phase 1: preview (CSRF-only, no reauth gate).
    let preview_resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .header("Cookie", cookie.clone())
        .header("X-CSRF-Token", csrf.clone())
        .json(&json!({ "object_ids": [object_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), 200);
    let preview: Value = preview_resp.json().await.unwrap();
    let preview_hash = preview["preview"]["preview_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let nonce = preview["preview"]["nonce"].as_str().unwrap().to_owned();

    // Phase 2: execute WITHOUT re-auth → 403 reauth_required.
    let resp = client
        .post(format!("{base}/api/v1/purge/execute"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "preview_hash": preview_hash, "nonce": nonce }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"], "reauth_required",
        "execute without recent reauth must be reauth_required, got {body:?}"
    );
}

#[tokio::test]
async fn purge_execute_succeeds_after_reauth() {
    let (_parent, store, _ctx) = make_store_with_purge_targets();
    let object_id = confirm_a_claim_object_id(&store, &store.trusted_context());
    // Generous freshness so a re-auth within the test run stays fresh.
    let state = ConsoleApiState::with_session_ttl_and_reauth_freshness(
        store,
        SECRET.to_owned(),
        false,
        Duration::hours(24),
        Duration::minutes(5),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    // Re-auth first to satisfy the freshness gate.
    let reauth_resp = reauth(&client, &base, &cookie, SECRET).await;
    assert_eq!(reauth_resp.status(), 200);

    let preview_resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .header("Cookie", cookie.clone())
        .header("X-CSRF-Token", csrf.clone())
        .json(&json!({ "object_ids": [object_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), 200);
    let preview: Value = preview_resp.json().await.unwrap();
    let preview_hash = preview["preview"]["preview_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let nonce = preview["preview"]["nonce"].as_str().unwrap().to_owned();

    let resp = client
        .post(format!("{base}/api/v1/purge/execute"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "preview_hash": preview_hash, "nonce": nonce }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["state"], "completed");
    assert!(body["purge_id"].is_string());
}

#[tokio::test]
async fn purge_execute_requires_correct_nonce() {
    let (_parent, store, _ctx) = make_store_with_purge_targets();
    let object_id = confirm_a_claim_object_id(&store, &store.trusted_context());
    let state = ConsoleApiState::with_session_ttl_and_reauth_freshness(
        store,
        SECRET.to_owned(),
        false,
        Duration::hours(24),
        Duration::minutes(5),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");
    // Re-auth to clear the freshness gate, isolating the nonce check.
    let reauth_resp = reauth(&client, &base, &cookie, SECRET).await;
    assert_eq!(reauth_resp.status(), 200);

    let preview_resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .header("Cookie", cookie.clone())
        .header("X-CSRF-Token", csrf.clone())
        .json(&json!({ "object_ids": [object_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), 200);
    let preview: Value = preview_resp.json().await.unwrap();
    let preview_hash = preview["preview"]["preview_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    // Deliberately wrong nonce.
    let wrong_nonce = Uuid::new_v4().to_string();

    let resp = client
        .post(format!("{base}/api/v1/purge/execute"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "preview_hash": preview_hash, "nonce": wrong_nonce }))
        .send()
        .await
        .unwrap();
    // Unknown nonce → InvalidClaim → 400 invalid_request.
    assert!(
        resp.status().is_client_error() && resp.status() != 401 && resp.status() != 403,
        "wrong nonce must be a 4xx semantic error, got {}",
        resp.status()
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_request");
}

// ── /auth/reauth ──────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_reauth_requires_existing_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/auth/reauth"))
        .json(&json!({ "secret": SECRET }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn auth_reauth_wrong_secret() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = reauth(&client, &base, &cookie, "not-the-secret").await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn auth_reauth_refreshes_freshness() {
    let (_parent, store, _ctx) = make_store();
    // Pin a tiny freshness window so a subsequent /purge/execute would fail
    // without this reauth (the gate uses reauthenticated_at after reauth).
    let state = ConsoleApiState::with_session_ttl_and_reauth_freshness(
        store,
        SECRET.to_owned(),
        false,
        Duration::hours(24),
        Duration::zero(),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = reauth(&client, &base, &cookie, SECRET).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["reauthenticated"], true);
    assert_eq!(body["fresh_for_seconds"], 0);

    // A read route still works after reauth (session is intact).
    let ok = client
        .get(format!("{base}/api/v1/ops/jobs"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

// ── Carry-over from E3.2 review (filled in E3.3) ───────────────────────────
//
// Two test gaps surfaced in the E3.2 review: the round-trip from
// `/purge/execute` → `/purge/status` (proving the saga state survives the
// hand-off and is observable read-only), and the 404 path for retract when
// the confirm operation_id is unknown.

/// `/purge/status?purge_id=<id>` returns the same `PurgeReceipt` (same `state`
/// + `purge_id`) that `/purge/execute` returned. Reuses the
/// `purge_execute_succeeds_after_reauth` fixture (two-target registry so the
/// saga reaches `completed`).
#[tokio::test]
async fn purge_status_returns_receipt_after_execute() {
    let (_parent, store, _ctx) = make_store_with_purge_targets();
    let object_id = confirm_a_claim_object_id(&store, &store.trusted_context());
    let state = ConsoleApiState::with_session_ttl_and_reauth_freshness(
        store,
        SECRET.to_owned(),
        false,
        Duration::hours(24),
        Duration::minutes(5),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    // Re-auth + preview + execute → capture the receipt's purge_id.
    let reauth_resp = reauth(&client, &base, &cookie, SECRET).await;
    assert_eq!(reauth_resp.status(), 200);

    let preview_resp = client
        .post(format!("{base}/api/v1/purge/preview"))
        .header("Cookie", cookie.clone())
        .header("X-CSRF-Token", csrf.clone())
        .json(&json!({ "object_ids": [object_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), 200);
    let preview: Value = preview_resp.json().await.unwrap();
    let preview_hash = preview["preview"]["preview_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let nonce = preview["preview"]["nonce"].as_str().unwrap().to_owned();

    let execute_resp = client
        .post(format!("{base}/api/v1/purge/execute"))
        .header("Cookie", cookie.clone())
        .header("X-CSRF-Token", csrf.clone())
        .json(&json!({ "preview_hash": preview_hash, "nonce": nonce }))
        .send()
        .await
        .unwrap();
    assert_eq!(execute_resp.status(), 200);
    let executed: Value = execute_resp.json().await.unwrap();
    assert_eq!(executed["state"], "completed");
    let purge_id = executed["purge_id"].as_str().expect("purge_id is a string");

    // The carry-over assertion: GET /purge/status returns the SAME receipt
    // (state + purge_id) read-only. This is what the Console polls to
    // surface saga progress to the operator.
    let status_resp = client
        .get(format!("{base}/api/v1/purge/status?purge_id={purge_id}"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(status_resp.status(), 200);
    let status_body: Value = status_resp.json().await.unwrap();
    assert_eq!(status_body["purge_id"], purge_id);
    assert_eq!(status_body["state"], "completed");
    // The composite_checksum + new_backup_path are populated on a completed
    // saga — assert presence (not exact value, which is content-dependent).
    assert!(
        status_body["composite_checksum"].as_str().is_some(),
        "completed receipt must carry a composite_checksum: {status_body:?}"
    );
}

/// `/claim/{unknown-op-id}/retract` → 404 `not_found`. The path's
/// `claim_operation_id` resolves to no stored confirm outcome; the
/// resulting `InvalidTransition` (claim not found) surfaces as 404 via the
/// semantic-error mapper.
#[tokio::test]
async fn entity_retract_unknown_returns_404() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let unknown_op = format!("unknown-{}", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/v1/claim/{unknown_op}/retract"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"], "not_found",
        "unknown confirm operation_id must map to not_found, got {body:?}"
    );
}
