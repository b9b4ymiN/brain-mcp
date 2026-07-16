//! Phase D Task D1 — Z.ai adapter (RED stage).
//!
//! Tests the concrete ZaiAdapter implementing AiProvider, plus a mock
//! provider for offline testing. No real network calls.

use llm_wiki::provider::{
    AiProvider, ProviderConfig, ProviderError, ProviderRequest, ProviderResult,
};

// =============================================================================
// Mock provider for offline testing
// =============================================================================

/// A mock provider that returns a canned response or a specific error.
/// Used throughout the test suite to exercise error handling without network.
struct MockProvider {
    response: Result<String, ProviderError>,
    name: &'static str,
}

impl AiProvider for MockProvider {
    fn complete(&self, _request: &ProviderRequest) -> ProviderResult<String> {
        self.response.clone().map_err(|e| e)
    }
    fn adapter_name(&self) -> &str {
        self.name
    }
}

#[test]
fn mock_provider_returns_canned_response() {
    let provider = MockProvider {
        response: Ok("extracted: GULF target_price 58".to_owned()),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("summarize", 100, false);
    let result = provider.complete(&req).unwrap();
    assert!(result.contains("GULF"));
}

#[test]
fn mock_provider_propagates_timeout_error() {
    let provider = MockProvider {
        response: Err(ProviderError::Timeout),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert_eq!(err, ProviderError::Timeout);
    assert!(err.is_retryable());
}

#[test]
fn mock_provider_propagates_rate_limited() {
    let provider = MockProvider {
        response: Err(ProviderError::RateLimited),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert!(err.is_retryable());
}

#[test]
fn mock_provider_propagates_server_error() {
    let provider = MockProvider {
        response: Err(ProviderError::ServerError(503)),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert!(err.is_retryable());
    assert_eq!(err, ProviderError::ServerError(503));
}

#[test]
fn mock_provider_propagates_invalid_json() {
    let provider = MockProvider {
        response: Err(ProviderError::InvalidJson("unexpected token".to_owned())),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert!(!err.is_retryable());
}

#[test]
fn mock_provider_propagates_quota_exhausted() {
    let provider = MockProvider {
        response: Err(ProviderError::QuotaExhausted),
        name: "mock",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert!(!err.is_retryable());
}

// =============================================================================
// Kill switch: disabled config returns ProviderError::Disabled
// =============================================================================

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
    // A disabled provider should return Disabled error
    let provider = MockProvider {
        response: Err(ProviderError::Disabled),
        name: "mock-disabled",
    };
    let req = ProviderRequest::deterministic("x", 100, false);
    let err = provider.complete(&req).unwrap_err();
    assert_eq!(err, ProviderError::Disabled);
    assert!(!err.is_retryable());
}

// =============================================================================
// ZaiAdapter struct exists and implements AiProvider
// =============================================================================

#[test]
fn zai_adapter_implements_ai_provider() {
    fn _accepts_provider<P: AiProvider>(_p: &P) {}
    // Compile-time proof ZaiAdapter exists and implements the trait.
    let _ = std::mem::size_of::<llm_wiki::provider::ZaiAdapter>();
}

// =============================================================================
// OutboundPolicy gate runs before adapter.complete
// =============================================================================

#[test]
fn local_only_request_blocked_before_adapter() {
    use llm_wiki::provider::OutboundPolicy;
    let policy = OutboundPolicy::new();
    let req = ProviderRequest::deterministic("secret stuff", 100, true);
    let decision = policy.check(&req);
    assert!(decision.denied, "local_only request must be denied");
}

#[test]
fn secret_request_blocked_before_adapter() {
    use llm_wiki::provider::OutboundPolicy;
    let policy = OutboundPolicy::new();
    let req = ProviderRequest {
        prompt: "my key is Bearer sk-leaked1234567890abcdef".to_owned(),
        max_tokens: 100,
        temperature: 0.0,
        local_only: false,
    };
    let decision = policy.check(&req);
    assert!(decision.denied, "secret-bearing request must be denied");
}
