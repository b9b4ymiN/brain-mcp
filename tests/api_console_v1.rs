//! Phase E Task E0.2 — Console HTTP JSON API (`/api/v1/*`). Drives the real
//! axum router over a loopback OS-assigned port with `reqwest`, exercising the
//! dev-grade session-cookie + double-submit-CSRF auth model and every read /
//! review route. The API must reach storage ONLY through `SemanticStore`
//! public methods (§9); these tests assert the observable HTTP contract, a
//! separate grep gate (DoD item 3) asserts the no-direct-write property.

use std::sync::Arc;

use chrono::Duration;
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

fn draft(subject: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: "stocks".to_owned(),
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

fn seed_confirmed(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    subject: &str,
    value: i64,
) -> Uuid {
    seed_proposal(store, context, op, "evidence text", subject, value);
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
    // "brain_console_session=<id>; HttpOnly; ..." -> "brain_console_session=<id>"
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    assert!(set_cookie.contains("HttpOnly"), "cookie must be HttpOnly");
    assert!(
        set_cookie.contains("SameSite=Strict"),
        "cookie must be SameSite=Strict"
    );
    let body: Value = resp.json().await.unwrap();
    let csrf = body["csrf_token"].as_str().unwrap().to_owned();
    Some((cookie, csrf))
}

/// Phase G (2026-07-20): logs in with the new username+password shape.
/// Mirrors [`login`] but POSTs `{username, password}`. Used to exercise the
/// dual-mode handler against state built with `with_credentials`.
async fn login_with_credentials(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> Option<(String, String)> {
    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "username": username, "password": password }))
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

// ── auth: negative ──────────────────────────────────────────────────────────

#[tokio::test]
async fn login_wrong_secret_returns_401() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "secret": "not-the-secret" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── Phase G (2026-07-20): username+password login mode ─────────────────────
//
// `ConsoleApiState::with_credentials(store, Some(username), password, _)`
// turns on username+password mode: the login route requires BOTH fields to
// match, and the legacy `{secret}` body is rejected. These tests cover the
// happy path + both rejection paths (wrong username, wrong password).

const USERNAME: &str = "console-admin";

#[tokio::test]
async fn login_with_credentials_succeeds() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::with_credentials(
        store,
        Some(USERNAME.to_owned()),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let result = login_with_credentials(&client, &base, USERNAME, SECRET).await;
    assert!(result.is_some(), "correct username+password → 200 + csrf");
}

#[tokio::test]
async fn login_with_wrong_username_returns_401() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::with_credentials(
        store,
        Some(USERNAME.to_owned()),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "username": "wrong-user", "password": SECRET }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn login_with_wrong_password_returns_401() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::with_credentials(
        store,
        Some(USERNAME.to_owned()),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "username": USERNAME, "password": "wrong-pw" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn login_username_mode_rejects_legacy_secret_body() {
    // When username mode is ON, an old client posting `{secret}` is rejected,
    // even if `secret` matches the configured password. This forces the
    // migration: nobody silently authenticates without identifying themselves.
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::with_credentials(
        store,
        Some(USERNAME.to_owned()),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "secret": SECRET }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn login_legacy_mode_still_accepts_secret_body() {
    // Backward-compat: state built with `::new` (no username) must still
    // accept `{secret}` alone. Old deployments and old CLI clients keep
    // working through the migration.
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let result = login(&client, &base, SECRET).await;
    assert!(result.is_some(), "legacy single-credential mode still works");
}

#[tokio::test]
async fn no_session_returns_401_on_every_protected_route() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let pid = Uuid::new_v4();

    let gets = [
        format!("{base}/api/v1/search?query=x"),
        format!("{base}/api/v1/get?subject=x"),
        format!("{base}/api/v1/entity/timeline?domain=d&subject=s&predicate=p"),
        format!("{base}/api/v1/inbox"),
        format!("{base}/api/v1/inbox/{pid}/evidence"),
    ];
    for url in gets {
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), 401, "GET {url} without session must be 401");
    }

    let posts = [
        format!("{base}/api/v1/inbox/{pid}/approve"),
        format!("{base}/api/v1/inbox/{pid}/reject"),
        format!("{base}/api/v1/inbox/{pid}/supersede"),
        format!("{base}/api/v1/auth/logout"),
    ];
    for url in posts {
        let resp = client.post(&url).send().await.unwrap();
        assert_eq!(resp.status(), 401, "POST {url} without session must be 401");
    }
}

