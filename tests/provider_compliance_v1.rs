//! Task 4.1 — Provider adapter + Z.ai compliance gate (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 4.1 DoD at the contract level. The provider is
//! NOT called for real in-env (no network); this task delivers the domain
//! interface, config, kill switch, outbound redaction policy, error coverage,
//! and compliance record that a future deployment wires to a real HTTP
//! adapter. §8.1 Provider boundary + §8.2 Privacy + §10 secret egress.

use llm_wiki::provider::{
    AiProvider, ComplianceRecord, OutboundPolicy, ProviderConfig, ProviderError, ProviderRequest,
    ProviderResult,
};
use serde_json::json;

// =============================================================================
// DoD: domain core has no Z.ai-specific field/model id
// =============================================================================

/// `ProviderRequest` is provider-agnostic: it carries a prompt + params only,
/// no `zai_endpoint` / `glm_model` / provider-specific field. A future General
/// API / OpenAI-compatible / local model adapter consumes the same struct.
#[test]
fn provider_request_is_provider_agnostic() {
    let req = ProviderRequest {
        prompt: "summarize".to_owned(),
        max_tokens: 512,
        temperature: 0.0,
        local_only: false,
    };
    // Serialize and assert no Z.ai-specific key leaks into the wire shape.
    let serialized = serde_json::to_value(&req).unwrap();
    assert!(serialized.get("zai_endpoint").is_none());
    assert!(serialized.get("glm_model").is_none());
    assert!(serialized.get("provider").is_none());
    assert_eq!(serialized["prompt"], "summarize");
}

/// `AiProvider` is a trait the domain core depends on — there is no concrete
/// Z.ai struct in the domain layer. Compile-time proof.
#[test]
fn ai_provider_is_a_trait() {
    fn _accepts_provider<P: AiProvider>(_p: &P) {}
    // If this compiles, AiProvider is a trait the domain depends on.
}

// =============================================================================
// DoD: provider config — endpoint in config, not durable schema; kill switch
// =============================================================================

/// `ProviderConfig` carries the endpoint, key reference, and model names as
/// CONFIG (not durable schema). The kill switch disables all outbound calls
/// when set, regardless of other config.
#[test]
fn provider_config_has_kill_switch() {
    let config = ProviderConfig {
        base_url: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned(),
        api_key_ref: "op://vault/zai/key".to_owned(), // secret manager ref, not the key
        routine_model: "glm-coding".to_owned(),
        reasoning_model: "glm-reasoning".to_owned(),
        kill_switch: true,
    };
    assert!(
        config.is_disabled(),
        "kill_switch=true must disable the provider"
    );
}

/// When the kill switch is off, the provider is enabled and the config is
/// usable. The endpoint + model are configurable (swappable without migrating
/// canonical data — §8.1 "สลับ base URL/model/provider ได้โดยไม่ migrate data").
#[test]
fn provider_config_enabled_when_kill_switch_off() {
    let config = ProviderConfig {
        base_url: "https://api.other.com/v1/chat".to_owned(),
        api_key_ref: "op://vault/other/key".to_owned(),
        routine_model: "gpt-equivalent".to_owned(),
        reasoning_model: "gpt-equivalent".to_owned(),
        kill_switch: false,
    };
    assert!(!config.is_disabled());
    // The config never embeds the raw key — only a reference.
    assert!(
        !config.api_key_ref.starts_with("sk-"),
        "config must hold a secret-manager REFERENCE, not a raw key"
    );
}

// =============================================================================
// DoD: outbound policy — deny-by-default, local_only/secret never egress
// =============================================================================

/// `OutboundPolicy` is deny-by-default: a request must pass the policy check
/// before it is allowed to reach the provider. §8.2 "outbound policy ต้อง
/// deny-by-default".
#[test]
fn outbound_policy_denies_by_default_when_local_only() {
    let policy = OutboundPolicy::new();
    let req = ProviderRequest {
        prompt: "internal note".to_owned(),
        max_tokens: 10,
        temperature: 0.0,
        local_only: true,
    };
    let decision = policy.check(&req);
    assert!(
        decision.denied,
        "local_only request must be denied by the outbound policy"
    );
}

/// A request carrying a detected secret is denied even if not local_only.
#[test]
fn outbound_policy_denies_detected_secrets() {
    let policy = OutboundPolicy::new();
    let req = ProviderRequest {
        prompt: "my key is Bearer sk-leaked-1234567890 please help".to_owned(),
        max_tokens: 10,
        temperature: 0.0,
        local_only: false,
    };
    let decision = policy.check(&req);
    assert!(
        decision.denied,
        "request with a detected secret must be denied"
    );
    assert!(
        decision.reason.contains("secret"),
        "denial reason should mention the secret: got {}",
        decision.reason
    );
}

