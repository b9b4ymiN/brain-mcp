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
    Empty,
    Slug,
    Filename,
    Url,
    Date,
    TimeExpr,
    NumberLed,
    CurrencyLed,
    Sentence,
    Question,
    ThaiPure,
    ThaiLatinMixed,
    Ticker,
    Acronym,
    TitleCase,
    LowercaseNoun,
    VerbLed,
    Demonstrative,
    MultiEntity,
    Possessive,
    WikiMarkup,
    Placeholder,
    Plain,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    if !chars
        .iter()
        .any(|c| c.is_alphanumeric() || ('\u{0E00}'..='\u{0E7F}').contains(c))
    {
        return Some(MechanicalDefect::PunctuationOnly);
    }
    // Control chars (Unicode General_Category Cc, except tab/newline/CR which get their own bucket)
    if chars
        .iter()
        .any(|&c| c != '\t' && c != '\n' && c != '\r' && (c.is_control()))
    {
        return Some(MechanicalDefect::ControlChars);
    }
    if chars.iter().any(|&c| c == '\t' || c == '\n' || c == '\r') {
        return Some(MechanicalDefect::TabNewlineCr);
    }
    // RTL override
    if chars.contains(&'\u{202E}') {
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
    if normalized.contains("${")
        || normalized.contains("{{")
        || normalized.contains("%{")
        || normalized.contains("<%")
    {
        return Some(MechanicalDefect::TemplateInjection);
    }
    // Wiki markup leak
    if normalized.contains("[[")
        || normalized.contains("]]")
        || normalized.contains("'''")
        || normalized.starts_with("==")
    {
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

static RE_ACRONYM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z]{2,8}$").unwrap());
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
    let has_thai = normalized
        .chars()
        .any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
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
    if normalized.ends_with('.')
        && !normalized
            .chars()
            .all(|c| !c.is_alphabetic() || c.is_uppercase())
    {
        return SubjectShape::Sentence;
    }
    // Possessive
    if normalized.contains("'s") || normalized.ends_with('\'') {
        return SubjectShape::Possessive;
    }
    // Grammatical cues (English) — first-token-based
    let first_token = normalized
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
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
    let all_alpha_lower = normalized
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_lowercase());
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
    if normalized.ends_with('.')
        && !normalized
            .chars()
            .all(|c| !c.is_alphabetic() || c.is_uppercase())
    {
        return SubjectShape::Sentence;
    }
    SubjectShape::Plain
}

fn is_title_case(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    words
        .iter()
        .all(|w| w.chars().next().is_some_and(|c| c.is_uppercase()))
}

const VERB_CUES: &[&str] = &[
    "produced",
    "filed",
    "grew",
    "increased",
    "decreased",
    "reported",
    "announced",
    "launched",
    "shipped",
    "posted",
];

const DEMONSTRATIVES: &[&str] = &["it", "this", "that", "these", "those", "the"];

