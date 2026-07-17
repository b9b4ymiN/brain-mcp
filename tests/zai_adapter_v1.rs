//! Phase D Task D1 — Z.ai adapter (ZaiHttpAdapter).
//!
//! Tests the concrete `ZaiHttpAdapter` implementing `AiProvider` against a
//! scripted `HttpTransport` (no real network) — plus the plain `MockProvider`
//! kept from the RED stage for basic `AiProvider` propagation checks. The
//! live-network smoke test lives in a separate file (gated on `ZAI_API_KEY`,
//! run only with explicit approval).

mod common;

use common::FixtureTransport;
use llm_wiki::provider::{
    AiProvider, ComplianceRecord, DeadLetterEntry, HttpTransport, OutboundPolicy, ProviderConfig,
    ProviderError, ProviderRequest, ProviderResult, RetryPolicy, TransportResponse, ZaiHttpAdapter,
    repair_json,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

// =============================================================================
// Mock provider for offline testing (RED-stage propagation checks)
// =============================================================================

struct MockProvider {
    response: Result<String, ProviderError>,
    name: &'static str,
}

impl AiProvider for MockProvider {
    fn complete(&self, _request: &ProviderRequest) -> ProviderResult<String> {
        self.response.clone()
    }
    fn adapter_name(&self) -> &str {
        self.name
    }
}

fn req(prompt: &str, local_only: bool) -> ProviderRequest {
    ProviderRequest {
        prompt: prompt.to_owned(),
        max_tokens: 100,
        temperature: 0.0,
        local_only,
    }
}

#[test]
fn mock_provider_returns_canned_response() {
    let provider = MockProvider {
        response: Ok("extracted: GULF target_price 58".to_owned()),
        name: "mock",
    };
    let result = provider.complete(&req("summarize", false)).unwrap();
    assert!(result.contains("GULF"));
}

#[test]
fn mock_provider_propagates_timeout_error() {
    let provider = MockProvider {
        response: Err(ProviderError::Timeout),
        name: "mock",
    };
    let err = provider.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert!(err.is_retryable());
}

#[test]
fn disabled_config_blocks_all_calls() {
    let config = ProviderConfig {
        base_url: "https://api.z.ai/test".to_owned(),
        api_key_ref: "op://vault/test".to_owned(),
        routine_model: "test-model".to_owned(),
        reasoning_model: "test-model".to_owned(),
        kill_switch: true,
    };
    assert!(config.is_disabled());
    let provider = MockProvider {
        response: Err(ProviderError::Disabled),
        name: "mock-disabled",
    };
    let err = provider.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Disabled);
    assert!(!err.is_retryable());
}

#[test]
fn local_only_request_blocked_before_adapter() {
    let policy = OutboundPolicy::new();
    let decision = policy.check(&req("secret stuff", true));
    assert!(decision.denied, "local_only request must be denied");
}

#[test]
fn secret_request_blocked_before_adapter() {
    let policy = OutboundPolicy::new();
    let decision = policy.check(&req("my key is Bearer sk-leaked1234567890abcdef", false));
    assert!(decision.denied, "secret-bearing request must be denied");
}

// =============================================================================
// ScriptedTransport — canned HttpTransport for ZaiHttpAdapter tests
// =============================================================================

/// A scripted `HttpTransport`: pops one outcome per call from a queue, and
/// counts every call it received so tests can assert what actually reached
/// the "network" boundary — e.g. that a denied request never got this far.
struct ScriptedTransport {
    outcomes: Mutex<VecDeque<Result<TransportResponse, ProviderError>>>,
    call_count: Mutex<usize>,
}

