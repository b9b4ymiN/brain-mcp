//! AI provider boundary (Task 4.1).
//!
//! Domain-level interface for an AI extraction/synthesis/consolidation
//! provider (GOAL-vNext §8.1, §13 Task 4.1). The domain core depends on the
//! [`AiProvider`] trait only — there is NO Z.ai-specific struct or field here.
//! A concrete adapter (Z.ai OpenAI-compatible, General API, or a local model)
//! lives in the deployment layer and implements the trait.
//!
//! Design rules enforced by this module:
//! - [`ProviderRequest`] is provider-agnostic (prompt + params, no endpoint/
//!   model/provider field in the wire shape).
//! - [`ProviderConfig`] holds the endpoint + key reference + model names as
//!   CONFIG (not durable schema) and carries a kill switch.
//! - [`OutboundPolicy`] is deny-by-default: `local_only` requests and requests
//!   carrying detected secrets are denied before they reach the provider.
//! - [`ProviderError`] covers every §8.1 failure mode with an actionable
//!   `is_retryable` classification.
//! - [`ComplianceRecord`] is the auditable §8.2 acknowledgement that the user
//!   accepted the provider's terms risk before the provider was enabled.

use serde::{Deserialize, Serialize};

// ── Provider-agnostic request/response ───────────────────────────────────────

/// A provider-agnostic request: prompt + sampling params + a `local_only`
/// flag the outbound policy consults. Carries NO endpoint, model id, or
/// provider name — those live in [`ProviderConfig`], not in the request wire
/// shape, so the domain never bakes in a provider-specific field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub prompt: String,
    pub max_tokens: u32,
    /// Sampling temperature (0.0 = deterministic). The deployment adapter maps
    /// this to the provider's sampling field; it is NOT a provider id.
    pub temperature: f64,
    /// If true, the outbound policy denies this request regardless of content
    /// (§8.2 `local_only`). The caller sets it from the source's privacy label.
    pub local_only: bool,
}

/// The result of a provider call: either a response string or a
/// [`ProviderError`].
pub type ProviderResult<T> = Result<T, ProviderError>;

// ── AiProvider trait (domain core depends on this, not on Z.ai) ──────────────

/// The domain-core interface for an AI provider. A concrete adapter (Z.ai,
/// General API, local model) implements this. The domain never names a
/// provider-specific field; it only calls `complete` with a
/// [`ProviderRequest`].
pub trait AiProvider: Send + Sync {
    /// Complete a request, returning a response string or a provider error.
    /// The adapter is responsible for timeout, retry, quota, and redaction
    /// per its [`ProviderConfig`] + [`OutboundPolicy`].
    fn complete(&self, request: &ProviderRequest) -> ProviderResult<String>;

    /// Human-readable adapter name for logs/audit (e.g. "zai_openai_compatible",
    /// "local_llama"). NOT stored in durable schema.
    fn adapter_name(&self) -> &str;
}

// ── ProviderConfig (config, not durable schema) ──────────────────────────────

/// Provider configuration. Lives in config (env/secret-manager/toml), NOT in
/// the durable semantic schema. §8.1: "สลับ base URL/model/provider ได้โดยไม่
/// migrate canonical data". The `kill_switch` disables all outbound calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// The endpoint URL (user-supplied; never embedded in durable schema).
    pub base_url: String,
    /// A SECRET-MANAGER REFERENCE (e.g. `op://vault/zai/key`), never the raw
    /// key. The deployment adapter resolves this at call time.
    pub api_key_ref: String,
    /// Routine-extraction model name (config, not durable schema).
    pub routine_model: String,
    /// Reasoning/synthesis model name (config).
    pub reasoning_model: String,
    /// When true, every `complete` call returns [`ProviderError::Disabled`]
    /// without touching the network. §8.1 kill switch.
    pub kill_switch: bool,
}

impl ProviderConfig {
    /// True when the kill switch is engaged — the provider must not be called.
    pub fn is_disabled(&self) -> bool {
        self.kill_switch
    }
}