fn has_multi_entity_pattern(s: &str) -> bool {
    // "X, Y" with both X and Y starting uppercase
    let comma_joined = s.contains(',')
        && s.split(',').filter(|p| !p.trim().is_empty()).count() >= 2
        && s.split(',')
            .all(|p| p.trim().chars().next().is_some_and(|c| c.is_uppercase()));
    if comma_joined {
        return true;
    }
    // "X and Y" with both capitalized (check original case, not lowercased)
    if s.contains(" and ") {
        let parts: Vec<&str> = s.split(" and ").collect();
        if parts
            .iter()
            .all(|p| p.trim().chars().next().is_some_and(|c| c.is_uppercase()))
        {
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
        assert_eq!(
            check_subject_mechanical("A"),
            Some(MechanicalDefect::TooShort)
        );
    }

    #[test]
    fn over_80_chars_too_long() {
        let long = "A".repeat(81);
        assert_eq!(
            check_subject_mechanical(&long),
            Some(MechanicalDefect::TooLong)
        );
    }

    #[test]
    fn punctuation_only_rejected() {
        assert_eq!(
            check_subject_mechanical("---"),
            Some(MechanicalDefect::PunctuationOnly)
        );
        assert_eq!(
            check_subject_mechanical("..."),
            Some(MechanicalDefect::PunctuationOnly)
        );
    }

    #[test]
    fn control_char_rejected() {
        assert_eq!(
            check_subject_mechanical("CA\u{0001}TL"),
            Some(MechanicalDefect::ControlChars)
        );
    }

    #[test]
    fn tab_rejected() {
        assert_eq!(
            check_subject_mechanical("CA\tTL"),
            Some(MechanicalDefect::TabNewlineCr)
        );
    }

    #[test]
    fn html_script_rejected() {
        assert_eq!(
            check_subject_mechanical("<script>alert(1)</script>"),
            Some(MechanicalDefect::HtmlInjection)
        );
    }

    #[test]
    fn template_injection_rejected() {
        assert_eq!(
            check_subject_mechanical("${evil}"),
            Some(MechanicalDefect::TemplateInjection)
        );
        assert_eq!(
            check_subject_mechanical("{{evil}}"),
            Some(MechanicalDefect::TemplateInjection)
        );
    }

    #[test]
    fn wiki_markup_rejected() {
        assert_eq!(
            check_subject_mechanical("[[wiki]]"),
            Some(MechanicalDefect::WikiMarkupLeak)
        );
        assert_eq!(
            check_subject_mechanical("==Heading=="),
            Some(MechanicalDefect::WikiMarkupLeak)
        );
    }

    #[test]
    fn rtl_override_rejected() {
        assert_eq!(
            check_subject_mechanical("\u{202E}CATL"),
            Some(MechanicalDefect::RtlOverride)
        );
    }

    #[test]
    fn emoji_rejected() {
        assert_eq!(
            check_subject_mechanical("CATL 🚀"),
            Some(MechanicalDefect::Emoji)
        );
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
    use super::SubjectShape;
    use super::classify_shape;

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
        assert_eq!(
            classify_shape("international-peers-deep"),
            SubjectShape::Slug
        );
        assert_eq!(
            classify_shape("thai-shipping-bf-report"),
            SubjectShape::Slug
        );
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
    use super::classify_shape;
    use super::*;

    #[test]
    fn thai_pure_no_latin() {
        assert_eq!(classify_shape("บมจ. ปตท."), SubjectShape::ThaiPure);
    }
    #[test]
    fn thai_latin_mixed() {
        assert_eq!(
            classify_shape("บมจ. ปตท. (PTT)"),
            SubjectShape::ThaiLatinMixed
        );
    }
    #[test]
    fn verb_led() {
        assert_eq!(
            classify_shape("Produced deliverable"),
            SubjectShape::VerbLed
        );
        assert_eq!(classify_shape("reported earnings"), SubjectShape::VerbLed);
    }
    #[test]
    fn demonstrative_led() {
        assert_eq!(classify_shape("the company"), SubjectShape::Demonstrative);
        assert_eq!(classify_shape("This stock"), SubjectShape::Demonstrative);
    }
    #[test]
    fn lowercase_noun_most_common_fail() {
        assert_eq!(
            classify_shape("risk-free rate"),
            SubjectShape::LowercaseNoun
        );
        assert_eq!(classify_shape("beta"), SubjectShape::LowercaseNoun);
        assert_eq!(
            classify_shape("current case price"),
            SubjectShape::LowercaseNoun
        );
    }
    #[test]
    fn title_case() {
        assert_eq!(classify_shape("BYD Group"), SubjectShape::TitleCase);
        assert_eq!(classify_shape("Tesla Inc"), SubjectShape::TitleCase);
    }
    #[test]
    fn sentence_with_period() {
        assert_eq!(
            classify_shape("China's largest battery maker."),
            SubjectShape::Sentence
        );
    }
    #[test]
    fn plain_fallback() {
        // Mixed-case non-title string falls through to Plain
        assert_eq!(classify_shape("iPhone 15"), SubjectShape::Plain);
    }
}

// ── Layer 3: Rules (TOML) ────────────────────────────────────────────────

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct SubjectRulesFile {
    pub last_reviewed: String,
    pub version: String,
    pub verdicts: std::collections::HashMap<String, String>,
}

/// Parsed Layer 3 rules — shape name → verdict.
#[derive(Clone, Debug)]
pub struct SubjectRules {
    pub last_reviewed: String,
    pub version: String,
    pub verdicts: std::collections::HashMap<SubjectShape, SubjectVerdict>,
}

impl SubjectRules {
    /// Load from a TOML string. Validates every shape has a verdict and
    /// every verdict is one of the 5 known values.
    pub fn parse(toml_str: &str) -> Result<Self, RulesError> {
        let file: SubjectRulesFile = toml::from_str(toml_str).map_err(RulesError::TomlSyntax)?;
        let mut verdicts = std::collections::HashMap::new();
        for (shape_name, verdict_str) in &file.verdicts {
            let shape = parse_shape(shape_name)
                .ok_or_else(|| RulesError::UnknownShape(shape_name.clone()))?;
            let verdict = parse_verdict(verdict_str)
                .ok_or_else(|| RulesError::UnknownVerdict(verdict_str.clone()))?;
            verdicts.insert(shape, verdict);
        }
        // Validate completeness: every shape variant must have a verdict
        for s in ALL_SHAPES.iter() {
            if !verdicts.contains_key(s) {
                return Err(RulesError::MissingShape(s.as_str().to_string()));
            }
        }
        Ok(Self {
            last_reviewed: file.last_reviewed,
            version: file.version,
            verdicts,
        })
    }

    pub fn verdict_for(&self, shape: SubjectShape) -> SubjectVerdict {
        self.verdicts
            .get(&shape)
            .copied()
            .unwrap_or(SubjectVerdict::DeferToLLM)
    }
}

pub(crate) const ALL_SHAPES: &[SubjectShape] = &[
    SubjectShape::Empty,
    SubjectShape::Slug,
    SubjectShape::Filename,
    SubjectShape::Url,
    SubjectShape::Date,
    SubjectShape::TimeExpr,
    SubjectShape::NumberLed,
    SubjectShape::CurrencyLed,
    SubjectShape::Sentence,
    SubjectShape::Question,
    SubjectShape::ThaiPure,
    SubjectShape::ThaiLatinMixed,
    SubjectShape::Ticker,
    SubjectShape::Acronym,
    SubjectShape::TitleCase,
    SubjectShape::LowercaseNoun,
    SubjectShape::VerbLed,
    SubjectShape::Demonstrative,
    SubjectShape::MultiEntity,
    SubjectShape::Possessive,
    SubjectShape::WikiMarkup,
    SubjectShape::Placeholder,
    SubjectShape::Plain,
    SubjectShape::Unknown,
];

fn parse_shape(name: &str) -> Option<SubjectShape> {
    // DEV NOTE: accepts BOTH snake_case (from `SubjectShape::as_str()`) AND
    // PascalCase (the canonical form used in `rules/subject_rules.toml`).
    // The original spec provided a snake_case-only matcher that did not agree
    // with the PascalCase keys in the TOML file. Broadening the matcher keeps
    // the TOML (DATA layer) authoritative and the test fixtures unchanged.
    Some(match name {
        "empty" | "Empty" => SubjectShape::Empty,
        "slug" | "Slug" => SubjectShape::Slug,
        "filename" | "Filename" => SubjectShape::Filename,
        "url" | "Url" => SubjectShape::Url,
        "date" | "Date" => SubjectShape::Date,
        "time_expr" | "TimeExpr" => SubjectShape::TimeExpr,
        "number_led" | "NumberLed" => SubjectShape::NumberLed,
        "currency_led" | "CurrencyLed" => SubjectShape::CurrencyLed,
        "sentence" | "Sentence" => SubjectShape::Sentence,
        "question" | "Question" => SubjectShape::Question,
        "thai_pure" | "ThaiPure" => SubjectShape::ThaiPure,
        "thai_latin_mixed" | "ThaiLatinMixed" => SubjectShape::ThaiLatinMixed,
        "ticker" | "Ticker" => SubjectShape::Ticker,
        "acronym" | "Acronym" => SubjectShape::Acronym,
        "title_case" | "TitleCase" => SubjectShape::TitleCase,
        "lowercase_noun" | "LowercaseNoun" => SubjectShape::LowercaseNoun,
        "verb_led" | "VerbLed" => SubjectShape::VerbLed,
        "demonstrative" | "Demonstrative" => SubjectShape::Demonstrative,
        "multi_entity" | "MultiEntity" => SubjectShape::MultiEntity,
        "possessive" | "Possessive" => SubjectShape::Possessive,
        "wiki_markup" | "WikiMarkup" => SubjectShape::WikiMarkup,
        "placeholder" | "Placeholder" => SubjectShape::Placeholder,
        "plain" | "Plain" => SubjectShape::Plain,
        "unknown" | "Unknown" => SubjectShape::Unknown,
        _ => return None,
    })
}

fn parse_verdict(s: &str) -> Option<SubjectVerdict> {
    Some(match s {
        "accept" => SubjectVerdict::Accept,
        "accept_info" => SubjectVerdict::AcceptWithInfo,
        "soft_flag" => SubjectVerdict::SoftFlag,
        "defer_to_llm" => SubjectVerdict::DeferToLLM,
        "reject_critical" => SubjectVerdict::Reject,
        _ => return None,
    })
}

#[derive(Debug)]
pub enum RulesError {
    TomlSyntax(toml::de::Error),
    UnknownShape(String),
    UnknownVerdict(String),
    MissingShape(String),
}

impl std::fmt::Display for RulesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TomlSyntax(e) => write!(f, "TOML syntax error: {e}"),
            Self::UnknownShape(s) => write!(f, "unknown shape in rules TOML: {s}"),
            Self::UnknownVerdict(s) => write!(f, "unknown verdict in rules TOML: {s}"),
            Self::MissingShape(s) => write!(f, "rules TOML is missing verdict for shape: {s}"),
        }
    }
}

