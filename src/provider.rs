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
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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

impl ComplianceRecord {
    /// Append this record as one JSON line to `path` (creating the file/parent
    /// directory if needed). This is a standalone audit log — NOT the durable
    /// semantic schema (see module docs) — so a compliance acknowledgement is
    /// recoverable even though it never touches `SemanticStore`.
    pub fn persist_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let line = serde_json::to_string(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")
    }

    /// Read every record previously appended to `path` (one JSON object per
    /// line). Returns an empty vec if the file does not exist yet.
    pub fn load_all_from(path: &Path) -> io::Result<Vec<ComplianceRecord>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = std::fs::File::open(path)?;
        std::io::BufReader::new(file)
            .lines()
            .filter(|line| !matches!(line, Ok(l) if l.trim().is_empty()))
            .map(|line| {
                let line = line?;
                serde_json::from_str(&line)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            })
            .collect()
    }
}

// ── HttpTransport — injectable transport for ZaiHttpAdapter ─────────────────

/// The raw HTTP outcome of a transport call: a status code + response body.
/// [`ZaiHttpAdapter`] classifies this into a content string or a
/// [`ProviderError`] — the transport itself only reports connection-level
/// failures (timeout/outage/partial-stream) as `Err`.
#[derive(Clone, Debug)]
pub struct TransportResponse {
    pub status: u16,
    pub body: String,
}

/// An injectable HTTP transport. Production code uses [`ReqwestTransport`];
/// tests implement this trait directly with canned responses — no boxed
/// closures, which keeps clippy's `type_complexity` lint clean (Task D1: the
/// previous `Box<dyn Fn(&str, &str) -> Result<String, ProviderError>>` field
/// this replaces).
pub trait HttpTransport: Send + Sync {
    /// Send `body` (a JSON request) to `url` with bearer `api_key`. Returns
    /// the raw response on any completed HTTP exchange (even 4xx/5xx status);
    /// returns `Err` only for a connection-level failure (timeout, DNS/TCP
    /// failure, or the body failing to read after headers arrived).
    fn send(
        &self,
        url: &str,
        api_key: &str,
        body: &str,
    ) -> Result<TransportResponse, ProviderError>;
}

/// Default per-request HTTP timeout for the production [`ReqwestTransport`]
/// (600 s / 10 min). Found live 2026-07-19: the previous hardcoded 60 s was
/// too short for `brain_extract` against reasoning models — a single ~4 KB
/// chunk measured 119 s end-to-end against Z.ai glm-4.6. Operators should
/// usually tune this via `[provider] timeout_secs` (which threads through
/// `ProviderSection::resolve` → `ZaiHttpAdapter::with_timeout`); the
/// constant here is the fallback when no config is supplied.
pub const DEFAULT_PROVIDER_TIMEOUT_SECS: u64 = 600;

/// The production [`HttpTransport`]: a blocking `reqwest` client (rustls).
/// Blocking is safe today because nothing calls [`AiProvider::complete`] from
/// an async context yet (Phase 4 is contract-level — see module docs); a
/// future async caller wraps the call in `tokio::task::spawn_blocking`
/// instead of this trait growing an async fn.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    /// Construct with the default timeout (`DEFAULT_PROVIDER_TIMEOUT_SECS`).
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(DEFAULT_PROVIDER_TIMEOUT_SECS))
    }

    /// Construct with an explicit per-request timeout. `Duration::ZERO` maps
    /// to "no timeout" at the reqwest layer — only do this for debugging a
    /// stuck connection, never in production (a hung provider call would
    /// then block forever).
    pub fn with_timeout(timeout: Duration) -> Self {
        let mut builder = reqwest::blocking::Client::builder();
        if !timeout.is_zero() {
            builder = builder.timeout(timeout);
        }
        Self {
            client: builder
                .build()
                .expect("reqwest client builds with default TLS config"),
        }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(
        &self,
        url: &str,
        api_key: &str,
        body: &str,
    ) -> Result<TransportResponse, ProviderError> {
        let result = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .body(body.to_owned())
            .send();
        let response = match result {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(ProviderError::Timeout),
            Err(_) => return Err(ProviderError::Outage),
        };
        let status = response.status().as_u16();
        match response.text() {
            Ok(body) => Ok(TransportResponse { status, body }),
            Err(_) => Err(ProviderError::PartialStream),
        }
    }
}

// ── Retry policy (bounded, exponential backoff) ──────────────────────────────

/// Bounded exponential backoff. `max_attempts` bounds the loop (no unbounded
/// retry); `delay_for` doubles `base_delay` per attempt, capped at
/// `max_delay`.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// A policy with near-zero delays, for tests that exercise the retry loop
    /// without slowing down the suite.
    pub fn fast_for_tests() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
        }
    }

    fn delay_for(&self, attempt: u32) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
        exp.min(self.max_delay)
    }
}

// ── Dead-letter (exhausted retries / non-retryable failures) ────────────────

