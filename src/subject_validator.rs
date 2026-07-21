//! Subject Validator v1 — Phase 1.5 deterministic subject-name validator.
//!
//! Architecture (Wikidata pattern): engine vs. data separation.
//! - **Engine** (this file, Layer 0-2 + 5): stable Rust code, 5+ year lifetime.
//! - **Data** (rules/*.toml, Layer 3-4): TOML config reviewed annually.
//!
//! See `docs/plans/subject-validator-v1-spec.md` for full design.
//!
//! # Pipeline
//!
//! ```text
//! subject ──► L0 Normalize ──► L1 Mechanical ──► L2 Shape ──► L3 Rules ──► L4 Overrides ──► L5 Verdict
//! ```

use crate::quality::QualityTag;

/// Bumped when the engine's behavior or shape enum changes. Recorded in audit
/// trail so old claims can be re-validated in bulk when rules evolve.
pub const SUBJECT_VALIDATOR_VERSION: &str = "subject-validator-v1";

// ── Layer 2: SubjectShape ────────────────────────────────────────────────

/// The 23 structural shapes Layer 2 classifies a subject into. Layer 3
/// (rules/subject_rules.toml) maps each shape to a verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubjectShape {
    Empty, Slug, Filename, Url, Date, TimeExpr,
    NumberLed, CurrencyLed, Sentence, Question,
    ThaiPure, ThaiLatinMixed,
    Ticker, Acronym, TitleCase, LowercaseNoun,
    VerbLed, Demonstrative, MultiEntity, Possessive,
    WikiMarkup, Placeholder, Plain,
    Unknown,
}

impl SubjectShape {
    pub fn as_str(self) -> &'static str {
        match self {
            SubjectShape::Empty => "empty",
            SubjectShape::Slug => "slug",
            SubjectShape::Filename => "filename",
            SubjectShape::Url => "url",
            SubjectShape::Date => "date",
            SubjectShape::TimeExpr => "time_expr",
            SubjectShape::NumberLed => "number_led",
            SubjectShape::CurrencyLed => "currency_led",
            SubjectShape::Sentence => "sentence",
            SubjectShape::Question => "question",
            SubjectShape::ThaiPure => "thai_pure",
            SubjectShape::ThaiLatinMixed => "thai_latin_mixed",
            SubjectShape::Ticker => "ticker",
            SubjectShape::Acronym => "acronym",
            SubjectShape::TitleCase => "title_case",
            SubjectShape::LowercaseNoun => "lowercase_noun",
            SubjectShape::VerbLed => "verb_led",
            SubjectShape::Demonstrative => "demonstrative",
            SubjectShape::MultiEntity => "multi_entity",
            SubjectShape::Possessive => "possessive",
            SubjectShape::WikiMarkup => "wiki_markup",
            SubjectShape::Placeholder => "placeholder",
            SubjectShape::Plain => "plain",
            SubjectShape::Unknown => "unknown",
        }
    }
}

// ── Layer 5: SubjectVerdict ──────────────────────────────────────────────

/// Final verdict produced by Layer 5 after combining all signals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubjectVerdict {
    Accept,
    AcceptWithInfo,
    SoftFlag,
    DeferToLLM,
    Reject,
}

// ── SubjectReport ─────────────────────────────────────────────────────────

/// The complete result of validating one subject.
#[derive(Clone, Debug)]
pub struct SubjectReport {
    pub normalized: String,
    pub shape: SubjectShape,
    pub verdict: SubjectVerdict,
    pub quality_tags: Vec<QualityTag>,
    pub validator_version: &'static str,
}

// ── Layer 0: Normalize ──────────────────────────────────────────────────

