//! Phase D Task D1.9 — LIVE smoke test against the real Z.ai endpoint.
//!
//! GATED: `#[ignore]` by default — never runs in a normal `cargo test` sweep,
//! not in CI, not by accident. Even run explicitly
//! (`cargo test --test zai_adapter_live_v1 -- --ignored`), it degrades to a
//! documented no-op unless `ZAI_API_KEY` is set (Phase D decision: "ไม่มี key
//! → live tests skip (ไม่ fabricate)"). Makes exactly ONE real HTTP call.
//! Run ONLY with the user's explicit in-session approval — see
//! `docs/plans/phase-D-zai-adapter.md` Task D1: "ยิงbatch แรกต้องขออนุมัติ".
//!
//! Model name defaults to `glm-4.6`; override with `ZAI_MODEL` if the
//! Coding Plan account uses a different model id. Endpoint defaults to the
//! Coding Plan chat-completions URL from the Phase D plan doc; override with
//! `ZAI_ENDPOINT` for a different region/plan endpoint.

mod common;

use llm_wiki::provider::{
    AiProvider, ComplianceRecord, HttpTransport, ProviderConfig, ProviderError, ProviderRequest,
    ReqwestTransport, TransportResponse, ZaiHttpAdapter, repair_json,
};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

/// Wraps the real [`ReqwestTransport`] and stashes the last raw response so
/// it can be saved as a golden fixture after the adapter's full retry/policy
/// path (not just the raw transport) has proven the round trip works.
struct CapturingTransport {
    inner: ReqwestTransport,
    last_response: Arc<Mutex<Option<TransportResponse>>>,
}

impl HttpTransport for CapturingTransport {
    fn send(
        &self,
        url: &str,
        api_key: &str,
        body: &str,
    ) -> Result<TransportResponse, ProviderError> {
        let result = self.inner.send(url, api_key, body);
        if let Ok(resp) = &result {
            *self.last_response.lock().unwrap() = Some(resp.clone());
        }
        result
    }
}

#[test]
#[ignore = "hits the real Z.ai network endpoint — run only with explicit user approval and ZAI_API_KEY set"]
fn live_chat_completion_succeeds_and_captures_golden_fixture() {
    if std::env::var("ZAI_API_KEY").is_err() {
        eprintln!("ZAI_API_KEY not set — skipping live smoke test (no fabricated data)");
        return;
    }

    let model = std::env::var("ZAI_MODEL").unwrap_or_else(|_| "glm-4.6".to_owned());
    let endpoint = std::env::var("ZAI_ENDPOINT")
        .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned());
    let config = ProviderConfig {
        base_url: endpoint,
        api_key_ref: "env:ZAI_API_KEY".to_owned(),
        routine_model: model.clone(),
        reasoning_model: model,
        kill_switch: false,
    };

    let compliance = ComplianceRecord {
        user_decision: "approved Z.ai Coding Plan endpoint for Task D1 live smoke test".to_owned(),
        endpoint: config.base_url.clone(),
        workload: "extraction/synthesis/consolidation — D1 live smoke".to_owned(),
        known_terms_risk:
            "GLM Coding Plan limits supported coding tools; account risk if used from a custom backend"
                .to_owned(),
        retention_terms: "unconfirmed".to_owned(),
        training_terms: "unconfirmed".to_owned(),
        processing_region: "unconfirmed".to_owned(),
        acknowledged_at: chrono::Utc::now().to_rfc3339(),
    };

    let last_response = Arc::new(Mutex::new(None));
    let transport = CapturingTransport {
        inner: ReqwestTransport::new(),
        last_response: last_response.clone(),
    };

    let dir = tempdir().unwrap();
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        compliance,
        dir.path().join("compliance.jsonl"),
        Box::new(transport),
    )
    .expect("compliance record persists to a fresh tempdir");

    let request = ProviderRequest {
        prompt: "Reply with a JSON object of the exact shape {\"ack\": \"d1-live-smoke\"} and nothing else."
            .to_owned(),
        // glm-4.6 is a reasoning model: it spends tokens on `reasoning_content`
        // before ever writing `content`. A live run at max_tokens=64 hit
        // finish_reason="length" with reasoning still in progress and content
        // empty — not an adapter defect (the round trip, envelope parse, and
        // empty-content-is-not-JSON detection all worked correctly), just too
        // small a budget for a reasoning-capable model. 512 leaves headroom.
        max_tokens: 512,
        temperature: 0.0,
        local_only: false,
    };

    let outcome = adapter.complete(&request);

    // Diagnostic: always show what the transport actually captured (redacted
    // defensively, even though a provider's response body isn't expected to
    // echo request secrets back) — this is what tells us WHY a live call
    // failed (rate limit vs quota vs auth vs schema) without guessing.
    if let Some(resp) = last_response.lock().unwrap().clone() {
        eprintln!(
            "raw transport response: status={} body={}",
            resp.status,
            llm_wiki::provider::OutboundPolicy::check_text_redact(&resp.body)
        );
    } else {
        eprintln!(
            "no response was captured — the failure happened before any HTTP response arrived"
        );
    }

    let content = outcome.expect("live chat completion must succeed");

    let parsed = repair_json(&content)
        .expect("live response content must be schema-valid JSON after bounded repair");
    assert!(parsed.is_object(), "expected a JSON object, got: {content}");

    let captured = last_response
        .lock()
        .unwrap()
        .clone()
        .expect("transport must have captured a response");
    common::save_golden_fixture("chat_completion_v1", &captured)
        .expect("golden fixture must be saved for CI replay");

    eprintln!(
        "captured golden fixture at {}",
        common::golden_fixture_path("chat_completion_v1").display()
    );
}