/// A record of a request that could not be completed — either because the
/// outbound policy denied it or because the transport failed permanently
/// (non-retryable error) or exhausted its retry budget. `prompt` is ALWAYS
/// the [`OutboundPolicy::check_text_redact`]-ed text, never the raw prompt —
/// this is what makes the dead-letter path safe for a `local_only`/secret
/// request to pass through on its way to being denied (Task D1 DoD:
/// intercepted-outbound must not appear in the dead-letter record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    pub adapter_name: String,
    pub redacted_prompt: String,
    pub error: ProviderError,
    pub attempts: u32,
    pub occurred_at: String,
}

// ── ZaiHttpAdapter — concrete AiProvider for Z.ai OpenAI-compatible endpoint ─

/// A concrete `AiProvider` for the Z.ai (GLM) OpenAI-compatible chat
/// completions endpoint (Task D1). The HTTP call goes through an injectable
/// [`HttpTransport`] (production: [`ReqwestTransport`]; tests: any transport
/// impl with canned responses).
///
/// Every `complete` call, on every attempt (including retries), re-runs the
/// [`OutboundPolicy`] check before touching the transport — a `local_only`/
/// secret-bearing request never reaches the network, and any resulting
/// dead-letter/log entry carries only redacted text.
pub struct ZaiHttpAdapter {
    config: ProviderConfig,
    transport: Box<dyn HttpTransport>,
    retry: RetryPolicy,
    compliance: ComplianceRecord,
    dead_letters: Mutex<Vec<DeadLetterEntry>>,
}

impl ZaiHttpAdapter {
    /// Construct a production adapter with a real `reqwest` transport.
    /// Persists `compliance` to `compliance_log_path` before returning — a
    /// live adapter cannot exist without a recorded acknowledgement (§8.2).
    pub fn new(
        config: ProviderConfig,
        compliance: ComplianceRecord,
        compliance_log_path: PathBuf,
    ) -> io::Result<Self> {
        Self::with_transport(
            config,
            compliance,
            compliance_log_path,
            Box::new(ReqwestTransport::new()),
        )
    }

    /// Construct a production adapter with a real `reqwest` transport using
    /// an explicit per-request `timeout`. Used by `serve()` to thread
    /// `[provider] timeout_secs` into the transport without touching the
    /// mock-injectable `with_transport` constructor. See
    /// `DEFAULT_PROVIDER_TIMEOUT_SECS` for why this needs to be tunable.
    pub fn with_timeout(
        config: ProviderConfig,
        compliance: ComplianceRecord,
        compliance_log_path: PathBuf,
        timeout: Duration,
    ) -> io::Result<Self> {
        Self::with_transport(
            config,
            compliance,
            compliance_log_path,
            Box::new(ReqwestTransport::with_timeout(timeout)),
        )
    }

    /// Construct an adapter with an injected transport (tests use this with a
    /// scripted/mock [`HttpTransport`] — no network).
    pub fn with_transport(
        config: ProviderConfig,
        compliance: ComplianceRecord,
        compliance_log_path: PathBuf,
        transport: Box<dyn HttpTransport>,
    ) -> io::Result<Self> {
        compliance.persist_to(&compliance_log_path)?;
        Ok(Self {
            config,
            transport,
            retry: RetryPolicy::default(),
            compliance,
            dead_letters: Mutex::new(Vec::new()),
        })
    }

    /// Override the retry policy (tests use [`RetryPolicy::fast_for_tests`]).
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The compliance record this adapter was constructed with.
    pub fn compliance_record(&self) -> &ComplianceRecord {
        &self.compliance
    }

    /// Every dead-lettered request recorded so far (redacted prompts only).
    pub fn dead_letters(&self) -> Vec<DeadLetterEntry> {
        self.dead_letters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn record_dead_letter(&self, request: &ProviderRequest, error: &ProviderError, attempts: u32) {
        let entry = DeadLetterEntry {
            adapter_name: self.adapter_name().to_owned(),
            redacted_prompt: safe_prompt_for_audit(request),
            error: error.clone(),
            attempts,
            occurred_at: chrono_now_rfc3339(),
        };
        tracing::warn!(
            adapter = %entry.adapter_name,
            attempts,
            error = ?entry.error,
            "zai request moved to dead-letter"
        );
        self.dead_letters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry);
    }
}