/// NFC-normalize, strip zero-width chars, convert NBSP/thin/em-spaces to
/// regular ASCII space, collapse internal whitespace runs, trim.
///
/// Always runs; never rejects. The returned bool indicates whether
/// normalization actually changed the input (useful for emitting an info
/// tag in the caller).
pub(crate) fn normalize_subject(raw: &str) -> (String, bool) {
    use unicode_normalization::UnicodeNormalization;
    let normalized: String = raw
        .nfc()
        .filter(|&c| c != '\u{200B}' && c != '\u{200C}' && c != '\u{200D}' && c != '\u{FEFF}')
        .map(|c| match c {
            '\u{00A0}' | '\u{202F}' | '\u{2009}' | '\u{200A}' | '\u{2003}' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let fired = normalized != raw;
    (normalized, fired)
}

// ── Layer 1: Mechanical hard-fail ──────────────────────────────────────

/// Family C/D/F defects caught before shape classification. Any hit →
/// Critical, no further layers run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MechanicalDefect {
    Empty,
    PunctuationOnly,
    ControlChars,
    TabNewlineCr,
    WikiMarkupLeak,
    HtmlInjection,
    TemplateInjection,
    RtlOverride,
    Emoji,
    TooLong,
    TooShort,
}

const SUBJECT_MAX_LEN: usize = 80;
const SUBJECT_MIN_LEN: usize = 2;

/// Returns Some(defect) if Layer 1 should hard-reject; None if shape
/// classification should proceed. Input is the *normalized* subject.
pub(crate) fn check_subject_mechanical(normalized: &str) -> Option<MechanicalDefect> {
    if normalized.is_empty() {
        return Some(MechanicalDefect::Empty);
    }
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() < SUBJECT_MIN_LEN {
        return Some(MechanicalDefect::TooShort);
    }
    if chars.len() > SUBJECT_MAX_LEN {
        return Some(MechanicalDefect::TooLong);
    }
    // Punctuation-only: no alphanumeric chars and no Thai
    if !chars.iter().any(|c| c.is_alphanumeric() || ('\u{0E00}'..='\u{0E7F}').contains(c)) {
        return Some(MechanicalDefect::PunctuationOnly);
    }
    // Control chars (Unicode General_Category Cc, except tab/newline/CR which get their own bucket)
    if chars.iter().any(|&c| {
        c != '\t' && c != '\n' && c != '\r' && (c.is_control())
    }) {
        return Some(MechanicalDefect::ControlChars);
    }
    if chars.iter().any(|&c| c == '\t' || c == '\n' || c == '\r') {
        return Some(MechanicalDefect::TabNewlineCr);
    }
    // RTL override
    if chars.iter().any(|&c| c == '\u{202E}') {
        return Some(MechanicalDefect::RtlOverride);
    }
    // Emoji: chars with Emoji property that aren't ASCII digits/symbols.
    // Conservative approximation: any char in common emoji blocks.
    if chars.iter().any(|&c| is_emoji_like(c)) {
        return Some(MechanicalDefect::Emoji);
    }
    // HTML injection
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("<script") || lower.contains('<') && lower.contains('>') {
        return Some(MechanicalDefect::HtmlInjection);
    }
    // Template injection
    if normalized.contains("${") || normalized.contains("{{") || normalized.contains("%{") || normalized.contains("<%") {
        return Some(MechanicalDefect::TemplateInjection);
    }
    // Wiki markup leak
    if normalized.contains("[[") || normalized.contains("]]") || normalized.contains("'''") || normalized.starts_with("==") {
        return Some(MechanicalDefect::WikiMarkupLeak);
    }
    None
}

/// Conservative emoji detection — covers the most common ranges without
/// pulling in a unicode-emoji crate. Phase 2 can swap for `\p{Emoji}` if
/// needed.
fn is_emoji_like(c: char) -> bool {
    matches!(c as u32,
        0x1F300..=0x1FAFF |    // Misc Symbols & Pictographs .. Symbols & Pictographs Extended-A
        0x2600..=0x27BF |      // Misc Symbols .. Dingbats
        0x1F1E6..=0x1F1FF      // Regional indicator pairs (flags)
    )
}

#[cfg(test)]
mod tests_layer0 {
    use super::normalize_subject;

    #[test]
    fn passthrough_when_already_clean() {
        let (out, fired) = normalize_subject("CATL");
        assert_eq!(out, "CATL");
        assert!(!fired);
    }

    #[test]
    fn strips_zero_width_chars() {
        let (out, fired) = normalize_subject("CATL\u{200B}");
        assert_eq!(out, "CATL");
        assert!(fired);
    }

    #[test]
    fn converts_nbsp_to_space_then_trims() {
        let (out, fired) = normalize_subject("CATL\u{00A0}");
        assert_eq!(out, "CATL");
        assert!(fired);
    }

    #[test]
    fn collapses_internal_whitespace_runs() {
        let (out, fired) = normalize_subject("BYD   Group");
        assert_eq!(out, "BYD Group");
        assert!(fired);
    }

    #[test]
    fn nfc_normalizes_decomposed_chars() {
        // "é" as decomposed e + combining acute (U+0301)
        let decomposed = "CAFE\u{0301}";
        let (out, _fired) = normalize_subject(decomposed);
        assert_eq!(out, "CAFÉ");
    }

    #[test]
    fn empty_string_stays_empty() {
        let (out, _) = normalize_subject("");
        assert_eq!(out, "");
    }

    #[test]
    fn whitespace_only_becomes_empty() {
        let (out, _) = normalize_subject("   \t  ");
        assert_eq!(out, "");
    }
}

// ── Layer 2: Shape Classifier ───────────────────────────────────────────

use regex::Regex;
use std::sync::LazyLock;

static RE_ACRONYM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{2,8}$").unwrap());
static RE_TICKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{1,6}(\.[A-Z]{1,4})?$").unwrap());
static RE_SLUG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+){1,}$").unwrap());
static RE_FILENAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z0-9]+(_[a-z0-9]+){1,}$").unwrap());
static RE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(https?://|www\.|ftp://)").unwrap());
static RE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}(-\d{2})?$|^Q[1-4]\s+\d{4}$").unwrap());
static RE_CURRENCY_LED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[¥$€£฿]\s*[\d,.]+").unwrap());
static RE_NUMBER_LED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\d,.]+\s*[BMK]?\b").unwrap());