/// A SINGLE realistic `sk-...` API key (not four repeats) must be denied.
/// This is the regression test for the Task 4.1 Validator finding: the
/// previous `sk-` length-arithmetic counted substring occurrences and missed
/// a single key. Realistic OpenAI/Z.ai shapes: `sk-proj-...`, `sk-<48 hex>`.
#[test]
fn outbound_policy_denies_a_single_realistic_sk_key() {
    let policy = OutboundPolicy::new();
    for key in [
        "sk-proj-1234567890abcdefghijklmnop",
        "sk-1234567890abcdef1234567890abcdef",
        "key=sk-deadbeefcafef00dbaadf00dcafe1234f00d",
    ] {
        let req = ProviderRequest {
            prompt: format!("please use {key}"),
            max_tokens: 10,
            temperature: 0.0,
            local_only: false,
        };
        let decision = policy.check(&req);
        assert!(
            decision.denied,
            "a single realistic sk- key must be denied: prompt was {:?}",
            req.prompt
        );
    }
}

/// The denial reason must NEVER contain the secret substring itself — only a
/// label. Otherwise the audit log/redacted payload would re-leak the secret
/// it caught. (Validator secondary observation: this invariant was correct
/// but untested.)
#[test]
fn denial_reason_never_contains_the_secret() {
    let policy = OutboundPolicy::new();
    let secret = "sk-1234567890abcdef1234567890abcdef";
    let req = ProviderRequest {
        prompt: format!("here is my key {secret}"),
        max_tokens: 10,
        temperature: 0.0,
        local_only: false,
    };
    let decision = policy.check(&req);
    assert!(decision.denied);
    assert!(
        !decision.reason.contains(secret),
        "denial reason must not echo the secret: got {}",
        decision.reason
    );
}

/// A clean, non-local-only request is allowed through.
#[test]
fn outbound_policy_allows_clean_non_local_request() {
    let policy = OutboundPolicy::new();
    let req = ProviderRequest {
        prompt: "summarize this public article".to_owned(),
        max_tokens: 100,
        temperature: 0.0,
        local_only: false,
    };
    let decision = policy.check(&req);
    assert!(
        !decision.denied,
        "clean non-local request should be allowed"
    );
}

// =============================================================================
// DoD: error coverage — timeout/quota/429/5xx/invalid JSON/partial stream/outage
// =============================================================================

/// `ProviderError` covers every failure mode §8.1 names. Each variant is
/// actionable (a caller can decide retry vs dead-letter vs abort).
#[test]
fn provider_error_covers_all_failure_modes() {
    let _ = ProviderError::Timeout;
    let _ = ProviderError::QuotaExhausted;
    let _ = ProviderError::RateLimited; // 429
    let _ = ProviderError::ServerError(503); // 5xx
    let _ = ProviderError::InvalidJson("unexpected token".to_owned());
    let _ = ProviderError::PartialStream;
    let _ = ProviderError::Outage;
    let _ = ProviderError::Disabled; // kill switch
}

/// `ProviderError::is_retryable` distinguishes transient (timeout, 429, 5xx,
/// outage) from permanent (invalid JSON, quota exhausted, disabled). A caller
/// uses this to decide backoff vs dead-letter.
#[test]
fn provider_error_retryable_classification() {
    assert!(ProviderError::Timeout.is_retryable());
    assert!(ProviderError::RateLimited.is_retryable());
    assert!(ProviderError::ServerError(503).is_retryable());
    assert!(ProviderError::Outage.is_retryable());
    assert!(!ProviderError::InvalidJson("x".to_owned()).is_retryable());
    assert!(!ProviderError::QuotaExhausted.is_retryable());
    assert!(!ProviderError::Disabled.is_retryable());
}

// =============================================================================
// DoD: compliance record — user decision, endpoint, terms risk, timestamp
// =============================================================================

/// `ComplianceRecord` captures the §8.2 compliance acknowledgement: who
/// decided, what endpoint/workload, known terms risk, retention/training/
/// region, and a timestamp. It is the auditable evidence that the user
/// accepted the Z.ai terms risk before the provider was enabled.
#[test]
fn compliance_record_carries_required_fields() {
    let record = ComplianceRecord {
        user_decision: "approved Z.ai Coding Plan endpoint".to_owned(),
        endpoint: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned(),
        workload: "extraction/synthesis/consolidation".to_owned(),
        known_terms_risk: "GLM Coding Plan limits supported coding tools; account risk if used from custom backend".to_owned(),
        retention_terms: "unconfirmed".to_owned(),
        training_terms: "unconfirmed".to_owned(),
        processing_region: "unconfirmed".to_owned(),
        acknowledged_at: "2026-07-16T00:00:00Z".to_owned(),
    };
    assert!(!record.user_decision.is_empty());
    assert!(record.endpoint.contains("z.ai"));
    assert!(!record.known_terms_risk.is_empty());
    assert!(!record.acknowledged_at.is_empty());
}

// keep json import alive for future expansion
#[test]
fn _json_compile_check() {
    let _ = json!({"ok": true});
}

/// ProviderResult is the return type carrying either a response or a
/// ProviderError. Compile-time proof the type alias exists.
#[test]
fn provider_result_type_exists() {
    let ok: ProviderResult<String> = Ok("response".to_owned());
    let err: ProviderResult<String> = Err(ProviderError::Timeout);
    assert!(ok.is_ok());
    assert!(err.is_err());
}
