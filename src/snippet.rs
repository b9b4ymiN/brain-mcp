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

/// Normalize a proposal's `value` (a `serde_json::Value`) into a list of
/// candidate search strings to look for in the span text. We try the
/// longest/most-specific candidate first and fall back to shorter forms.
///
/// Examples:
///   `"¥361"`    → `["¥361", "361"]`
///   `"1.75%"`   → `["1.75%", "1.75"]`
///   `"2,000.8B"`→ `["2,000.8B", "2000.8B", "2,000.8", "2000.8"]`
///   `361`        → `["361"]`
///   `null` / `[]` / `{}` → `[]`
pub(crate) fn normalize_value_candidates(value: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;
    let primary = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Vec::new(),
    };
    let mut cands: Vec<String> = vec![primary.clone()];

    let stripped_currency: String = primary
        .trim_start_matches(['¥', '$', '€', '£', '฿'])
        .to_string();
    if stripped_currency != primary {
        cands.push(stripped_currency.clone());
    }

    let stripped_pct: String = primary.trim_end_matches('%').to_string();
    if stripped_pct != primary {
        cands.push(stripped_pct);
    }

    let no_commas = remove_inter_digit_commas(&primary);
    if no_commas != primary {
        cands.push(no_commas);
    }

    let combined = remove_inter_digit_commas(&stripped_currency)
        .trim_end_matches('%')
        .to_string();
    if combined != primary && !cands.contains(&combined) {
        cands.push(combined);
    }

    cands.sort_by(|a, b| b.len().cmp(&a.len()));
    cands.dedup();
    cands
}

fn remove_inter_digit_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            if matches!(prev, Some(p) if p.is_ascii_digit())
                && matches!(next, Some(n) if n.is_ascii_digit())
            {
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests_normalize {
    use super::normalize_value_candidates;
    use serde_json::json;

    #[test]
    fn string_value_passes_through_with_and_without_currency() {
        let v = json!("¥361");
        let cands = normalize_value_candidates(&v);
        assert!(cands.contains(&"¥361".to_string()));
        assert!(cands.contains(&"361".to_string()));
    }

    #[test]
    fn percent_value_keeps_with_and_without_sign() {
        let v = json!("1.75%");
        let cands = normalize_value_candidates(&v);
        assert!(cands.contains(&"1.75%".to_string()));
        assert!(cands.contains(&"1.75".to_string()));
    }

    #[test]
    fn suffix_scaled_value_provides_with_and_without_commas() {
        let v = json!("2,000.8B");
        let cands = normalize_value_candidates(&v);
        assert!(cands.iter().any(|c| c == "2,000.8B"));
        assert!(cands.iter().any(|c| c == "2000.8B"));
    }

    #[test]
    fn number_value_stringifies() {
        let v = json!(361);
        let cands = normalize_value_candidates(&v);
        assert!(cands.contains(&"361".to_string()));
    }

    #[test]
    fn float_value_stringifies() {
        let v = json!(1.75);
        let cands = normalize_value_candidates(&v);
        assert!(cands.contains(&"1.75".to_string()));
    }

    #[test]
    fn none_value_returns_empty() {
        let cands = normalize_value_candidates(&json!(null));
        assert!(cands.is_empty());
    }

    #[test]
    fn array_or_object_returns_empty() {
        assert!(normalize_value_candidates(&json!([1, 2, 3])).is_empty());
        assert!(normalize_value_candidates(&json!({"a": 1})).is_empty());
    }
}