// ── OutboundPolicy (deny-by-default, §8.2) ───────────────────────────────────

/// A decision from the outbound policy: was the request allowed or denied,
/// and why. The reason is safe to log (it never contains the detected
/// secret — only a label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundDecision {
    pub denied: bool,
    pub reason: String,
}

/// The deny-by-default outbound policy (§8.2). Consulted before any request
/// reaches the provider. Denies: (a) `local_only` requests unconditionally;
/// (b) requests whose prompt contains a detected secret pattern.
#[derive(Clone, Debug, Default)]
pub struct OutboundPolicy;

impl OutboundPolicy {
    /// Construct the default policy. (Preferred over `Default::default` for a
    /// unit struct so clippy's `default_constructed_unit_struct` stays clean.)
    pub const fn new() -> Self {
        Self
    }
}

impl OutboundPolicy {
    /// Check raw text + a local_only flag WITHOUT first allocating a
    /// `ProviderRequest`. This is the preferred entry point for callers that
    /// build a request from untrusted source text: it gates BEFORE the source
    /// is copied into a heap `String`, so a denied secret never lands in a
    /// `ProviderRequest.prompt` even momentarily (defense-in-depth, Task 4.2
    /// Validator F3).
    pub fn check_text(&self, text: &str, local_only: bool) -> OutboundDecision {
        if local_only {
            return OutboundDecision {
                denied: true,
                reason: "request is marked local_only".to_owned(),
            };
        }
        if let Some(kind) = detect_secret(text) {
            return OutboundDecision {
                denied: true,
                reason: format!("request contains a detected secret ({kind})"),
            };
        }
        OutboundDecision {
            denied: false,
            reason: "allowed".to_owned(),
        }
    }

    /// Redact known secret patterns from `text`, replacing values with
    /// `[REDACTED]`. Used by the observability log redactor (Task 6.2) and
    /// anywhere a string carrying untrusted content is about to enter a log
    /// or URL. Reuses `detect_secret`'s patterns.
    pub fn check_text_redact(text: &str) -> String {
        redact_secrets(text)
    }

    /// Check a request against the policy. Returns a decision the adapter
    /// honors before constructing the outbound HTTP call. Prefer
    /// [`Self::check_text`] when building a request from untrusted text (it
    /// gates before the source is copied into the request struct).
    pub fn check(&self, request: &ProviderRequest) -> OutboundDecision {
        self.check_text(&request.prompt, request.local_only)
    }
}

/// Detect a known secret pattern in a string. Returns the kind label
/// (`bearer`, `sk-key`, `access_token`) or `None`. Conservative — over-
/// detection is the safe direction for an egress filter.
///
/// `sk-` detection looks for the prefix followed by ≥16 chars of key material
/// (alphanumeric/`-`/`_`), matching realistic OpenAI/Z.ai key shapes
/// (`sk-proj-...`, `sk-1234...`). The previous length-arithmetic here was
/// inverted (it counted substring occurrences, not key length) and missed a
/// single real key — caught by the Task 4.1 Independent Validator.
fn detect_secret(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("bearer ") {
        return Some("bearer");
    }
    if let Some(idx) = lower.find("sk-") {
        let after = &lower[idx + 3..];
        let key_len = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .count();
        if key_len >= 16 {
            return Some("sk-key");
        }
    }
    if lower.contains("access_token=") || lower.contains("api_key=") {
        return Some("access_token");
    }
    None
}

