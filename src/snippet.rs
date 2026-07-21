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
    let candidates = value.map(normalize_value_candidates).unwrap_or_default();

    if let Some(value_byte_off) = locate_value(span_text, &candidates) {
        // Find the matched candidate's length.
        let matched_cand = candidates
            .iter()
            .find(|c| span_text[value_byte_off..].starts_with(c.as_str()))
            .cloned()
            .unwrap_or_default();
        let matched_byte_len = matched_cand.len();
        let (window, start_byte, _end_byte, truncated) =
            window_around(span_text, value_byte_off, matched_byte_len, 200);

        let ellipsis_prefix_len = if start_byte > 0 { "…".len() } else { 0 };
        let value_offset_in_window =
            value_byte_off.saturating_sub(start_byte) + ellipsis_prefix_len;

        let value_char_offset = window[..value_offset_in_window].chars().count();
        let value_char_len = matched_cand.chars().count();

        SnippetResult {
            excerpt: window,
            value_located: true,
            value_offset: Some(value_char_offset),
            value_len: Some(value_char_len),
            excerpt_truncated: truncated,
            additional_sources: additional_rendition_ids.to_vec(),
        }
    } else {
        let fallback: String = span_text.chars().take(300).collect();
        let truncated = span_text.chars().count() > 300;
        let excerpt = if truncated {
            format!("{fallback}…")
        } else {
            fallback
        };
        SnippetResult {
            excerpt,
            value_located: false,
            value_offset: None,
            value_len: None,
            excerpt_truncated: truncated,
            additional_sources: additional_rendition_ids.to_vec(),
        }
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

    // Deviation from spec: spec wrote `cands.sort_by(|a, b| b.len().cmp(&a.len()))`
    // but clippy::unnecessary_sort_by is enforced as -D warnings (build-breaking).
    // Equivalent: descending-by-length via `sort_by_key` + `Reverse`.
    cands.sort_by_key(|b| std::cmp::Reverse(b.len()));
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

/// Find the byte offset of the first matching candidate in `text`. Returns
/// `None` if no candidate matches. Candidates are tried in priority order
/// (caller passes longest-first via [`normalize_value_candidates`]).
pub(crate) fn locate_value(text: &str, candidates: &[String]) -> Option<usize> {
    candidates.iter().find_map(|c| text.find(c.as_str()))
}

/// Extract a window of approximately `half_window_chars` characters on each
/// side of the value match, snapped outward to whitespace, hard-capped to
/// 600 chars total.
///
/// Returns `(window, start_byte_offset_in_text, end_byte_offset_in_text, truncated)`.
pub(crate) fn window_around(
    text: &str,
    value_byte_offset: usize,
    value_len_bytes: usize,
    half_window_chars: usize,
) -> (String, usize, usize, bool) {
    const HARD_CAP_CHARS: usize = 600;

    let total_chars = text.chars().count();
    if total_chars <= HARD_CAP_CHARS {
        return (text.to_string(), 0, text.len(), false);
    }

    let value_char_offset = text[..value_byte_offset].chars().count();
    let value_char_end = value_char_offset
        + text[value_byte_offset..value_byte_offset + value_len_bytes]
            .chars()
            .count();

    let mut start_char = value_char_offset.saturating_sub(half_window_chars);
    let mut end_char = (value_char_end + half_window_chars).min(total_chars);

    if start_char > 0 {
        let chars: Vec<char> = text.chars().collect();
        while start_char > 0
            && !chars[start_char - 1].is_whitespace()
            && !chars[start_char].is_whitespace()
        {
            start_char -= 1;
        }
    }
    if end_char < total_chars {
        let chars: Vec<char> = text.chars().collect();
        while end_char < total_chars
            && !chars[end_char - 1].is_whitespace()
            && !chars[end_char].is_whitespace()
        {
            end_char += 1;
        }
    }

    let window_chars = end_char - start_char;
    if window_chars > HARD_CAP_CHARS {
        let left_excess = value_char_offset.saturating_sub(start_char);
        let right_excess = end_char.saturating_sub(value_char_end);
        if left_excess >= right_excess {
            start_char = end_char - HARD_CAP_CHARS;
        } else {
            end_char = start_char + HARD_CAP_CHARS;
        }
    }

    let start_byte = char_index_to_byte(text, start_char);
    let end_byte = char_index_to_byte(text, end_char);

    let mut window = String::with_capacity(end_byte - start_byte);
    if start_byte > 0 {
        window.push('…');
    }
    window.push_str(&text[start_byte..end_byte]);
    if end_byte < text.len() {
        window.push('…');
    }

    (window, start_byte, end_byte, true)
}

fn char_index_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or_else(|| s.len())
}

#[cfg(test)]
mod tests_locate {
    use super::locate_value;

    #[test]
    fn finds_first_candidate_in_text() {
        let cands = vec!["1.75%".to_string(), "1.75".to_string()];
        let text = "Risk-free rate 1.75% (10Y CGB)";
        let byte_off = text.find("1.75%").unwrap();
        let got = locate_value(text, &cands);
        assert_eq!(got, Some(byte_off));
    }