#[tokio::test]
async fn garbage_session_cookie_returns_401() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/inbox"))
        .header("Cookie", "brain_console_session=not-a-real-session")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn expired_session_returns_401() {
    let (_parent, store, _ctx) = make_store();
    // TTL zero => the session is already past-expiry on the next lookup.
    let state =
        ConsoleApiState::with_session_ttl(store, SECRET.to_owned(), false, Duration::zero());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let resp = client
        .get(format!("{base}/api/v1/inbox"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── CSRF: negative ──────────────────────────────────────────────────────────

/// The three mutating routes, as `(url_suffix, has_body)`. CSRF is enforced by
/// a shared extractor that fires before the body is read, so a missing/bad
/// token must 403 on all of them regardless of payload.
fn mutating_routes(pid: Uuid) -> [String; 3] {
    [
        format!("inbox/{pid}/approve"),
        format!("inbox/{pid}/reject"),
        format!("inbox/{pid}/supersede"),
    ]
}

#[tokio::test]
async fn missing_csrf_returns_403_on_every_mutating_route() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_proposal(&store, &ctx, "p1", "ev", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    for suffix in mutating_routes(pid) {
        let resp = client
            .post(format!("{base}/api/v1/{suffix}"))
            .header("Cookie", cookie.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403, "{suffix} without CSRF must be 403");
    }
}

#[tokio::test]
async fn bad_csrf_returns_403_on_every_mutating_route() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_proposal(&store, &ctx, "p1", "ev", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    for suffix in mutating_routes(pid) {
        let resp = client
            .post(format!("{base}/api/v1/{suffix}"))
            .header("Cookie", cookie.clone())
            .header("X-CSRF-Token", "wrong-token")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403, "{suffix} with bad CSRF must be 403");
    }
}

// ── read routes: happy path ────────────────────────────────────────────────

#[tokio::test]
async fn search_happy_path() {
    let (_parent, store, ctx) = make_store();
    seed_confirmed(&store, &ctx, "p1", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/search?query=gulf"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["count"], 1);
    assert_eq!(body["results"][0]["subject"], "GULF");
}

#[tokio::test]
async fn get_happy_path() {
    let (_parent, store, ctx) = make_store();
    seed_confirmed(&store, &ctx, "p1", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/get?subject=GULF&domain=stocks"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["count"], 1);
    assert_eq!(body["claims"][0]["subject"], "GULF");
}

#[tokio::test]
async fn timeline_happy_path() {
    let (_parent, store, ctx) = make_store();
    seed_confirmed(&store, &ctx, "p1", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!(
            "{base}/api/v1/entity/timeline?domain=stocks&subject=GULF&predicate=target_price"
        ))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["value"], 58);
}

#[tokio::test]
async fn inbox_happy_path() {
    let (_parent, store, ctx) = make_store();
    seed_proposal(&store, &ctx, "p1", "ev", "GULF", 58);
    seed_proposal(&store, &ctx, "p2", "ev", "PTT", 70);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/inbox"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn evidence_happy_path() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_proposal(&store, &ctx, "p1", "GULF target raised to 58", "GULF", 58);
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/inbox/{pid}/evidence"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["provenance_kind"], "evidence");
    assert_eq!(body["excerpt"], "GULF target raised to 58");
}

// ── review routes: happy path ──────────────────────────────────────────────

#[tokio::test]
async fn approve_happy_path() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_proposal(&store, &ctx, "p1", "ev", "GULF", 58);
    let base = spawn(ConsoleApiState::new(
        store.clone(),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/inbox/{pid}/approve"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["claim_id"].is_string());

    // The proposal is now off the inbox.
    assert!(store.list_pending_proposals().unwrap().is_empty());
}

#[tokio::test]
async fn reject_happy_path() {
    let (_parent, store, ctx) = make_store();
    let pid = seed_proposal(&store, &ctx, "p1", "ev", "GULF", 58);
    let base = spawn(ConsoleApiState::new(
        store.clone(),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/inbox/{pid}/reject"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(store.list_pending_proposals().unwrap().is_empty());
}

#[tokio::test]
async fn supersede_happy_path() {
    let (_parent, store, ctx) = make_store();
    let old_claim = seed_confirmed(&store, &ctx, "p1", "GULF", 58);
    let pid = seed_proposal(&store, &ctx, "p2", "ev", "GULF", 62);
    let base = spawn(ConsoleApiState::new(
        store.clone(),
        SECRET.to_owned(),
        false,
    ))
    .await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/inbox/{pid}/supersede"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "superseded_claim_ids": [old_claim.to_string()] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["claim_id"].is_string());

    let timeline = store
        .claim_timeline("stocks", "GULF", "target_price")
        .unwrap();
    assert_eq!(timeline.len(), 2);
}

#[tokio::test]
async fn approve_unknown_proposal_returns_404() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");
    let unknown = Uuid::new_v4();

    let resp = client
        .post(format!("{base}/api/v1/inbox/{unknown}/approve"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn logout_clears_session() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    // Session works before logout.
    let ok = client
        .get(format!("{base}/api/v1/inbox"))
        .header("Cookie", cookie.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    let logout = client
        .post(format!("{base}/api/v1/auth/logout"))
        .header("Cookie", cookie.clone())
        .send()
        .await
        .unwrap();
    assert!(logout.status().is_success());

    // Same cookie is now dead server-side.
    let after = client
        .get(format!("{base}/api/v1/inbox"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 401);
}

// ── SSE events ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn sse_receives_published_event() {
    let (_parent, store, _ctx) = make_store();
    let state = ConsoleApiState::new(store, SECRET.to_owned(), false);
    // Keep a publisher handle before the state is moved into the router. In
    // production anything in-process could hold this and push job/update events.
    let events = state.events_sender();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let mut resp = client
        .get(format!("{base}/api/v1/events"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ctype = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        ctype.starts_with("text/event-stream"),
        "unexpected content-type: {ctype}"
    );

    // Publish only after the client is connected — a broadcast channel reaches
    // live subscribers, and the handler subscribes before the response head is
    // flushed (which is what `send().await` above resolved on).
    events.send("ping-42".to_owned()).expect("send event");

    let mut buf = String::new();
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(bytes) = resp.chunk().await.unwrap() {
            buf.push_str(&String::from_utf8_lossy(&bytes));
            if buf.contains("ping-42") {
                break;
            }
        }
    })
    .await;
    assert!(got.is_ok(), "timed out waiting for SSE event; buf={buf:?}");
    assert!(
        buf.contains("data: ping-42"),
        "SSE frame not well-formed: {buf:?}"
    );
}

#[tokio::test]
async fn sse_without_session_returns_401() {
    let (_parent, store, _ctx) = make_store();
    let base = spawn(ConsoleApiState::new(store, SECRET.to_owned(), false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}
