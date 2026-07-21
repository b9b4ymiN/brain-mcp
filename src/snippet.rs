//! Evidence snippet builder (Phase 1.6 Review Clarity Part 2, C1).
//!
//! Replaces the "wall of text" excerpt (3000+ char chunk dump produced by
//! `EvidenceSpan::whole_rendition()` + `"\n---\n"` join) with a tight,
//! value-anchored window matching the canonical snippet shape used by
//! Google Search, GitHub code search, Wikidata references, and LlamaIndex
//! SentenceWindowRetriever.
//!
//! # Shape
//!
//! ±200 chars around the value, snapped outward to whitespace, hard cap 600
//! chars. If the value isn't found in the span (LLM inferred it), fall back
//! to first 300 chars of the span with `value_located: false` — honest
//! rather than dumping the whole chunk.
//!
//! # Security
//!
//! This module produces plain `String`s only. The caller (Svelte Inbox)
//! renders them via `{...}` plain interpolation, NOT `{@html}` — XSS-safe
//! by construction. The caller receives `value_offset` + `value_len` so it
//! can wrap the middle slice in `<mark>` itself.

use serde::Serialize;

/// Output of [`build_value_snippet`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnippetResult {
    /// The windowed excerpt (already truncated to ≤600 chars with `…` if needed).
    pub excerpt: String,
    /// True iff `value` (or one of its normalized candidates) was found in span_text.
    pub value_located: bool,
    /// Char offset of the value match within `excerpt` (None if `!value_located`).
    pub value_offset: Option<usize>,
    /// Char length of the value match within `excerpt` (None if `!value_located`).
    pub value_len: Option<usize>,
    /// True if the excerpt was windowed/clamped (vs returned in full).
    pub excerpt_truncated: bool,
    /// Rendition IDs of spans that were NOT chosen as primary (for collapsed "N more sources" UI).
    pub additional_sources: Vec<String>,
}

/// Build a value-anchored snippet.
///
/// - `span_text`: decrypted text of the primary span (the one we'll excerpt from).
/// - `value`: the proposal's `value` field (used to find the highlight offset).
/// - `additional_rendition_ids`: rendition IDs of other spans (passed through to `additional_sources`).
pub fn build_value_snippet(
    span_text: &str,
    value: Option<&serde_json::Value>,
    additional_rendition_ids: &[String],
) -> SnippetResult {
    // Implementation in Task 4.
    let _ = (span_text, value, additional_rendition_ids);
    SnippetResult {
        excerpt: String::new(),
        value_located: false,
        value_offset: None,
        value_len: None,
        excerpt_truncated: false,
        additional_sources: Vec::new(),
    }
}
