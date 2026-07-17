//! Phase E Task E0.3 — Console static-file serving. Spawns
//! `llm_wiki::api::static_router` over loopback and asserts the security
//! contract the (future) Task E1 Svelte build output will inherit: a strict
//! CSP on every response, the index served, and no file escaping the
//! configured root via `..` traversal. The served directory is the
//! placeholder `web/console/dist` (a trivial `index.html`); this task is the
//! transport plumbing, not the Console UI.

use std::net::SocketAddr;
use std::path::PathBuf;

use llm_wiki::api::{CONSOLE_CSP, static_router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join("console")
        .join("dist")
}

async fn spawn_static() -> SocketAddr {
    let app = static_router(dist_dir());
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
    let addr = spawn_static().await;
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
    assert!(body.contains("Console placeholder"), "index body: {body:?}");
}

#[tokio::test]
async fn serves_index_html_explicitly() {
    let addr = spawn_static().await;
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
    let addr = spawn_static().await;

    // Both a bare `..` escape and a percent-encoded one, sent raw on the wire.
    // `ServeDir` must never resolve either to a file outside `web/console/dist`.
    for path in [
        "/../../../../Cargo.toml",
        "/%2e%2e/%2e%2e/%2e%2e/%2e%2e/Cargo.toml",
    ] {
        let (status_line, text) = raw_get(addr, path).await;
        assert!(
            !status_line.contains("200"),
            "traversal {path} must not return 200: {status_line}"
        );
        assert!(
            !text.contains("[package]"),
            "traversal {path} leaked Cargo.toml contents"
        );
    }
}

#[tokio::test]
async fn csp_present_on_not_found() {
    let addr = spawn_static().await;
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
