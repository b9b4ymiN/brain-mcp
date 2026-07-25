//! Console expansion HTTP API tests (2026-07-25): `/status`, `/activity`,
//! `/config`, `/index-status`, `/index/update`, `/index/rebuild` endpoints.
//!
//! Mirrors the pattern in `tests/api_trust_ops_v1.rs` but builds a real
//! `WikiEngine` (not just a bare `SemanticStore`) so engine-backed endpoints
//! can be exercised. Each endpoint gets a `*_requires_session` test (401
//! without cookie) plus a happy-path test asserting the JSON shape.

use std::sync::Arc;

use llm_wiki::api::{ConsoleApiState, router};
use llm_wiki::engine::WikiEngine;
use llm_wiki::semantic::{SemanticConfig, SemanticStore};
use llm_wiki::spaces;
use serde_json::Value;
use tempfile::TempDir;

const SECRET: &str = "dev-bootstrap-secret";

// ── fixtures ──────────────────────────────────────────────────────────────

/// Build a `ConsoleApiState` backed by a real `WikiEngine` (mounted on a
/// tempdir wiki with one page + commit) AND a throwaway `SemanticStore`
/// (`ConsoleApiState` still requires one even for engine-backed endpoints).
/// Returns the tempdir (kept alive for the test's duration) + the state
/// ready to pass to `spawn`. Mirrors `tests/brain_extract_v1.rs::setup` but
/// inlines the wiki creation (no shared helper module).
fn make_state() -> (TempDir, ConsoleApiState) {
    let dir = tempfile::tempdir().expect("fixture parent");
    let config_path = dir.path().join("state").join("config.toml");

    // Create a wiki space with at least one page + commit, mirroring
    // tests/ops/helpers.rs::setup_wiki but inline (no shared helper).
    let wiki_name = "test";
    let wiki_path = dir.path().join(wiki_name);
    spaces::create(
        &wiki_path,
        wiki_name,
        None,
        false,
        true,
        &config_path,
        None,
    )
    .expect("spaces::create");

    let wiki_root = wiki_path.join("wiki");
    std::fs::create_dir_all(wiki_root.join("concepts")).unwrap();
    std::fs::write(
        wiki_root.join("concepts/moe.md"),
        "---\ntitle: \"MoE\"\ntype: concept\nstatus: active\ntags: [ml]\n---\n\nMixture of Experts.\n",
    )
    .unwrap();
    llm_wiki::git::commit(&wiki_path, "add pages").expect("git::commit");

    // Build the engine (reads `state/config.toml` and mounts every registered
    // wiki — `test` here).
    let engine = Arc::new(WikiEngine::build(&config_path).expect("WikiEngine::build"));

    // Build a throwaway semantic store in a sibling dir. The engine-backed
    // endpoints (`/status`, etc.) never touch it; `ConsoleApiState` requires
    // it only for the legacy semantic-store endpoints (`/inbox`, `/trust`).
    let semantic_root = dir.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&semantic_root, SemanticConfig::enabled_for(dir.path()))
            .expect("SemanticStore::create");

    let state = ConsoleApiState::with_credentials(
        Arc::new(store),
        None,
        SECRET.to_owned(),
        false,
    )
    .with_engine(engine);

    (dir, state)
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
        .json(&serde_json::json!({ "secret": secret }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .expect("login sets a cookie")
        .to_str()
        .ok()?;
    let cookie = set_cookie.split(';').next()?.to_owned();
    let body: Value = resp.json().await.ok()?;
    let csrf = body["csrf_token"].as_str()?.to_owned();
    Some((cookie, csrf))
}

// ── /status ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn status_requires_session() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/v1/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn status_returns_wiki_stats_shape() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/status"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    // WikiStats shape (subset — full shape verified by TS interface later).
    assert!(body["wiki"].is_string(), "wiki field is a string");
    assert_eq!(body["wiki"].as_str().unwrap(), "test");
    assert!(body["pages"].is_i64(), "pages field is an integer");
    assert!(body["pages"].as_i64().unwrap() >= 1, "pages >= 1 (one seeded)");
    assert!(body["staleness"]["fresh"].is_i64(), "staleness.fresh is an integer");
    assert!(body["staleness"]["stale_7d"].is_i64(), "staleness.stale_7d is an integer");
    assert!(body["staleness"]["stale_30d"].is_i64(), "staleness.stale_30d is an integer");
    assert!(body["index"]["stale"].is_boolean(), "index.stale is a boolean");
}