impl std::error::Error for RulesError {}

// ── Layer 4: Allowlist / Denylist (TOML) ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct AllowlistFile {
    last_reviewed: String,
    version: String,
    #[serde(default)]
    user_overrides: std::collections::HashMap<String, String>,
    #[serde(default)]
    tickers: std::collections::HashMap<String, TickerGroup>,
    #[serde(default)]
    corporate_suffixes: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct TickerGroup {
    #[serde(default)]
    symbols: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SubjectAllowlist {
    pub last_reviewed: String,
    pub version: String,
    /// Subject strings that are always accepted (case-sensitive exact match).
    pub user_overrides: std::collections::HashSet<String>,
    /// Ticker symbols (uppercase) — always accepted as Ticker shape.
    pub tickers: std::collections::HashSet<String>,
    /// Canonical subjects from SemanticStore (inverted HashMap<Uuid,String>).
    pub canonical: std::collections::HashSet<String>,
    /// Corporate suffix tokens (e.g. "Inc", "Group", "PCL") — when present
    /// as the last token of a subject, the metric-phrase check is bypassed.
    pub corporate_suffixes: std::collections::HashSet<String>,
}

impl SubjectAllowlist {
    pub fn parse(
        toml_str: &str,
        canonical: std::collections::HashSet<String>,
    ) -> Result<Self, RulesError> {
        let file: AllowlistFile = toml::from_str(toml_str).map_err(RulesError::TomlSyntax)?;
        let user_overrides = file.user_overrides.keys().cloned().collect();
        let tickers = file
            .tickers
            .values()
            .flat_map(|g| g.symbols.iter().map(|s| s.to_uppercase()))
            .collect();
        let corporate_suffixes = file.corporate_suffixes.keys().cloned().collect();
        Ok(Self {
            last_reviewed: file.last_reviewed,
            version: file.version,
            user_overrides,
            tickers,
            canonical,
            corporate_suffixes,
        })
    }

    pub fn contains(&self, subject: &str) -> bool {
        self.user_overrides.contains(subject)
            || self.tickers.contains(&subject.to_uppercase())
            || self.canonical.contains(subject)
    }

    /// Returns true if the subject's last token is a corporate suffix
    /// (case-insensitive match against the configured set).
    pub fn has_corporate_suffix(&self, subject: &str) -> bool {
        let last = subject
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_ascii_lowercase();
        self.corporate_suffixes
            .iter()
            .any(|s| s.to_ascii_lowercase() == last)
    }
}

#[derive(Debug, Deserialize)]
struct DenylistFile {
    last_reviewed: String,
    version: String,
    #[serde(default)]
    headings: std::collections::HashMap<String, String>,
    #[serde(default)]
    stopwords: std::collections::HashMap<String, String>,
    #[serde(default)]
    llm_bleed: std::collections::HashMap<String, String>,
    #[serde(default)]
    ambiguous_acronyms: std::collections::HashMap<String, String>,
    #[serde(default)]
    metric_heads: std::collections::HashMap<String, String>,
    #[serde(default)]
    metric_single_words: std::collections::HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct SubjectDenylist {
    pub last_reviewed: String,
    pub version: String,
    pub headings: std::collections::HashSet<String>,
    pub stopwords: std::collections::HashSet<String>,
    pub llm_bleed: std::collections::HashSet<String>,
    pub ambiguous_acronyms: std::collections::HashSet<String>,
    pub metric_heads: std::collections::HashSet<String>,
    pub metric_single_words: std::collections::HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenylistCategory {
    Heading,
    Stopword,
    LlmBleed,
}

impl SubjectDenylist {
    pub fn parse(toml_str: &str) -> Result<Self, RulesError> {
        let file: DenylistFile = toml::from_str(toml_str).map_err(RulesError::TomlSyntax)?;
        Ok(Self {
            last_reviewed: file.last_reviewed,
            version: file.version,
            headings: file.headings.keys().cloned().collect(),
            stopwords: file.stopwords.keys().cloned().collect(),
            llm_bleed: file.llm_bleed.keys().cloned().collect(),
            ambiguous_acronyms: file.ambiguous_acronyms.keys().cloned().collect(),
            metric_heads: file.metric_heads.keys().cloned().collect(),
            metric_single_words: file.metric_single_words.keys().cloned().collect(),
        })
    }

    /// Returns Some(category) if the subject (exact match, case-sensitive
    /// for headings, case-insensitive for stopwords/llm_bleed) is denied.
    pub fn matches(&self, subject: &str) -> Option<DenylistCategory> {
        if self.headings.contains(subject) {
            return Some(DenylistCategory::Heading);
        }
        let lower = subject.to_ascii_lowercase();
        if self.stopwords.contains(&lower) {
            return Some(DenylistCategory::Stopword);
        }
        if self.llm_bleed.contains(subject) || self.llm_bleed.contains(&lower) {
            return Some(DenylistCategory::LlmBleed);
        }
        None
    }

    pub fn is_ambiguous_acronym(&self, subject: &str) -> bool {
        self.ambiguous_acronyms.contains(subject)
    }

    /// Returns true if the subject is a metric phrase (not an entity name).
    /// Two detection modes:
    ///   1. Subject IS exactly one of [metric_single_words] tokens
    ///      (case-insensitive). Examples: "Beta", "WACC", "NPV".
    ///   2. Subject's LAST whitespace token (after stripping trailing
    ///      parenthetical) lowercases to a member of [metric_heads].
    ///      Examples: "Risk-free rate" -> "rate", "Equity Value" -> "value".
    ///
    /// Caller MUST gate this by shape (only run on TitleCase / LowercaseNoun
    /// / Plain — never on Ticker / Acronym / Thai). Caller MUST also bypass
    /// when the subject's last token is a corporate suffix.
    pub fn is_metric_phrase(&self, subject: &str) -> bool {
        // Mode 1: exact single-token metric
        let lower_full = subject.trim().to_ascii_lowercase();
        if self.metric_single_words.contains(&lower_full) {
            return true;
        }
        // Mode 2: last-token head noun
        // Strip trailing parenthetical: "Cash (post-placement)" -> "Cash"
        let without_paren = subject.split('(').next().unwrap_or(subject).trim();
        let last_token = without_paren
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_ascii_lowercase();
        self.metric_heads.contains(&last_token)
    }
}

// ── SubjectValidator facade + Layer 5 combiner ──────────────────────────

use crate::quality::{QualitySeverity, QualityTagKind};
use std::path::Path;
use std::sync::Arc;

/// Top-level validator. Holds the parsed rules. Construct once at app boot.
pub struct SubjectValidator {
    pub rules: SubjectRules,
    pub allowlist: SubjectAllowlist,
    pub denylist: SubjectDenylist,
    /// Frequency counter for Unknown-shape inputs (Phase 3 trigger signal).
    unknown_counter: parking_lot::Mutex<std::collections::HashMap<String, u64>>,
}

impl SubjectValidator {
    /// Load all three rule files from a directory and merge canonical subjects.
    pub fn load(
        rules_dir: &Path,
        canonical: std::collections::HashMap<uuid::Uuid, String>,
    ) -> Result<Arc<Self>, ValidatorError> {
        let rules_str = std::fs::read_to_string(rules_dir.join("subject_rules.toml"))
            .map_err(|e| ValidatorError::FileMissing("subject_rules.toml".into(), e))?;
        let allow_str = std::fs::read_to_string(rules_dir.join("subject_allowlist.toml"))
            .map_err(|e| ValidatorError::FileMissing("subject_allowlist.toml".into(), e))?;
        let deny_str = std::fs::read_to_string(rules_dir.join("subject_denylist.toml"))
            .map_err(|e| ValidatorError::FileMissing("subject_denylist.toml".into(), e))?;
        Self::from_strings(&rules_str, &allow_str, &deny_str, canonical)
    }

    /// Like `load`, but falls back to embedded rules if files are missing.
    /// Used for tests and for boot resilience.
    pub fn load_with_embedded_fallback(
        rules_dir: &Path,
        canonical: std::collections::HashMap<uuid::Uuid, String>,
    ) -> Result<Arc<Self>, ValidatorError> {
        let rules_str = std::fs::read_to_string(rules_dir.join("subject_rules.toml"))
            .unwrap_or_else(|_| include_str!("../rules/subject_rules.toml").to_string());
        let allow_str = std::fs::read_to_string(rules_dir.join("subject_allowlist.toml"))
            .unwrap_or_else(|_| include_str!("../rules/subject_allowlist.toml").to_string());
        let deny_str = std::fs::read_to_string(rules_dir.join("subject_denylist.toml"))
            .unwrap_or_else(|_| include_str!("../rules/subject_denylist.toml").to_string());
        Self::from_strings(&rules_str, &allow_str, &deny_str, canonical)
    }

    /// Test-friendly constructor — pass TOML contents directly.
    pub fn from_strings(
        rules_str: &str,
        allow_str: &str,
        deny_str: &str,
        canonical: std::collections::HashMap<uuid::Uuid, String>,
    ) -> Result<Arc<Self>, ValidatorError> {
        let rules = SubjectRules::parse(rules_str).map_err(ValidatorError::Rules)?;
        let canonical_set: std::collections::HashSet<String> = canonical.into_values().collect();
        let allowlist =
            SubjectAllowlist::parse(allow_str, canonical_set).map_err(ValidatorError::Rules)?;
        let denylist = SubjectDenylist::parse(deny_str).map_err(ValidatorError::Rules)?;
        Ok(Arc::new(Self {
            rules,
            allowlist,
            denylist,
            unknown_counter: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }))
    }

    /// Validate one subject. Layer pipeline runs in order.
    pub fn validate(&self, raw_subject: &str) -> SubjectReport {
        // Layer 0: normalize
        let (normalized, _norm_fired) = normalize_subject(raw_subject);

        // Layer 1: mechanical hard-fail (short-circuits everything)
        if let Some(defect) = check_subject_mechanical(&normalized) {
            let tags = vec![mechanical_defect_tag(&defect, &normalized)];
            return SubjectReport {
                normalized,
                shape: SubjectShape::Empty,
                verdict: SubjectVerdict::Reject,
                quality_tags: tags,
                validator_version: SUBJECT_VALIDATOR_VERSION,
            };
        }

        // Layer 2: classify shape
        let shape = classify_shape(&normalized);

        // Layer 4 (denylist — runs before Layer 3 to allow heading override
        // on TitleCase shapes that would otherwise accept_info)
        if let Some(category) = self.denylist.matches(&normalized) {
            let tag = match category {
                DenylistCategory::Heading => QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!(
                        "subject `{}` matches a section heading (not an entity)",
                        normalized
                    ),
                )
                .with_evidence(normalized.clone()),
                DenylistCategory::Stopword => QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!("subject `{}` is a stopword", normalized),
                )
                .with_evidence(normalized.clone()),
                DenylistCategory::LlmBleed => QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!("subject `{}` is an LLM placeholder bleed", normalized),
                )
                .with_evidence(normalized.clone()),
            };
            return SubjectReport {
                normalized,
                shape,
                verdict: SubjectVerdict::Reject,
                quality_tags: vec![tag],
                validator_version: SUBJECT_VALIDATOR_VERSION,
            };
        }

        // Layer 4 (allowlist — escape hatch)
        if self.allowlist.contains(&normalized) {
            return SubjectReport {
                normalized,
                shape,
                verdict: SubjectVerdict::Accept,
                quality_tags: vec![],
                validator_version: SUBJECT_VALIDATOR_VERSION,
            };
        }

        // Layer 4b: metric-phrase check (gated by shape to avoid FP on legit entities)
        let metric_check_shapes = [
            SubjectShape::TitleCase,
            SubjectShape::LowercaseNoun,
            SubjectShape::Plain,
        ];
        if metric_check_shapes.contains(&shape)
            && !self.allowlist.has_corporate_suffix(&normalized)
            && self.denylist.is_metric_phrase(&normalized)
        {
            return SubjectReport {
                normalized: normalized.clone(),
                shape,
                verdict: SubjectVerdict::Reject,
                quality_tags: vec![QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!(
                        "subject `{}` is a metric/financial term (not an entity name) — restructure as entity+predicate",
                        normalized
                    ),
                )
                .with_evidence(normalized.clone())],
                validator_version: SUBJECT_VALIDATOR_VERSION,
            };
        }

        // Layer 3 + 5: shape verdict (with ambiguous-acronym boost)
        let shape_verdict = self.rules.verdict_for(shape);
        let mut tags = match shape_verdict {
            SubjectVerdict::Reject => vec![
                QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!(
                        "subject `{}` has shape `{:?}` which is not a valid entity name",
                        normalized, shape
                    ),
                )
                .with_evidence(normalized.clone()),
            ],
            SubjectVerdict::SoftFlag => vec![
                QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Warning,
                    soft_flag_message(shape, &normalized),
                )
                .with_evidence(normalized.clone()),
            ],
            SubjectVerdict::AcceptWithInfo => {
                let mut v = vec![];
                if shape == SubjectShape::Acronym && self.denylist.is_ambiguous_acronym(&normalized)
                {
                    v.push(
                        QualityTag::new(
                            QualityTagKind::SubjectAmbiguousAcronym,
                            QualitySeverity::Info,
                            format!(
                                "subject `{}` is an ambiguous acronym — verify intent",
                                normalized
                            ),
                        )
                        .with_evidence(normalized.clone()),
                    );
                }
                v
            }
            SubjectVerdict::DeferToLLM => vec![
                QualityTag::new(
                    QualityTagKind::SubjectNeedsContext,
                    QualitySeverity::Warning,
                    format!(
                        "subject `{}` could not be confidently classified",
                        normalized
                    ),
                )
                .with_evidence(normalized.clone()),
            ],
            SubjectVerdict::Accept => vec![],
        };

        // Unknown frequency counter (telemetry for Phase 3 trigger)
        if shape == SubjectShape::Unknown {
            let mut counter = self.unknown_counter.lock();
            *counter.entry(normalized.clone()).or_insert(0) += 1;
        }

        let _ = &mut tags; // (placeholder for future info-tag attachment)

        SubjectReport {
            normalized,
            shape,
            verdict: shape_verdict,
            quality_tags: tags,
            validator_version: SUBJECT_VALIDATOR_VERSION,
        }
    }

    /// Snapshot of Unknown-shape input frequencies (Phase 3 trigger metric).
    pub fn unknown_frequency_snapshot(&self) -> std::collections::HashMap<String, u64> {
        self.unknown_counter.lock().clone()
    }
}