/// First-match-wins shape classifier. Input is the *normalized* subject.
/// Order matters: cheap structural tests first, language-specific last.
pub(crate) fn classify_shape(normalized: &str) -> SubjectShape {
    if normalized.is_empty() {
        return SubjectShape::Empty;
    }
    // Wiki markup (shouldn't reach here post-Layer 1, defensive)
    if normalized.contains("[[") || normalized.starts_with("==") {
        return SubjectShape::WikiMarkup;
    }
    if RE_ACRONYM.is_match(normalized) {
        return SubjectShape::Acronym;
    }
    if RE_TICKER.is_match(normalized) {
        return SubjectShape::Ticker;
    }
    // Date must be checked before Slug/Filename: ISO dates match the slug regex
    // (all-digit "words" joined by hyphens). Date is a more specific shape.
    if RE_DATE.is_match(normalized) {
        return SubjectShape::Date;
    }
    if RE_SLUG.is_match(normalized) {
        return SubjectShape::Slug;
    }
    if RE_FILENAME.is_match(normalized) {
        return SubjectShape::Filename;
    }
    if RE_URL.is_match(normalized) {
        return SubjectShape::Url;
    }
    if RE_CURRENCY_LED.is_match(normalized) {
        return SubjectShape::CurrencyLed;
    }
    if RE_NUMBER_LED.is_match(normalized) {
        return SubjectShape::NumberLed;
    }
    if normalized.ends_with('?') {
        return SubjectShape::Question;
    }
    // Multi-entity: comma joining capitalized tokens OR " X and Y " pattern
    if has_multi_entity_pattern(normalized) {
        return SubjectShape::MultiEntity;
    }
    // Thai detection — must run BEFORE Possessive/Sentence so "บมจ. ปตท."
    // (ends with '.') and similar Thai entities ending in punctuation
    // classify by script, not by trailing punctuation.
    let has_thai = normalized.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
    let has_latin = normalized.chars().any(|c| c.is_ascii_alphabetic());
    if has_thai && !has_latin {
        return SubjectShape::ThaiPure;
    }
    if has_thai && has_latin {
        return SubjectShape::ThaiLatinMixed;
    }
    // Sentence — ends with '.' and not all caps. Checked BEFORE Possessive
    // because "China's largest battery maker." is a full statement, not a
    // possessive-noun pattern.
    if normalized.ends_with('.') && !normalized.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        return SubjectShape::Sentence;
    }
    // Possessive
    if normalized.contains("'s") || normalized.ends_with('\'') {
        return SubjectShape::Possessive;
    }
    // Grammatical cues (English) — first-token-based
    let first_token = normalized.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let first_token_lower = first_token.as_str();

    // Verb-led
    if VERB_CUES.contains(&first_token_lower) {
        return SubjectShape::VerbLed;
    }
    // Demonstrative
    if DEMONSTRATIVES.contains(&first_token_lower) {
        return SubjectShape::Demonstrative;
    }
    // Lowercase noun — all alphabetic chars are lowercase (most common LLM fail)
    let all_alpha_lower = normalized.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_lowercase());
    if all_alpha_lower && normalized.chars().any(|c| c.is_alphabetic()) {
        return SubjectShape::LowercaseNoun;
    }
    // TitleCase — every word starts uppercase
    if is_title_case(normalized) {
        return SubjectShape::TitleCase;
    }
    // Sentence — ends with '.' and not all caps. Checked AFTER Thai + Title
    // detection so that Thai entities ending in '.' ("บมจ. ปตท.") classify
    // as ThaiPure, not Sentence. Only English full statements land here.
    if normalized.ends_with('.') && !normalized.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        return SubjectShape::Sentence;
    }
    SubjectShape::Plain
}