impl ScriptedTransport {
    fn new(outcomes: Vec<Result<TransportResponse, ProviderError>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            call_count: Mutex::new(0),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl HttpTransport for ScriptedTransport {
    fn send(
        &self,
        _url: &str,
        _api_key: &str,
        _body: &str,
    ) -> Result<TransportResponse, ProviderError> {
        *self.call_count.lock().unwrap() += 1;
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(ProviderError::Outage))
    }
}

/// Lets a test keep an `Arc` handle to a `ScriptedTransport` after boxing a
/// clone of it into the adapter, so call counts can be inspected afterward.
struct SharedTransport(Arc<ScriptedTransport>);

impl HttpTransport for SharedTransport {
    fn send(
        &self,
        url: &str,
        api_key: &str,
        body: &str,
    ) -> Result<TransportResponse, ProviderError> {
        self.0.send(url, api_key, body)
    }
}

fn ok_response(content: &str) -> TransportResponse {
    TransportResponse {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string(),
    }
}

fn status_response(status: u16, body: &str) -> TransportResponse {
    TransportResponse {
        status,
        body: body.to_owned(),
    }
}

fn test_config() -> ProviderConfig {
    ProviderConfig {
        base_url: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned(),
        api_key_ref: "env:ZAI_TEST_KEY_UNSET".to_owned(),
        routine_model: "glm-coding".to_owned(),
        reasoning_model: "glm-reasoning".to_owned(),
        kill_switch: false,
    }
}

fn test_compliance() -> ComplianceRecord {
    ComplianceRecord {
        user_decision: "approved for test".to_owned(),
        endpoint: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned(),
        workload: "extraction".to_owned(),
        known_terms_risk: "test risk".to_owned(),
        retention_terms: "unconfirmed".to_owned(),
        training_terms: "unconfirmed".to_owned(),
        processing_region: "unconfirmed".to_owned(),
        acknowledged_at: "2026-07-16T00:00:00Z".to_owned(),
    }
}

/// Build an adapter with a fresh scripted transport + fast retry policy +
/// a throwaway compliance-log tempdir (returned so it isn't dropped early).
fn build_adapter(transport: ScriptedTransport) -> (ZaiHttpAdapter, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("compliance.jsonl");
    let adapter = ZaiHttpAdapter::with_transport(
        test_config(),
        test_compliance(),
        log_path,
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    (adapter, dir)
}

fn set_test_env(name: &str, value: &str) {
    unsafe {
        std::env::set_var(name, value);
    }
}

// =============================================================================
// ZaiHttpAdapter implements AiProvider (compile-time proof)
// =============================================================================

#[test]
fn zai_adapter_implements_ai_provider() {
    fn _accepts_provider<P: AiProvider>(_p: &P) {}
    let (adapter, _dir) = build_adapter(ScriptedTransport::new(vec![Ok(ok_response("hello"))]));
    _accepts_provider(&adapter);
}

// =============================================================================
// Success path
// =============================================================================

#[test]
fn successful_completion_extracts_message_content() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_OK", "test-key-value");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_OK".to_owned();
    let dir = tempdir().unwrap();
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("compliance.jsonl"),
        Box::new(ScriptedTransport::new(vec![Ok(ok_response(
            "extracted: GULF 58",
        ))])),
    )
    .unwrap();
    let result = adapter.complete(&req("summarize", false)).unwrap();
    assert!(result.contains("GULF"));
}

#[test]
fn missing_api_key_env_maps_to_retryable_outage() {
    let (adapter, _dir) = build_adapter(ScriptedTransport::new(vec![])); // never reached
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Outage);
    assert!(err.is_retryable());
}

// =============================================================================
// 8 error modes
// =============================================================================

#[test]
fn kill_switch_returns_disabled_without_touching_transport() {
    let mut config = test_config();
    config.kill_switch = true;
    let dir = tempdir().unwrap();
    let shared = Arc::new(ScriptedTransport::new(vec![]));
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(SharedTransport(shared.clone())),
    )
    .unwrap();
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Disabled);
    assert!(!err.is_retryable());
    assert_eq!(
        shared.call_count(),
        0,
        "kill switch must block before any transport call"
    );
}

#[test]
fn rate_limited_429_is_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_429", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_429".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Ok(status_response(
            429,
            r#"{"error":{"message":"rate limit exceeded"}}"#,
        )),
        Ok(status_response(
            429,
            r#"{"error":{"message":"rate limit exceeded"}}"#,
        )),
        Ok(status_response(
            429,
            r#"{"error":{"message":"rate limit exceeded"}}"#,
        )),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::RateLimited);
    assert!(err.is_retryable());
}

#[test]
fn quota_exhausted_429_is_not_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_QUOTA", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_QUOTA".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![Ok(status_response(
        429,
        r#"{"error":{"type":"insufficient_quota","message":"you exceeded your current quota"}}"#,
    ))]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::QuotaExhausted);
    assert!(!err.is_retryable());
}

#[test]
fn server_error_5xx_is_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_5XX", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_5XX".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Ok(status_response(503, "service unavailable")),
        Ok(status_response(503, "service unavailable")),
        Ok(status_response(503, "service unavailable")),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::ServerError(503));
    assert!(err.is_retryable());
}

#[test]
fn invalid_json_envelope_is_not_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_BADJSON", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_BADJSON".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![Ok(status_response(200, "not json at all"))]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert!(matches!(err, ProviderError::InvalidJson(_)));
    assert!(!err.is_retryable());
}

#[test]
fn transport_timeout_is_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_TIMEOUT", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_TIMEOUT".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert!(err.is_retryable());
}

#[test]
fn transport_outage_is_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_OUTAGE", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_OUTAGE".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Err(ProviderError::Outage),
        Err(ProviderError::Outage),
        Err(ProviderError::Outage),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Outage);
    assert!(err.is_retryable());
}