fn mechanical_defect_tag(defect: &MechanicalDefect, subject: &str) -> QualityTag {
    let (kind, msg) = match defect {
        MechanicalDefect::Empty => (
            QualityTagKind::BadSubjectEmpty,
            "subject is empty".to_string(),
        ),
        MechanicalDefect::TooShort => (
            QualityTagKind::BadSubjectLength,
            "subject is too short (1 char)".to_string(),
        ),
        MechanicalDefect::TooLong => (
            QualityTagKind::BadSubjectLength,
            "subject is too long (>80 chars)".to_string(),
        ),
        MechanicalDefect::PunctuationOnly => (
            QualityTagKind::BadSubjectStructural,
            "subject is punctuation-only".to_string(),
        ),
        MechanicalDefect::ControlChars => (
            QualityTagKind::BadSubjectStructural,
            "subject contains control characters".to_string(),
        ),
        MechanicalDefect::TabNewlineCr => (
            QualityTagKind::BadSubjectStructural,
            "subject contains tab/newline/CR".to_string(),
        ),
        MechanicalDefect::WikiMarkupLeak => (
            QualityTagKind::BadSubjectStructural,
            "subject contains wiki markup".to_string(),
        ),
        MechanicalDefect::Emoji => (
            QualityTagKind::BadSubjectStructural,
            "subject contains emoji".to_string(),
        ),
        MechanicalDefect::HtmlInjection => (
            QualityTagKind::BadSubjectAdversarial,
            "subject contains HTML injection".to_string(),
        ),
        MechanicalDefect::TemplateInjection => (
            QualityTagKind::BadSubjectAdversarial,
            "subject contains template injection".to_string(),
        ),
        MechanicalDefect::RtlOverride => (
            QualityTagKind::BadSubjectAdversarial,
            "subject contains RTL override (U+202E)".to_string(),
        ),
    };
    QualityTag::new(kind, QualitySeverity::Critical, msg).with_evidence(subject.to_string())
}