impl AiProvider for ZaiHttpAdapter {
    fn complete(&self, request: &ProviderRequest) -> ProviderResult<String> {
        if self.config.is_disabled() {
            return Err(ProviderError::Disabled);
        }
        let policy = OutboundPolicy::new();
        let mut last_err = ProviderError::Outage;
        let mut attempts_made = 0;

        for attempt in 1..=self.retry.max_attempts {
            attempts_made = attempt;
            let decision = policy.check(request);
            if decision.denied {
                last_err = ProviderError::InvalidJson(format!(
                    "outbound policy denied: {reason}",
                    reason = decision.reason
                ));
                self.record_dead_letter(request, &last_err, attempts_made);
                return Err(last_err);
            }

            tracing::debug!(
                attempt,
                adapter = self.adapter_name(),
                prompt = %safe_prompt_for_audit(request),
                "sending zai request"
            );

            let api_key = match resolve_api_key(&self.config.api_key_ref) {
                Ok(key) => key,
                Err(err) => {
                    last_err = err;
                    self.record_dead_letter(request, &last_err, attempts_made);
                    return Err(last_err);
                }
            };
            let body = build_request_body(&self.config, request);

            let outcome = self
                .transport
                .send(&self.config.base_url, &api_key, &body)
                .and_then(classify_response);

            match outcome {
                Ok(content) => return Ok(content),
                Err(err) => {
                    last_err = err;
                    if !last_err.is_retryable() || attempt == self.retry.max_attempts {
                        break;
                    }
                    std::thread::sleep(self.retry.delay_for(attempt));
                }
            }
        }

        self.record_dead_letter(request, &last_err, attempts_made);
        Err(last_err)
    }

    fn adapter_name(&self) -> &str {
        "zai_openai_compatible"
    }
}

/// The text safe to put in a dead-letter entry or a log/telemetry line for
/// `request`. A `local_only` request is withheld entirely — by definition it
/// must never leave the local machine, so pattern-based redaction (which only
/// strips *recognized secret shapes*) is not sufficient on its own; any other
/// request goes through [`OutboundPolicy::check_text_redact`].
fn safe_prompt_for_audit(request: &ProviderRequest) -> String {
    if request.local_only {
        "[local_only — content withheld]".to_owned()
    } else {
        OutboundPolicy::check_text_redact(&request.prompt)
    }
}

/// Resolve `api_key_ref` at call time. Only the `env:VARNAME` scheme is
/// implemented today (this repo has no secret-manager client) — the raw key
/// is never stored on [`ProviderConfig`] or the adapter, only read
/// transiently here. A missing/unresolvable ref maps to `Outage` (retryable —
/// setting the env var and retrying will succeed).
fn resolve_api_key(api_key_ref: &str) -> Result<String, ProviderError> {
    match api_key_ref.strip_prefix("env:") {
        Some(var_name) => std::env::var(var_name).map_err(|_| ProviderError::Outage),
        None => Err(ProviderError::Outage),
    }
}

/// Build the OpenAI-compatible chat-completions request body.
///
/// `ProviderRequest` carries no task-class field (it is deliberately
/// provider-agnostic — see module docs), so there is no signal here to choose
/// between `routine_model` and `reasoning_model`. Task D1 always routes to
/// `routine_model`; per-call model routing is a caller-side decision for a
/// later task once a task-class concept exists.
fn build_request_body(config: &ProviderConfig, request: &ProviderRequest) -> String {
    serde_json::json!({
        "model": config.routine_model,
        "messages": [{"role": "user", "content": request.prompt}],
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
        "response_format": {"type": "json_object"},
    })
    .to_string()
}

/// Classify a completed HTTP exchange into the extracted message content or a
/// `ProviderError`. 429 is disambiguated into `RateLimited` vs
/// `QuotaExhausted` by inspecting the error body (OpenAI-compatible APIs
/// carry the distinction in `error.type`/`error.message`, not the status
/// code alone).
fn classify_response(resp: TransportResponse) -> Result<String, ProviderError> {
    match resp.status {
        200..=299 => extract_content(&resp.body),
        429 => {
            if body_indicates_quota_exhausted(&resp.body) {
                Err(ProviderError::QuotaExhausted)
            } else {
                Err(ProviderError::RateLimited)
            }
        }
        500..=599 => Err(ProviderError::ServerError(resp.status)),
        other => Err(ProviderError::InvalidJson(format!(
            "unexpected status {other}"
        ))),
    }
}

fn body_indicates_quota_exhausted(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("insufficient_quota") || lower.contains("quota") || lower.contains("balance")
}

/// Extract `choices[0].message.content` from an OpenAI-compatible chat
/// completion response. Any malformed envelope is `InvalidJson` (permanent —
/// the same input will fail again, no point retrying).
fn extract_content(body: &str) -> Result<String, ProviderError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| ProviderError::InvalidJson(e.to_string()))?;
    value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::InvalidJson("missing choices[0].message.content".to_owned()))
}

/// Bounded JSON repair: strip common LLM-response wrapping (markdown code
/// fences, surrounding whitespace) and retry parsing. "Bounded" means a fixed
/// small set of transformations — never an iterative/unbounded retry loop.
pub fn repair_json(text: &str) -> Result<serde_json::Value, serde_json::Error> {
    if let Ok(v) = serde_json::from_str(text) {
        return Ok(v);
    }
    let trimmed = text.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let stripped = stripped.strip_suffix("```").unwrap_or(stripped);
    serde_json::from_str(stripped.trim())
}

fn chrono_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
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