    #[test]
    fn falls_back_to_shorter_candidate() {
        let cands = vec!["¥361".to_string(), "361".to_string()];
        let text = "current price 361 per share";
        let byte_off = text.find("361").unwrap();
        let got = locate_value(text, &cands);
        assert_eq!(got, Some(byte_off));
    }

    #[test]
    fn no_match_returns_none() {
        let cands = vec!["99".to_string()];
        let text = "no number here";
        assert_eq!(locate_value(text, &cands), None);
    }

    #[test]
    fn empty_candidates_returns_none() {
        assert_eq!(locate_value("text", &[]), None);
    }
}

#[cfg(test)]
mod tests_window {
    use super::window_around;

    #[test]
    fn returns_full_text_when_already_small() {
        let text = "short text 1.75 here";
        let value_byte_off = text.find("1.75").unwrap();
        let (window, win_start, _, truncated) =
            window_around(text, value_byte_off, "1.75".len(), 200);
        assert!(!truncated);
        assert_eq!(window, text);
        assert_eq!(win_start, 0);
    }

    #[test]
    fn windows_to_around_400_chars_when_text_longer() {
        // Adapted: spec data summed to 500 chars, below the 600-char HARD_CAP,
        // so `truncated` was always false. Also had no internal whitespace,
        // which would have driven start_char to 0 and broken `win_start > 0`.
        // Use spaced tokens so the HARD_CAP triggers AND the snap logic lands
        // at a word boundary well inside the text.
        let prefix = "alpha ".repeat(60);
        let value = "VAL";
        let suffix = " beta".repeat(60);
        let text = format!("{prefix}{value}{suffix}");
        assert!(text.chars().count() > 600, "test data must exceed HARD_CAP");
        let value_byte_off = prefix.len();
        let (window, win_start, _, truncated) =
            window_around(&text, value_byte_off, value.len(), 200);
        assert!(truncated);
        assert!(win_start > 0);
        assert!(window.contains(value));
        assert!(
            window.chars().count() <= 450,
            "got len {}",
            window.chars().count()
        );
    }

    #[test]
    fn snaps_to_whitespace_on_left_cut() {
        // Adapted: implementation's snap-outward-to-whitespace invariant is
        // "char before cut is whitespace OR char at cut is whitespace" — the
        // window therefore starts at a word boundary (first letter of a word),
        // not at a whitespace char itself. Verify the cut is at a word
        // boundary by checking the char immediately *before* `win_start`.
        let prefix = "word ".repeat(60);
        let value = "V";
        let suffix = " tail ".repeat(60);
        let text = format!("{prefix}{value}{suffix}");
        let value_byte_off = prefix.len();
        let (_window, win_start, _, _truncated) =
            window_around(&text, value_byte_off, value.len(), 100);
        if win_start > 0 {
            let prev_char = text[..win_start].chars().last().unwrap_or('x');
            assert!(
                prev_char.is_whitespace(),
                "expected whitespace just before cut, got {prev_char:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests_facade {
    use super::*;
    use serde_json::json;

    #[test]
    fn locates_value_and_windows() {
        // Adapted: spec data summed to 505 chars, below the 600-char HARD_CAP,
        // so `excerpt_truncated` was always false. Bumped to >600 chars.
        let prefix = "x".repeat(300);
        let suffix = "y".repeat(300);
        let text = format!("{prefix}1.75%{suffix}");
        let result = build_value_snippet(&text, Some(&json!("1.75%")), &[]);
        assert!(result.value_located);
        assert!(result.excerpt_truncated);
        assert!(result.excerpt.contains("1.75%"));
        let off = result.value_offset.unwrap();
        let len = result.value_len.unwrap();
        let got: String = result.excerpt.chars().skip(off).take(len).collect();
        assert_eq!(got, "1.75%");
    }

    #[test]
    fn value_not_found_falls_back_to_300_chars() {
        let text = "a".repeat(1000);
        let result = build_value_snippet(&text, Some(&json!("zzz")), &[]);
        assert!(!result.value_located);
        assert!(result.excerpt_truncated);
        assert!(result.excerpt.chars().count() <= 301);
    }

    #[test]
    fn short_text_returns_full_untruncated() {
        let text = "Risk-free rate 1.75% (10Y CGB)";
        let result = build_value_snippet(text, Some(&json!("1.75%")), &[]);
        assert!(result.value_located);
        assert!(!result.excerpt_truncated);
        assert_eq!(result.excerpt, text);
    }

    #[test]
    fn additional_sources_passed_through() {
        let result = build_value_snippet(
            "short",
            Some(&json!("short")),
            &["rend2".to_string(), "rend3".to_string()],
        );
        assert_eq!(result.additional_sources, vec!["rend2", "rend3"]);
    }

    #[test]
    fn none_value_returns_fallback() {
        let text = "some text here";
        let result = build_value_snippet(text, None, &[]);
        assert!(!result.value_located);
        assert_eq!(result.excerpt, text);
    }
}