#[test]
fn transport_partial_stream_is_retryable() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_PARTIAL", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_PARTIAL".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Err(ProviderError::PartialStream),
        Err(ProviderError::PartialStream),
        Err(ProviderError::PartialStream),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::PartialStream);
    assert!(err.is_retryable());
}

// =============================================================================
// Bounded retry/backoff
// =============================================================================

#[test]
fn retry_succeeds_after_transient_failure_within_bound() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_RETRY_OK", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_RETRY_OK".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Err(ProviderError::Timeout),
        Ok(ok_response("recovered")),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    let result = adapter.complete(&req("x", false)).unwrap();
    assert_eq!(result, "recovered");
}

#[test]
fn retry_is_bounded_by_max_attempts() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_BOUND", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_BOUND".to_owned();
    let dir = tempdir().unwrap();
    let policy = RetryPolicy::fast_for_tests(); // max_attempts = 3
    let shared = Arc::new(ScriptedTransport::new(vec![
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout), // must never be consumed
    ]));
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(SharedTransport(shared.clone())),
    )
    .unwrap()
    .with_retry_policy(policy.clone());
    let err = adapter.complete(&req("x", false)).unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert_eq!(
        shared.call_count(),
        policy.max_attempts as usize,
        "retry loop must stop exactly at max_attempts"
    );
}

// =============================================================================
// OutboundPolicy gate: normal + retry + dead-letter — 100% redaction
// =============================================================================

#[test]
fn denied_request_makes_zero_transport_calls() {
    let dir = tempdir().unwrap();
    let shared = Arc::new(ScriptedTransport::new(vec![Ok(ok_response(
        "should never be used",
    ))]));
    let adapter = ZaiHttpAdapter::with_transport(
        test_config(),
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(SharedTransport(shared.clone())),
    )
    .unwrap();
    let _ = adapter.complete(&req("secret plan", true));
    assert_eq!(
        shared.call_count(),
        0,
        "denied request must never reach the transport"
    );
}

#[test]
fn local_only_request_dead_letter_never_contains_raw_prompt() {
    let (adapter, _dir) = build_adapter(ScriptedTransport::new(vec![]));
    let err = adapter
        .complete(&req("internal secret plan", true))
        .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidJson(_)));
    let dead_letters = adapter.dead_letters();
    assert_eq!(dead_letters.len(), 1);
    assert!(
        !dead_letters[0]
            .redacted_prompt
            .contains("internal secret plan"),
        "dead-letter must not contain the raw prompt: {}",
        dead_letters[0].redacted_prompt
    );
}

#[test]
fn secret_bearing_request_dead_letter_never_contains_the_key() {
    let (adapter, _dir) = build_adapter(ScriptedTransport::new(vec![]));
    let secret_prompt = "my key is Bearer sk-verysecretvalue1234567890";
    let err = adapter.complete(&req(secret_prompt, false)).unwrap_err();
    assert!(matches!(err, ProviderError::InvalidJson(_)));
    let dead_letters = adapter.dead_letters();
    assert_eq!(dead_letters.len(), 1);
    let debug_repr = format!("{:?}", dead_letters[0]);
    assert!(!debug_repr.contains("sk-verysecretvalue1234567890"));
}

#[test]
fn exhausted_retry_dead_letter_always_uses_redacted_prompt() {
    set_test_env("ZAI_ADAPTER_TEST_KEY_DL", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_DL".to_owned();
    let dir = tempdir().unwrap();
    let transport = ScriptedTransport::new(vec![
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
        Err(ProviderError::Timeout),
    ]);
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(transport),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy::fast_for_tests());
    // Deliberately no recognized secret pattern here (no "Bearer"/"sk-"/etc)
    // — that would get denied by OutboundPolicy at attempt 1, never reaching
    // the retry-exhaustion path this test exercises. The point of this test
    // is the STRUCTURAL invariant: the dead-letter path always calls through
    // the same redaction helper as the normal/retry path, never the raw
    // `request.prompt` field directly.
    let sensitive = "customer note: quarterly figures are not public yet";
    let _ = adapter.complete(&req(sensitive, false));
    let dead_letters = adapter.dead_letters();
    assert_eq!(dead_letters.len(), 1);
    assert_eq!(dead_letters[0].attempts, 3);
    assert_eq!(
        dead_letters[0].redacted_prompt,
        OutboundPolicy::check_text_redact(sensitive),
        "dead-letter must go through the same redaction helper as normal/retry paths"
    );
}

// =============================================================================
// Grep-gate (Task D1.10) — no key ever committed to a golden fixture
// =============================================================================

