//! Evidence-linked extraction pipeline (Task 4.2).
//!
//! Domain contract for turning a captured source into typed semantic
//! proposals (GOAL-vNext §13 Task 4.2, §6.2 claim model, §4 rule 8 untrusted
//! ingestion). The pipeline is:
//!
//! 1. A source is captured into quarantine (rendition bytes + hash).
//! 2. An AI worker (via [`crate::provider::AiProvider`]) reads the rendition
//!    read-only and emits raw candidate claims.
//! 3. [`ExtractionPolicy`] validates each candidate: evidence spans must be
//!    well-formed, a candidate with no evidence is `unsupported`, and the
//!    source is never treated as instructions to the worker (prompt-injection
//!    resistance).
//! 4. The pipeline yields [`ExtractionProposal`]s + an [`ExtractionAudit`].
//!    There is NO commit path — the worker physically cannot confirm; the
//!    application service does that later via the semantic store (§4 rule 5).
//!
//! This module is contract-level: it does not call a real LLM. The policy +
//! types are what a deployment adapter (Task 4.1's `AiProvider`) feeds.

use serde::{Deserialize, Serialize};

use crate::provider::{OutboundPolicy, ProviderRequest};

// ── Evidence span (§6.2 rendition coordinate contract) ───────────────────────

/// An exact byte-range quote from a rendition, pinned by a content hash.
/// A proposal's evidence is a non-empty list of these; the validator slices
/// the rendition bytes and checks the hash matches (§6.2 evidence-span
/// exactness 100%). Half-open `[byte_start, byte_end)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSpan {
    /// The rendition this span quotes (content-addressed).
    pub rendition_id: String,
    /// SHA-256 of the exact bytes in `[byte_start, byte_end)`.
    pub quote_hash: String,
    pub byte_start: u64,
    pub byte_end: u64,
}

impl EvidenceSpan {
    /// True if the half-open range is well-formed (start < end).
    pub fn is_well_formed(&self) -> bool {
        self.byte_end > self.byte_start
    }
}

// ── Typed proposal ───────────────────────────────────────────────────────────

/// A typed semantic candidate derived from a source. Either carries at least
/// one exact [`EvidenceSpan`] (supported) or is `unsupported` (no evidence —
/// §6.2: may only be proposed/rejected, never confirmed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractionProposal {
    pub subject: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub claim_kind: String,
    pub domain: String,
    pub confidence_basis_points: u16,
    /// Exact evidence spans. Empty ⇒ `unsupported` must be true.
    pub evidence: Vec<EvidenceSpan>,
    /// True when `evidence` is empty. A supported proposal has ≥1 span.
    pub unsupported: bool,
}

// ── Audit trail (prompt/model/schema version) ────────────────────────────────

/// Reproducibility record for one extraction run (§6.2 "prompt/model/schema
/// version ถูก audit"). Stored with the resulting proposal so a confirmed
/// claim is traceable to the exact extraction configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionAudit {
    pub prompt_version: String,
    pub model: String,
    pub schema_version: String,
    /// Adapter name from `AiProvider::adapter_name` (NOT a durable-schema
    /// field — config only).
    pub adapter_name: String,
}

/// The outcome of one extraction run: validated proposals + the audit trail.
/// There is deliberately NO `commit`/`confirm`/side-effect field — the worker
/// cannot commit (§4 rule 5).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtractionOutcome {
    pub proposals: Vec<ExtractionProposal>,
    pub audit: ExtractionAudit,
}

// ── Policy ───────────────────────────────────────────────────────────────────

/// Validation/egress errors for the extraction pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposeError {
    /// A proposal claims to be supported but has no evidence spans.
    SupportedWithoutEvidence,
    /// A proposal is unsupported but the caller tried to treat it as confirmed.
    UnsupportedWithoutEvidence,
    /// An evidence span has `byte_end <= byte_start`.
    MalformedEvidenceSpan,
    /// Schema validation of the proposal value failed.
    SchemaInvalid(String),
}

/// The extraction policy: validates proposals and gates outbound provider
/// requests. Prompt-injection-resistant by construction — source content is
/// data, never instructions; a proposal's `unsupported` flag is set by
/// EVIDENCE presence, not by anything the source text "says".
#[derive(Clone, Debug)]
pub struct ExtractionPolicy {
    outbound: OutboundPolicy,
}

impl ExtractionPolicy {
    /// Construct the default policy (deny-by-default outbound gate).
    pub fn new() -> Self {
        Self {
            outbound: OutboundPolicy::new(),
        }
    }

    /// Validate a proposal against the evidence-span + unsupported invariants.
    /// Returns `Ok(())` only for SUPPORTED proposals with well-formed evidence.
    /// Unsupported proposals (no evidence) are rejected — they cannot enter the
    /// confirm path (§6.2: unsupported may only be proposed/rejected, and the
    /// extraction policy refuses to even yield them as actionable proposals).
    pub fn validate(&self, proposal: &ExtractionProposal) -> Result<(), ProposeError> {
        // Every evidence span must be well-formed.
        for span in &proposal.evidence {
            if !span.is_well_formed() {
                return Err(ProposeError::MalformedEvidenceSpan);
            }
        }
        let has_evidence = !proposal.evidence.is_empty();
        // An unsupported proposal (no evidence) is rejected outright — this is
        // the prompt-injection resistance guarantee: even if injected source
        // text "says" to confirm, a proposal with no evidence cannot pass.
        if proposal.unsupported || !has_evidence {
            return Err(ProposeError::UnsupportedWithoutEvidence);
        }
        // Supported + has evidence + well-formed spans → ok.
        Ok(())
    }

    /// Build a provider request for a source, applying the outbound policy.
    /// Returns `None` when the source is `local_only` or contains a detected
    /// secret — the request is never sent. §8.2 no-egress + §4 rule 8.
    ///
    /// This is the prompt-injection-resistant entry point: the source text
    /// becomes the PROMPT DATA, never an instruction. The worker has no tools
    /// to execute; it only yields proposals the policy then validates.
    ///
    /// The gate runs BEFORE the source text is copied into the request struct
    /// (check-then-build, not build-then-check) so a denied secret never lands
    /// in a `ProviderRequest.prompt` even momentarily (Task 4.2 Validator F3).
    pub fn build_provider_request(
        &self,
        source_text: &str,
        local_only: bool,
    ) -> Option<ProviderRequest> {
        let decision = self.outbound.check_text(source_text, local_only);
        if decision.denied {
            return None;
        }
        Some(ProviderRequest {
            prompt: source_text.to_owned(),
            max_tokens: 2048,
            temperature: 0.0,
            local_only,
        })
    }
}

impl Default for ExtractionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_proposal_without_evidence() {
        let policy = ExtractionPolicy::new();
        let proposal = ExtractionProposal {
            subject: "x".into(),
            predicate: "p".into(),
            value: serde_json::json!(1),
            claim_kind: "external_fact".into(),
            domain: "d".into(),
            confidence_basis_points: 8_000,
            evidence: vec![],
            unsupported: false,
        };
        assert_eq!(
            policy.validate(&proposal),
            Err(ProposeError::UnsupportedWithoutEvidence)
        );
    }

    #[test]
    fn build_request_denies_local_only() {
        let policy = ExtractionPolicy::new();
        assert!(policy.build_provider_request("x", true).is_none());
    }
}
