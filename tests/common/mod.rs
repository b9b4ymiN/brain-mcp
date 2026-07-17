//! Shared test-only helpers for the Z.ai adapter test suites (Task D1).
//!
//! Golden fixtures hold REAL response bytes captured by the live-smoke test
//! (`zai_adapter_live_v1.rs`, gated + approval-only) and replayed by the
//! offline suite (`zai_adapter_v1.rs`) — never fabricated data.
//!
//! `mod common;` is compiled separately into each integration-test binary
//! that includes it, and no single binary uses every helper here (the
//! replay suite only loads fixtures; the live suite only saves them) —
//! `dead_code` per-binary is expected, not a real bug.
#![allow(dead_code)]

use llm_wiki::provider::{HttpTransport, ProviderError, TransportResponse};
use std::path::PathBuf;

/// Directory holding all Z.ai golden fixtures.
pub fn golden_fixture_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/zai")
}

/// Path to a named golden fixture under `tests/fixtures/golden/zai/`.
pub fn golden_fixture_path(name: &str) -> PathBuf {
    golden_fixture_dir().join(format!("{name}.json"))
}

/// Persist a real transport response as a golden fixture (pretty JSON: only
/// `status` + `body`, matching [`TransportResponse`]'s public shape).
pub fn save_golden_fixture(name: &str, resp: &TransportResponse) -> std::io::Result<()> {
    let path = golden_fixture_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let value = serde_json::json!({"status": resp.status, "body": resp.body});
    std::fs::write(path, serde_json::to_string_pretty(&value)?)
}

/// Replays a previously captured real response — no network touched. Returns
/// `None` if the named fixture has not been captured yet.
pub struct FixtureTransport {
    response: TransportResponse,
}

impl FixtureTransport {
    pub fn load(name: &str) -> Option<Self> {
        let content = std::fs::read_to_string(golden_fixture_path(name)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&content).ok()?;
        Some(Self {
            response: TransportResponse {
                status: value.get("status")?.as_u64()? as u16,
                body: value.get("body")?.as_str()?.to_owned(),
            },
        })
    }
}

impl HttpTransport for FixtureTransport {
    fn send(
        &self,
        _url: &str,
        _api_key: &str,
        _body: &str,
    ) -> Result<TransportResponse, ProviderError> {
        Ok(self.response.clone())
    }
}