fn soft_flag_message(shape: SubjectShape, subject: &str) -> String {
    match shape {
        SubjectShape::Possessive => format!(
            "subject `{}` is possessive — split into entity + predicate (e.g. \"Tesla's CFO\" → subject=\"Tesla\", predicate=\"CFO\")",
            subject
        ),
        SubjectShape::MultiEntity => format!(
            "subject `{}` contains multiple entities — split into N separate claims",
            subject
        ),
        _ => format!("subject `{}` needs review", subject),
    }
}

#[derive(Debug)]
pub enum ValidatorError {
    FileMissing(String, std::io::Error),
    Rules(RulesError),
}

impl std::fmt::Display for ValidatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileMissing(name, e) => write!(f, "could not read {name}: {e}"),
            Self::Rules(e) => write!(f, "rules error: {e}"),
        }
    }
}

impl std::error::Error for ValidatorError {}

#[cfg(test)]
mod tests_layer3 {
    use super::*;

    const TEST_RULES: &str = r#"
last_reviewed = "2026-07-21"
version = "test-v1"

[verdicts]
Empty = "reject_critical"
Slug = "reject_critical"
Filename = "reject_critical"
Url = "reject_critical"
Date = "reject_critical"
TimeExpr = "reject_critical"
NumberLed = "reject_critical"
CurrencyLed = "reject_critical"
Question = "reject_critical"
Sentence = "reject_critical"
VerbLed = "reject_critical"
Demonstrative = "reject_critical"
LowercaseNoun = "reject_critical"
WikiMarkup = "reject_critical"
Placeholder = "reject_critical"
Possessive = "soft_flag"
MultiEntity = "soft_flag"
ThaiPure = "accept_info"
ThaiLatinMixed = "accept_info"
Ticker = "accept"
Acronym = "accept_info"
TitleCase = "accept_info"
Plain = "accept_info"
Unknown = "accept_info"
"#;

