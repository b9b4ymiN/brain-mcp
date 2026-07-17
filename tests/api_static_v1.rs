//! Phase E Task E0.3 — Console static-file serving. Spawns
//! `llm_wiki::api::static_router` over loopback and asserts the security
//! contract the Task E1 Svelte build output inherits: a strict CSP on every
//! response, the index served, and no file escaping the configured root via
//! `..` traversal.
//!
//! The served directory is a self-contained `tempdir` fixture, NOT the real
//! `web/console/dist` build output — that path is a gitignored build
//! artifact (Task E1.0) that may not exist on a fresh checkout, and its
//! content changes as the Console UI evolves. Depending on it here would
//! make this suite flaky against both facts. The fixture also puts a sibling
//! "secret" file next to (not inside) the served root, so the traversal test
//! proves an escape is blocked against a file that verifiably exists,
//! instead of incidentally relying on the repo's own directory depth.

use std::net::SocketAddr;
use std::path::PathBuf;

use llm_wiki::api::{CONSOLE_CSP, static_router};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A served `dist/` dir plus a sibling `secret.txt` OUTSIDE it. `_root` is
/// held only for its `Drop` side effect (deletes the tree at end of scope) —
/// never read directly, hence the leading underscore.
struct StaticFixture {
    _root: TempDir,
    served_dir: PathBuf,
}

const SECRET_MARKER: &str = "TOP-SECRET-OUTSIDE-SERVED-ROOT";

fn build_fixture() -> StaticFixture {
    let root = tempfile::tempdir().expect("tempdir");
    let served_dir = root.path().join("dist");
    std::fs::create_dir(&served_dir).expect("create served dir");
    std::fs::write(
        served_dir.join("index.html"),
        "<!doctype html><title>Console fixture</title>",
    )
    .expect("write fixture index.html");
    std::fs::write(root.path().join("secret.txt"), SECRET_MARKER).expect("write sibling secret");
    StaticFixture {
        _root: root,
        served_dir,
    }
}

async fn spawn_static(fixture: &StaticFixture) -> SocketAddr {
    let app = static_router(fixture.served_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Sends a raw HTTP/1.1 request with a literal request-target. Bypasses the
/// reqwest/`url` client-side normalization that would collapse `..` before it
/// hits the wire — an attacker controls the raw request bytes, so the test
/// must too. Returns `(status_line, full_response_text)`.
async fn raw_get(addr: SocketAddr, raw_path: &str) -> (String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {raw_path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status_line = text.lines().next().unwrap_or_default().to_owned();
    (status_line, text)
}

#[tokio::test]
async fn serves_index_with_csp_header() {
    let fixture = build_fixture();
    let addr = spawn_static(&fixture).await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let csp = resp
        .headers()
        .get("content-security-policy")
        .expect("CSP header present")
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(csp, CONSOLE_CSP);
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "csp: {csp}");
    assert!(csp.contains("object-src 'none'"), "csp: {csp}");

    let body = resp.text().await.unwrap();
    assert!(body.contains("Console fixture"), "index body: {body:?}");
}

#[tokio::test]
async fn serves_index_html_explicitly() {
    let fixture = build_fixture();
    let addr = spawn_static(&fixture).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{addr}/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().get("content-security-policy").is_some(),
        "CSP must be present on explicit index.html too"
    );
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let fixture = build_fixture();
    let addr = spawn_static(&fixture).await;

    // Both a bare `..` escape and a percent-encoded one, sent raw on the
    // wire, reaching for `secret.txt` — a sibling of `served_dir` that
    // verifiably exists on disk. `ServeDir` must never resolve either to a
    // file outside the served root.
    for path in ["/../secret.txt", "/%2e%2e/secret.txt"] {
        let (status_line, text) = raw_get(addr, path).await;
        assert!(
            !status_line.contains("200"),
            "traversal {path} must not return 200: {status_line}"
        );
        assert!(
            !text.contains(SECRET_MARKER),
            "traversal {path} leaked the sibling secret file"
        );
    }
}

#[tokio::test]
async fn csp_present_on_not_found() {
    let fixture = build_fixture();
    let addr = spawn_static(&fixture).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{addr}/does-not-exist.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    assert!(
        resp.headers().get("content-security-policy").is_some(),
        "CSP must be applied to 404 responses too"
    );
}