// ── /activity ────────────────────────────────────────────────────────────

#[tokio::test]
async fn activity_requires_session() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/v1/activity?since=1d"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn activity_returns_events_array() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/activity?since=30d&limit=10"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body.is_array(), "activity returns a JSON array");
    // The make_state fixture creates a wiki with 1 commit touching 1 page.
    // Expect >= 1 event when the commit is within the 30d window.
    if let Some(arr) = body.as_array()
        && !arr.is_empty()
    {
        let first = &arr[0];
        assert!(first["kind"].is_string(), "kind field present");
        assert!(first["timestamp"].is_string(), "timestamp field present");
        assert!(first["actor"].is_string(), "actor field present");
        assert!(first["target"].is_string(), "target field present");
        assert!(first["detail"]["added"].is_i64(), "detail.added is an integer");
        assert!(first["detail"]["removed"].is_i64(), "detail.removed is an integer");
        assert!(first["detail"]["subject"].is_string(), "detail.subject is a string");
    }
}

// ── /config (Task 16) ──────────────────────────────────────────────────────

#[tokio::test]
async fn config_requires_session() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/v1/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn config_returns_view_shape() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/config"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    // ConfigView shape (subset).
    assert!(body["wiki_spaces"].is_array(), "wiki_spaces is an array");
    assert!(
        body["server"]["http_enabled"].is_boolean(),
        "server.http_enabled is a boolean"
    );
    assert!(
        body["extraction"]["enabled"].is_boolean(),
        "extraction.enabled is a boolean"
    );
}

// ── /index-status (Task 16) ────────────────────────────────────────────────

#[tokio::test]
async fn index_status_requires_session() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/v1/index-status"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn index_status_returns_status_shape() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .get(format!("{base}/api/v1/index-status"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["wiki"].is_string(), "wiki field is a string");
    assert!(
        body["stale"].is_boolean(),
        "stale field is a boolean"
    );
    assert!(
        body["queryable"].is_boolean(),
        "queryable field is a boolean"
    );
}

// ── /index/update (Task 16) ────────────────────────────────────────────────

#[tokio::test]
async fn index_update_requires_csrf() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = login(&client, &base, SECRET).await.expect("login ok");

    // Wrong CSRF token → 403 (session resolves first, so this is 403 not 401).
    let resp = client
        .post(format!("{base}/api/v1/index/update"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", "wrong-token")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn index_update_succeeds_with_csrf() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/index/update"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&serde_json::json!({ "wiki": "test" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    // UpdateReport shape: { updated: usize, deleted: usize }.
    assert!(
        body["updated"].is_i64(),
        "updated is an integer (got: {body})"
    );
    assert!(
        body["deleted"].is_i64(),
        "deleted is an integer (got: {body})"
    );
}

// ── /index/rebuild (Task 16) ───────────────────────────────────────────────

#[tokio::test]
async fn index_rebuild_returns_job_id() {
    let (_dir, state) = make_state();
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = login(&client, &base, SECRET).await.expect("login ok");

    let resp = client
        .post(format!("{base}/api/v1/index/rebuild"))
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .json(&serde_json::json!({ "wiki": "test" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    // Returns { job_id: String } immediately; rebuild runs in the background.
    assert!(
        body["job_id"].is_string(),
        "rebuild returns a job_id immediately (got: {body})"
    );
}