/// Redact known secret patterns from `text`, replacing the secret VALUE with
/// `[REDACTED]` (keeping the marker/prefix visible for debuggability). Used
/// by log redaction (Task 6.2). Conservative — over-redaction is safe.
fn redact_secrets(text: &str) -> String {
    let mut out = text.to_owned();
    out = redact_value_after_marker(&out, "Bearer ");
    out = redact_value_after_marker(&out, "access_token=");
    out = redact_value_after_marker(&out, "api_key=");
    // Redact sk-<key> (case-insensitive find, replace key material).
    let lower = out.to_ascii_lowercase();
    if let Some(idx) = lower.find("sk-") {
        let key_len = out[idx + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .count();
        if key_len >= 16 {
            out.replace_range(idx..idx + 3 + key_len, "sk-[REDACTED]");
        }
    }
    out
}

/// Replace the value following `marker` (case-insensitive, up to whitespace/
/// `&`/`#`/end) with `[REDACTED]`.
fn redact_value_after_marker(input: &str, marker: &str) -> String {
    let marker_lower = marker.to_ascii_lowercase();
    let lower = input.to_ascii_lowercase();
    match lower.find(&marker_lower) {
        None => input.to_owned(),
        Some(idx) => {
            let before = &input[..idx];
            let after = &input[idx + marker.len()..];
            let end = after
                .find(|c: char| c.is_whitespace() || c == '&' || c == '#')
                .unwrap_or(after.len());
            let rest = &after[end..];
            format!("{before}{marker}[REDACTED]{rest}")
        }
    }
}

// ── ProviderError (§8.1 failure modes) ───────────────────────────────────────

/// Every §8.1 failure mode, with an actionable `is_retryable` classification.
/// A caller uses `is_retryable` to decide backoff vs dead-letter vs abort.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderError {
    /// Request exceeded the timeout budget. Retryable.
    Timeout,
    /// Quota budget exhausted (not a transient 429). NOT retryable without a
    /// quota top-up.
    QuotaExhausted,
    /// Provider returned 429. Retryable (with backoff).
    RateLimited,
    /// Provider returned a 5xx status. Retryable.
    ServerError(u16),
    /// Response was not valid JSON. NOT retryable (same input will fail again).
    InvalidJson(String),
    /// Stream ended mid-response. Retryable.
    PartialStream,
    /// Provider is unreachable (DNS/connection failure). Retryable.
    Outage,
    /// Kill switch is engaged — provider disabled by operator. NOT retryable
    /// until the switch is turned off.
    Disabled,
}

impl ProviderError {
    /// True if a caller should retry (with backoff) rather than dead-letter.
    /// Permanent failures (invalid JSON, quota exhausted, disabled) are not
    /// retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            ProviderError::Timeout
            | ProviderError::RateLimited
            | ProviderError::ServerError(_)
            | ProviderError::PartialStream
            | ProviderError::Outage => true,
            ProviderError::InvalidJson(_)
            | ProviderError::QuotaExhausted
            | ProviderError::Disabled => false,
        }
    }
}

// ── ComplianceRecord (§8.2 acknowledgement) ──────────────────────────────────

/// The auditable compliance acknowledgement that the user accepted the
/// provider's terms risk before the provider was enabled (§8.2: "ก่อนเปิด Z.ai
/// worker ต้องมี compliance record"). Captured once at enable time; never
/// embedded in durable semantic schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceRecord {
    /// What the user decided (free text, e.g. "approved Z.ai Coding Plan").
    pub user_decision: String,
    /// The endpoint the decision covers.
    pub endpoint: String,
    /// The workload class permitted (extraction/synthesis/consolidation).
    pub workload: String,
    /// Known terms risk acknowledged by the user.
    pub known_terms_risk: String,
    /// Data-retention terms (confirmed / unconfirmed + detail).
    pub retention_terms: String,
    /// Training-usage terms.
    pub training_terms: String,
    /// Processing region.
    pub processing_region: String,
    /// ISO-8601 UTC timestamp of the acknowledgement.
    pub acknowledged_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_bearer() {
        assert_eq!(detect_secret("Authorization: Bearer xyz"), Some("bearer"));
    }

    #[test]
    fn detect_none_for_clean_text() {
        assert_eq!(detect_secret("summarize this article"), None);
    }

    #[test]
    fn kill_switch_disables() {
        let config = ProviderConfig {
            base_url: "x".into(),
            api_key_ref: "op://v/k".into(),
            routine_model: "m".into(),
            reasoning_model: "m".into(),
            kill_switch: true,
        };
        assert!(config.is_disabled());
    }
}