    #[test]
    fn parses_all_shapes() {
        let rules = SubjectRules::parse(TEST_RULES).expect("parse");
        assert_eq!(
            rules.verdict_for(SubjectShape::Slug),
            SubjectVerdict::Reject
        );
        assert_eq!(
            rules.verdict_for(SubjectShape::Ticker),
            SubjectVerdict::Accept
        );
        assert_eq!(
            rules.verdict_for(SubjectShape::ThaiPure),
            SubjectVerdict::AcceptWithInfo
        );
    }

    #[test]
    fn missing_shape_fails() {
        let bad = TEST_RULES.replace("Unknown = \"accept_info\"\n", "");
        let err = SubjectRules::parse(&bad).unwrap_err();
        assert!(matches!(err, RulesError::MissingShape(_)));
    }

    #[test]
    fn unknown_verdict_fails() {
        let bad = TEST_RULES.replace("Plain = \"accept_info\"", "Plain = \"maybe\"");
        let err = SubjectRules::parse(&bad).unwrap_err();
        assert!(matches!(err, RulesError::UnknownVerdict(_)));
    }

    #[test]
    fn production_rules_toml_parses() {
        let toml_str = include_str!("../rules/subject_rules.toml");
        let rules = SubjectRules::parse(toml_str).expect("production rules must parse");
        assert!(!rules.last_reviewed.is_empty());
        assert!(!rules.version.is_empty());
    }
}