#[test]
fn golden_fixtures_contain_no_recognized_secret_pattern() {
    let dir = common::golden_fixture_dir();
    if !dir.exists() {
        eprintln!(
            "no golden fixtures captured yet at {} — nothing to gate",
            dir.display()
        );
        return;
    }
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap();
        let redacted = OutboundPolicy::check_text_redact(&content);
        assert_eq!(
            redacted,
            content,
            "golden fixture {} contains a recognized secret pattern (bearer/sk-/api_key) — must never be committed",
            path.display()
        );
        if let Ok(real_key) = std::env::var("ZAI_API_KEY")
            && !real_key.is_empty()
        {
            assert!(
                !content.contains(&real_key),
                "golden fixture {} contains the literal ZAI_API_KEY value",
                path.display()
            );
        }
    }
}

// =============================================================================
// ComplianceRecord persistence
// =============================================================================

#[test]
fn compliance_record_is_persisted_on_adapter_construction() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("compliance.jsonl");
    let record = test_compliance();
    let _adapter = ZaiHttpAdapter::with_transport(
        test_config(),
        record.clone(),
        log_path.clone(),
        Box::new(ScriptedTransport::new(vec![])),
    )
    .unwrap();
    let loaded = ComplianceRecord::load_all_from(&log_path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], record);
}

#[test]
fn compliance_record_append_accumulates_across_constructions() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("compliance.jsonl");
    for i in 0..3 {
        let mut record = test_compliance();
        record.acknowledged_at = format!("2026-07-16T0{i}:00:00Z");
        let _adapter = ZaiHttpAdapter::with_transport(
            test_config(),
            record,
            log_path.clone(),
            Box::new(ScriptedTransport::new(vec![])),
        )
        .unwrap();
    }
    let loaded = ComplianceRecord::load_all_from(&log_path).unwrap();
    assert_eq!(loaded.len(), 3);
}

#[test]
fn compliance_record_load_from_missing_file_is_empty() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.jsonl");
    let loaded = ComplianceRecord::load_all_from(&missing).unwrap();
    assert!(loaded.is_empty());
}

// =============================================================================
// Bounded JSON repair (used by the live-smoke schema check)
// =============================================================================

#[test]
fn repair_json_parses_clean_json_directly() {
    let value = repair_json(r#"{"a": 1}"#).unwrap();
    assert_eq!(value["a"], 1);
}

#[test]
fn repair_json_strips_markdown_code_fences() {
    let value = repair_json("```json\n{\"a\": 1}\n```").unwrap();
    assert_eq!(value["a"], 1);
}

#[test]
fn repair_json_fails_on_genuinely_broken_json() {
    assert!(repair_json("not json { at all").is_err());
}

#[test]
fn dead_letter_entry_serializes_redacted_field_only() {
    let entry = DeadLetterEntry {
        adapter_name: "zai_openai_compatible".to_owned(),
        redacted_prompt: "[REDACTED]".to_owned(),
        error: ProviderError::Timeout,
        attempts: 1,
        occurred_at: "2026-07-16T00:00:00Z".to_owned(),
    };
    let value = serde_json::to_value(&entry).unwrap();
    assert!(value.get("prompt").is_none());
    assert!(value.get("redacted_prompt").is_some());
}

// =============================================================================
// Golden fixture replay (Task D1.8) — real captured bytes, no re-fire
// =============================================================================
//
// The fixture is captured by the live-smoke test (D1.9), which only runs with
// explicit user approval + `ZAI_API_KEY` set. Until that has run once, no
// fixture file exists yet — this replay test degrades to a documented no-op
// rather than fabricating response bytes (matches the Phase D decision:
// "response จริงถูกเก็บเป็น golden fixtures... ไม่มี key → live tests skip").

#[test]
fn golden_fixture_replay_is_schema_valid_after_bounded_repair() {
    let Some(fixture) = FixtureTransport::load("chat_completion_v1") else {
        eprintln!(
            "no golden fixture yet at {} — run the live smoke test (with approval + ZAI_API_KEY) to capture one",
            common::golden_fixture_path("chat_completion_v1").display()
        );
        return;
    };
    set_test_env("ZAI_ADAPTER_TEST_KEY_REPLAY", "k");
    let mut config = test_config();
    config.api_key_ref = "env:ZAI_ADAPTER_TEST_KEY_REPLAY".to_owned();
    let dir = tempdir().unwrap();
    let adapter = ZaiHttpAdapter::with_transport(
        config,
        test_compliance(),
        dir.path().join("c.jsonl"),
        Box::new(fixture),
    )
    .unwrap();
    let content = adapter.complete(&req("replay smoke", false)).unwrap();
    let parsed = repair_json(&content)
        .expect("golden fixture content must be schema-valid JSON after bounded repair");
    assert!(parsed.is_object() || parsed.is_array());
}