fn is_title_case(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    words.iter().all(|w| {
        w.chars().next().map_or(false, |c| c.is_uppercase())
    })
}

const VERB_CUES: &[&str] = &[
    "produced", "filed", "grew", "increased", "decreased", "reported",
    "announced", "launched", "shipped", "posted",
];

const DEMONSTRATIVES: &[&str] = &[
    "it", "this", "that", "these", "those", "the",
];

fn has_multi_entity_pattern(s: &str) -> bool {
    // "X, Y" with both X and Y starting uppercase
    let comma_joined = s.contains(',')
        && s.split(',').filter(|p| !p.trim().is_empty()).count() >= 2
        && s.split(',').all(|p| p.trim().chars().next().map_or(false, |c| c.is_uppercase()));
    if comma_joined {
        return true;
    }
    // "X and Y" with both capitalized (check original case, not lowercased)
    if s.contains(" and ") {
        let parts: Vec<&str> = s.split(" and ").collect();
        if parts.iter().all(|p| p.trim().chars().next().map_or(false, |c| c.is_uppercase())) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests_layer1 {
    use super::*;

    #[test]
    fn empty_after_normalize_is_empty_defect() {
        assert_eq!(check_subject_mechanical(""), Some(MechanicalDefect::Empty));
    }

    #[test]
    fn single_char_too_short() {
        assert_eq!(check_subject_mechanical("A"), Some(MechanicalDefect::TooShort));
    }

    #[test]
    fn over_80_chars_too_long() {
        let long = "A".repeat(81);
        assert_eq!(check_subject_mechanical(&long), Some(MechanicalDefect::TooLong));
    }

    #[test]
    fn punctuation_only_rejected() {
        assert_eq!(check_subject_mechanical("---"), Some(MechanicalDefect::PunctuationOnly));
        assert_eq!(check_subject_mechanical("..."), Some(MechanicalDefect::PunctuationOnly));
    }

    #[test]
    fn control_char_rejected() {
        assert_eq!(check_subject_mechanical("CA\u{0001}TL"), Some(MechanicalDefect::ControlChars));
    }

    #[test]
    fn tab_rejected() {
        assert_eq!(check_subject_mechanical("CA\tTL"), Some(MechanicalDefect::TabNewlineCr));
    }

    #[test]
    fn html_script_rejected() {
        assert_eq!(check_subject_mechanical("<script>alert(1)</script>"), Some(MechanicalDefect::HtmlInjection));
    }

    #[test]
    fn template_injection_rejected() {
        assert_eq!(check_subject_mechanical("${evil}"), Some(MechanicalDefect::TemplateInjection));
        assert_eq!(check_subject_mechanical("{{evil}}"), Some(MechanicalDefect::TemplateInjection));
    }

    #[test]
    fn wiki_markup_rejected() {
        assert_eq!(check_subject_mechanical("[[wiki]]"), Some(MechanicalDefect::WikiMarkupLeak));
        assert_eq!(check_subject_mechanical("==Heading=="), Some(MechanicalDefect::WikiMarkupLeak));
    }

    #[test]
    fn rtl_override_rejected() {
        assert_eq!(check_subject_mechanical("\u{202E}CATL"), Some(MechanicalDefect::RtlOverride));
    }

    #[test]
    fn emoji_rejected() {
        assert_eq!(check_subject_mechanical("CATL 🚀"), Some(MechanicalDefect::Emoji));
    }

    #[test]
    fn clean_entity_passes() {
        assert_eq!(check_subject_mechanical("CATL"), None);
        assert_eq!(check_subject_mechanical("BYD Group"), None);
        assert_eq!(check_subject_mechanical("บมจ. ปตท."), None);
    }
}

#[cfg(test)]
mod tests_layer2_structural {
    use super::classify_shape;
    use super::SubjectShape;

    #[test]
    fn empty_is_empty() {
        assert_eq!(classify_shape(""), SubjectShape::Empty);
    }
    #[test]
    fn acronym_all_caps_short() {
        assert_eq!(classify_shape("AAPL"), SubjectShape::Acronym);
        assert_eq!(classify_shape("BYD"), SubjectShape::Acronym);
    }
    #[test]
    fn ticker_with_dot_suffix() {
        assert_eq!(classify_shape("BRK.B"), SubjectShape::Ticker);
        assert_eq!(classify_shape("PTT.BK"), SubjectShape::Ticker);
    }
    #[test]
    fn slug_hyphen_separated() {
        assert_eq!(classify_shape("international-peers-deep"), SubjectShape::Slug);
        assert_eq!(classify_shape("thai-shipping-bf-report"), SubjectShape::Slug);
    }
    #[test]
    fn filename_underscore_separated() {
        assert_eq!(classify_shape("peers_primary_20F"), SubjectShape::Filename);
    }
    #[test]
    fn url_detected() {
        assert_eq!(classify_shape("https://example.com"), SubjectShape::Url);
        assert_eq!(classify_shape("www.example.com"), SubjectShape::Url);
    }
    #[test]
    fn date_iso_format() {
        assert_eq!(classify_shape("2026-07-21"), SubjectShape::Date);
        assert_eq!(classify_shape("Q3 2025"), SubjectShape::Date);
    }
    #[test]
    fn currency_led() {
        assert_eq!(classify_shape("¥361"), SubjectShape::CurrencyLed);
        assert_eq!(classify_shape("$1.2B"), SubjectShape::CurrencyLed);
    }
    #[test]
    fn number_led() {
        assert_eq!(classify_shape("4.470B"), SubjectShape::NumberLed);
    }
    #[test]
    fn question_mark() {
        assert_eq!(classify_shape("Who owns CATL?"), SubjectShape::Question);
    }
    #[test]
    fn multi_entity_comma() {
        assert_eq!(classify_shape("CATL, BYD, LG"), SubjectShape::MultiEntity);
    }
    #[test]
    fn possessive_with_s() {
        assert_eq!(classify_shape("Tesla's CFO"), SubjectShape::Possessive);
    }
}

#[cfg(test)]
mod tests_layer2_grammatical {
    use super::*;
    use super::classify_shape;

    #[test]
    fn thai_pure_no_latin() {
        assert_eq!(classify_shape("บมจ. ปตท."), SubjectShape::ThaiPure);
    }
    #[test]
    fn thai_latin_mixed() {
        assert_eq!(classify_shape("บมจ. ปตท. (PTT)"), SubjectShape::ThaiLatinMixed);
    }
    #[test]
    fn verb_led() {
        assert_eq!(classify_shape("Produced deliverable"), SubjectShape::VerbLed);
        assert_eq!(classify_shape("reported earnings"), SubjectShape::VerbLed);
    }
    #[test]
    fn demonstrative_led() {
        assert_eq!(classify_shape("the company"), SubjectShape::Demonstrative);
        assert_eq!(classify_shape("This stock"), SubjectShape::Demonstrative);
    }
    #[test]
    fn lowercase_noun_most_common_fail() {
        assert_eq!(classify_shape("risk-free rate"), SubjectShape::LowercaseNoun);
        assert_eq!(classify_shape("beta"), SubjectShape::LowercaseNoun);
        assert_eq!(classify_shape("current case price"), SubjectShape::LowercaseNoun);
    }
    #[test]
    fn title_case() {
        assert_eq!(classify_shape("BYD Group"), SubjectShape::TitleCase);
        assert_eq!(classify_shape("Tesla Inc"), SubjectShape::TitleCase);
    }
    #[test]
    fn sentence_with_period() {
        assert_eq!(classify_shape("China's largest battery maker."), SubjectShape::Sentence);
    }
    #[test]
    fn plain_fallback() {
        // Mixed-case non-title string falls through to Plain
        assert_eq!(classify_shape("iPhone 15"), SubjectShape::Plain);
    }
}