#[cfg(test)]
mod tests_layer4 {
    use super::*;
    use std::collections::HashSet;

    fn empty_canonical() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn allowlist_user_override_matches() {
        let toml_str = r#"
last_reviewed = "2026-07-21"
version = "v1"
[user_overrides]
"CATL" = "auto"
[tickers.NYSE]
symbols = ["BRK"]
"#;
        let al = SubjectAllowlist::parse(toml_str, empty_canonical()).unwrap();
        assert!(al.contains("CATL"));
        assert!(al.contains("BRK"));
        assert!(!al.contains("BYD"));
    }

    #[test]
    fn allowlist_canonical_subjects_merged() {
        let mut canon = HashSet::new();
        canon.insert("BYD".to_string());
        let toml_str = "last_reviewed=\"x\"\nversion=\"x\"\n";
        let al = SubjectAllowlist::parse(toml_str, canon).unwrap();
        assert!(al.contains("BYD"));
    }

    #[test]
    fn denylist_heading_match() {
        let toml_str = r#"
last_reviewed = "2026-07-21"
version = "v1"
[headings]
"DCF Assumptions" = "auto"
"#;
        let dl = SubjectDenylist::parse(toml_str).unwrap();
        assert_eq!(
            dl.matches("DCF Assumptions"),
            Some(DenylistCategory::Heading)
        );
        assert_eq!(dl.matches("CATL"), None);
    }

    #[test]
    fn denylist_stopword_case_insensitive() {
        let toml_str = r#"
last_reviewed = "x"
version = "x"
[stopwords]
"the" = "auto"
"#;
        let dl = SubjectDenylist::parse(toml_str).unwrap();
        assert_eq!(dl.matches("THE"), Some(DenylistCategory::Stopword));
    }

    #[test]
    fn denylist_llm_bleed_placeholder() {
        let toml_str = r#"
last_reviewed = "x"
version = "x"
[llm_bleed]
"<entity>" = "glm-4.6-v3"
"#;
        let dl = SubjectDenylist::parse(toml_str).unwrap();
        assert_eq!(dl.matches("<entity>"), Some(DenylistCategory::LlmBleed));
    }

    #[test]
    fn denylist_ambiguous_acronym() {
        let toml_str = r#"
last_reviewed = "x"
version = "x"
[ambiguous_acronyms]
"BAT" = "auto"
"#;
        let dl = SubjectDenylist::parse(toml_str).unwrap();
        assert!(dl.is_ambiguous_acronym("BAT"));
        assert!(!dl.is_ambiguous_acronym("CATL"));
    }

    #[test]
    fn production_allowlist_parses() {
        let s = include_str!("../rules/subject_allowlist.toml");
        SubjectAllowlist::parse(s, empty_canonical()).expect("prod allowlist");
    }

    #[test]
    fn production_denylist_parses() {
        let s = include_str!("../rules/subject_denylist.toml");
        SubjectDenylist::parse(s).expect("prod denylist");
    }
}

#[cfg(test)]
mod tests_facade {
    use super::*;
    use std::collections::HashMap;

    fn validator() -> Arc<SubjectValidator> {
        let rules = include_str!("../rules/subject_rules.toml");
        let allow = include_str!("../rules/subject_allowlist.toml");
        let deny = include_str!("../rules/subject_denylist.toml");
        SubjectValidator::from_strings(rules, allow, deny, HashMap::new())
            .expect("validator from production rules")
    }

    #[test]
    fn slug_subject_is_rejected_critical() {
        let v = validator();
        let r = v.validate("international-peers-deep");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
        assert!(
            r.quality_tags
                .iter()
                .any(|t| t.severity == QualitySeverity::Critical)
        );
    }

    #[test]
    fn lowercase_noun_subject_is_rejected_critical() {
        let v = validator();
        let r = v.validate("risk-free rate");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
        assert!(
            r.quality_tags
                .iter()
                .any(|t| t.kind == QualityTagKind::BadSubjectShape)
        );
    }

    #[test]
    fn acronym_silent_accept() {
        let v = validator();
        let r = v.validate("CATL");
        assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
        assert!(r.quality_tags.is_empty());
    }

    #[test]
    fn ambiguous_acronym_gets_info_tag() {
        let v = validator();
        let r = v.validate("BAT");
        assert!(
            r.quality_tags
                .iter()
                .any(|t| t.kind == QualityTagKind::SubjectAmbiguousAcronym)
        );
    }

    #[test]
    fn thai_pure_accept_info() {
        let v = validator();
        let r = v.validate("บมจ. ปตท.");
        assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
    }

    #[test]
    fn llm_bleed_placeholder_rejected() {
        let v = validator();
        let r = v.validate("<entity>");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn allowlist_user_override_forces_accept() {
        let rules = include_str!("../rules/subject_rules.toml");
        let allow = r#"
last_reviewed = "x"
version = "x"
[user_overrides]
"risk-free rate" = "auto"
"#;
        let deny = include_str!("../rules/subject_denylist.toml");
        let v = SubjectValidator::from_strings(rules, allow, deny, HashMap::new()).unwrap();
        let r = v.validate("risk-free rate");
        assert_eq!(r.verdict, SubjectVerdict::Accept);
    }

    #[test]
    fn heading_denylist_overrides_title_case() {
        let v = validator();
        let r = v.validate("DCF Assumptions");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn multi_entity_soft_flag() {
        let v = validator();
        let r = v.validate("CATL, BYD, LG");
        assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
    }

    #[test]
    fn possessive_soft_flag() {
        let v = validator();
        let r = v.validate("Tesla's CFO");
        assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
    }

    #[test]
    fn unknown_counter_increments_for_unknown_shape() {
        let v = validator();
        assert!(v.unknown_frequency_snapshot().is_empty());
    }

    // ── Phase 1.5.1: Metric Term Detection ─────────────────────────────────

    #[test]
    fn metric_phrase_rate_detected() {
        let v = validator();
        let r = v.validate("Risk-free rate");
        assert_eq!(r.verdict, SubjectVerdict::Reject, "got shape={:?}", r.shape);
    }

    #[test]
    fn metric_phrase_growth_detected() {
        let v = validator();
        let r = v.validate("Terminal growth");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn metric_phrase_value_detected() {
        let v = validator();
        let r = v.validate("Equity Value");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn metric_single_word_beta_detected() {
        let v = validator();
        let r = v.validate("Beta");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn metric_single_word_wacc_detected() {
        let v = validator();
        // NOTE: "WACC" (all-caps) would classify as Acronym and short-circuit
        // before the metric check — by design, to avoid FP on tickers like
        // CATL/BRK. The TitleCase form "Wacc" (the variant observed in LLM
        // output) is what this layer catches. See task Phase 1.5.1 goal list.
        let r = v.validate("Wacc");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn corporate_suffix_bypasses_metric_check() {
        let v = validator();
        // "Tesla Inc" — last token "Inc" is corporate suffix, bypasses metric check
        let r = v.validate("Tesla Inc");
        assert_ne!(
            r.verdict,
            SubjectVerdict::Reject,
            "Tesla Inc should not be rejected, got: {:?}",
            r.quality_tags
        );
    }

    #[test]
    fn corporate_group_bypasses_metric_check() {
        let v = validator();
        let r = v.validate("BYD Group");
        assert_ne!(r.verdict, SubjectVerdict::Reject);
    }

    #[test]
    fn real_entity_still_accepted() {
        let v = validator();
        let r = v.validate("CATL");
        assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
    }

    #[test]
    fn metric_phrase_cash_paren() {
        let v = validator();
        // "Cash (post-placement)" — paren stripped, "Cash" matches metric_heads
        let r = v.validate("Cash (post-placement)");
        // last token after paren strip is "Cash" — needs to be in metric_heads.
        // "cash" IS in metric_heads.
        assert_eq!(r.verdict, SubjectVerdict::Reject);
    }
}
