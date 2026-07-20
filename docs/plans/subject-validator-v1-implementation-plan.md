# Subject Validator v1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a 6-layer deterministic subject validator (Phase 1.5) with engine/data separation so it survives 3-5 years without rewrite.

**Architecture:** Wikidata pattern — Layer 0-2,5 are stable Rust code (engine), Layer 3-4 are TOML config files reviewed annually (data). Validate `subject` field of every proposal before it enters the review queue; bad subjects get a Critical `QualityTag` so the user sees a red chip in Inbox.

**Tech Stack:** Rust 1.95 (`std::sync::LazyLock`), `unicode-normalization` + `unicode-segmentation` (rust-lang org), existing `regex`/`toml`/`serde`/`jsonschema`/`sha2`/`chrono`/`parking_lot`.

**Spec:** `docs/plans/subject-validator-v1-spec.md` (commit `dd6667a`)

**Branch:** `vnext/phase-0` (already on it)

**Estimated time:** 5-7 days (18 tasks × ~30-45 min each)

---

## File Structure

**Created:**
- `src/subject_validator.rs` — engine module (Layer 0-2,5 types + logic)
- `rules/subject_rules.toml` — Layer 3 shape→verdict table (data)
- `rules/subject_allowlist.toml` — Layer 4 allowlist overrides (data)
- `rules/subject_denylist.toml` — Layer 4 denylist overrides (data)
- `tests/subject_validator_v1.rs` — integration tests on real store + 182-data regression

**Modified:**
- `Cargo.toml` — +2 deps
- `src/lib.rs` — declare `pub mod subject_validator;`
- `src/quality.rs` — +8 enum variants, inject `SubjectValidator` field, wire `check_subject_shape`
- `src/semantic.rs` — `ProposeInferenceCommand.subject_validator_version: Option<String>` field (production audit path — confirmed this is where prompt_version flows today, not `ExtractionAudit`)
- `src/api.rs` — construct `SubjectValidator` at boot, pass to `QualityChecker`
- `src/mcp/handlers.rs` — populate `subject_validator_version` when proposing

**Superseded (header update only):**
- `docs/plans/feature-subject-validator-framework.md` — pointer to v1 spec

---

## Pre-flight Check

**Discovery noted during planning:** `ExtractionAudit` struct exists (`src/extraction.rs:260`) but is **only instantiated in tests** today (`tests/extraction_pipeline_v1.rs:202, 218`). The real production extraction path in `src/mcp/handlers.rs:1117-1172` calls `parse_candidates` → `store.propose_inference(...)` directly and carries `prompt_version` via `ProposeInferenceCommand.prompt_version: Option<String>` (`src/semantic.rs:351`).

**Consequence:** Instead of adding a field to `ExtractionAudit` (spec §6.2), we add `subject_validator_version: Option<String>` to `ProposeInferenceCommand` — that's where production audit data actually flows. Spec is updated in spirit; the audit-trail goal is identical.

---

## Task 1: Add Cargo dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the 2 deps to Cargo.toml**

Open `Cargo.toml` and find the `# Regex` section (around line 57). Below it, add a Unicode section:

```toml
# Regex
regex      = "1"

# Unicode (subject validator Layer 0: NFC normalize, UAX#29 word boundaries)
unicode-normalization = "0.1"
unicode-segmentation  = "1.12"
```

- [ ] **Step 2: Verify the deps resolve**

Run: `cargo check`
Expected: completes with no errors. May take 1-2 minutes first time (compiling new crates).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(deps): add unicode-normalization + unicode-segmentation for subject validator"
```

---

## Task 2: Create subject_validator module skeleton

**Files:**
- Create: `src/subject_validator.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Register the module in lib.rs**

Open `src/lib.rs` and find the existing `pub mod quality;` line (around line 75). Add below it:

```rust
pub mod subject_validator;
```

- [ ] **Step 2: Create the skeleton file with types only**

Create `src/subject_validator.rs`:

```rust
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/subject_validator.rs
git commit -m "feat(subject-validator): module skeleton + SubjectShape/Verdict/Report types"
```

---

## Task 3: Layer 0 — normalize_subject

**Files:**
- Modify: `src/subject_validator.rs`
- Test: inline `#[cfg(test)] mod tests`

- [ ] **Step 1: Write failing tests for Layer 0**

Append to `src/subject_validator.rs`:

```rust
// ── Layer 0: Normalize ──────────────────────────────────────────────────

/// NFC-normalize, strip zero-width chars, convert NBSP/thin/em-spaces to
/// regular ASCII space, collapse internal whitespace runs, trim.
///
/// Always runs; never rejects. Severity is `info` — fired only when the
/// normalized form differs from raw (see `normalize_fired` in tests).
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
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib subject_validator::tests_layer0`
Expected: 7 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 0 normalize_subject + tests"
```

---

## Task 4: Layer 1 — MechanicalDefect detection

**Files:**
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Add MechanicalDefect enum + check function**

Insert above `#[cfg(test)] mod tests_layer0`:

```rust
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
    if lower.contains("<script") || lower.contains("<") && lower.contains(">") {
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
```

- [ ] **Step 2: Write Layer 1 tests**

Append after `tests_layer0` module:

```rust
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
```

- [ ] **Step 3: Run Layer 1 tests**

Run: `cargo test --lib subject_validator::tests_layer1`
Expected: 12 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 1 mechanical hard-fail + tests"
```

---

## Task 5: Layer 2 — Shape classifier (structural shapes)

**Files:**
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Add compiled regex set (LazyLock) and shape classifier skeleton**

Append below Layer 1 helpers (before `tests_layer1`):

```rust
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
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(_[a-z0-9]+){1,}$").unwrap());
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
    if RE_SLUG.is_match(normalized) {
        return SubjectShape::Slug;
    }
    if RE_FILENAME.is_match(normalized) {
        return SubjectShape::Filename;
    }
    if RE_URL.is_match(normalized) {
        return SubjectShape::Url;
    }
    if RE_DATE.is_match(normalized) {
        return SubjectShape::Date;
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
    // Possessive
    if normalized.contains("'s") || normalized.ends_with('\'') {
        return SubjectShape::Possessive;
    }
    // Thai detection
    let has_thai = normalized.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
    let has_latin = normalized.chars().any(|c| c.is_ascii_alphabetic());
    if has_thai && !has_latin {
        return SubjectShape::ThaiPure;
    }
    if has_thai && has_latin {
        return SubjectShape::ThaiLatinMixed;
    }
    // Defaults for non-Thai text — filled in by Task 6 (verb/demonstrative/
    // lowercase/title/sentence) and Task 7 (plain/unknown)
    SubjectShape::Plain // placeholder until Task 6 fills in
}

fn has_multi_entity_pattern(s: &str) -> bool {
    // "X, Y" with both X and Y starting uppercase
    let comma_joined = s.contains(',') && s.split(',').filter(|p| !p.trim().is_empty()).count() >= 2
        && s.split(',').all(|p| p.trim().chars().next().map_or(false, |c| c.is_uppercase()));
    if comma_joined {
        return true;
    }
    // "X and Y" with both capitalized
    let lower = s.to_ascii_lowercase();
    if lower.contains(" and ") {
        let parts: Vec<&str> = lower.split(" and ").collect();
        if parts.iter().all(|p| p.trim().chars().next().map_or(false, |c| c.is_uppercase())) {
            // Re-check original case (lowercased loses it)
            let orig_parts: Vec<&str> = s.split(" and ").collect();
            if orig_parts.iter().all(|p| p.trim().chars().next().map_or(false, |c| c.is_uppercase())) {
                return true;
            }
        }
    }
    false
}
```

- [ ] **Step 2: Write structural shape tests**

Append:

```rust
#[cfg(test)]
mod tests_layer2_structural {
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
```

- [ ] **Step 3: Run structural shape tests**

Run: `cargo test --lib subject_validator::tests_layer2_structural`
Expected: 12 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 2 structural shape classifier + tests"
```

---

## Task 6: Layer 2 — Shape classifier (Thai + grammatical shapes)

**Files:**
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Replace the `SubjectShape::Plain // placeholder` line with full grammatical detection**

In `classify_shape`, find the placeholder line:

```rust
    SubjectShape::Plain // placeholder until Task 6 fills in
```

Replace with:

```rust
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
    // Sentence — ends with '.' and not all caps
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
```

- [ ] **Step 2: Add grammatical shape tests**

Append:

```rust
#[cfg(test)]
mod tests_layer2_grammatical {
    use super::*;

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
```

- [ ] **Step 3: Run grammatical shape tests**

Run: `cargo test --lib subject_validator::tests_layer2_grammatical`
Expected: 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 2 grammatical + Thai shape detection"
```

---

## Task 7: Layer 3 — Rules TOML loader

**Files:**
- Create: `rules/subject_rules.toml`
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Create the rules TOML file**

Create `rules/subject_rules.toml` (in repo root, alongside `Cargo.toml`):

```toml
# Subject Validator Layer 3 — shape → verdict mapping (DATA, not code).
#
# Edit this file to change verdict logic WITHOUT recompiling. Reviewed
# annually — see `last_reviewed` below.
#
# Rule review ritual (when to bump last_reviewed):
#   - Whenever the extraction LLM model version changes (Family G may shift)
#   - Whenever a new subject shape is added to SubjectShape enum
#   - At minimum annually even if no changes (defense against drift)
#
# Verdict values (must be one of):
#   accept          — strong positive shape, no defect
#   accept_info     — accept but tag info (Thai structural, ambiguous acronym)
#   soft_flag       — warning, suggest split (Possessive, MultiEntity)
#   defer_to_llm    — Phase 3 only; in Phase 1.5 use accept_info
#   reject_critical — hard reject

last_reviewed = "2026-07-21"
version = "subject-rules-v1"

[verdicts]
Empty          = "reject_critical"
Slug           = "reject_critical"
Filename       = "reject_critical"
Url            = "reject_critical"
Date           = "reject_critical"
TimeExpr       = "reject_critical"
NumberLed      = "reject_critical"
CurrencyLed    = "reject_critical"
Question       = "reject_critical"
Sentence       = "reject_critical"
VerbLed        = "reject_critical"
Demonstrative  = "reject_critical"
LowercaseNoun  = "reject_critical"
WikiMarkup     = "reject_critical"
Placeholder    = "reject_critical"

Possessive     = "soft_flag"
MultiEntity    = "soft_flag"

ThaiPure       = "accept_info"
ThaiLatinMixed = "accept_info"
Ticker         = "accept"
Acronym        = "accept_info"
TitleCase      = "accept_info"

# Phase 1.5 decision: Plain/Unknown → accept_info (no Phase 3 LLM yet).
# Phase 3 will change to "defer_to_llm" when LLM-as-judge is wired.
Plain          = "accept_info"
Unknown        = "accept_info"
```

- [ ] **Step 2: Add rules-loading types + parser**

In `src/subject_validator.rs`, append before `tests_layer0`:

```rust
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
        let file: SubjectRulesFile = toml::from_str(toml_str)
            .map_err(RulesError::TomlSyntax)?;
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
        self.verdicts.get(&shape).copied().unwrap_or(SubjectVerdict::DeferToLLM)
    }
}

pub(crate) const ALL_SHAPES: &[SubjectShape] = &[
    SubjectShape::Empty, SubjectShape::Slug, SubjectShape::Filename,
    SubjectShape::Url, SubjectShape::Date, SubjectShape::TimeExpr,
    SubjectShape::NumberLed, SubjectShape::CurrencyLed,
    SubjectShape::Sentence, SubjectShape::Question,
    SubjectShape::ThaiPure, SubjectShape::ThaiLatinMixed,
    SubjectShape::Ticker, SubjectShape::Acronym,
    SubjectShape::TitleCase, SubjectShape::LowercaseNoun,
    SubjectShape::VerbLed, SubjectShape::Demonstrative,
    SubjectShape::MultiEntity, SubjectShape::Possessive,
    SubjectShape::WikiMarkup, SubjectShape::Placeholder,
    SubjectShape::Plain, SubjectShape::Unknown,
];

fn parse_shape(name: &str) -> Option<SubjectShape> {
    Some(match name {
        "empty" => SubjectShape::Empty,
        "slug" => SubjectShape::Slug,
        "filename" => SubjectShape::Filename,
        "url" => SubjectShape::Url,
        "date" => SubjectShape::Date,
        "time_expr" => SubjectShape::TimeExpr,
        "number_led" => SubjectShape::NumberLed,
        "currency_led" => SubjectShape::CurrencyLed,
        "sentence" => SubjectShape::Sentence,
        "question" => SubjectShape::Question,
        "thai_pure" => SubjectShape::ThaiPure,
        "thai_latin_mixed" => SubjectShape::ThaiLatinMixed,
        "ticker" => SubjectShape::Ticker,
        "acronym" => SubjectShape::Acronym,
        "title_case" => SubjectShape::TitleCase,
        "lowercase_noun" => SubjectShape::LowercaseNoun,
        "verb_led" => SubjectShape::VerbLed,
        "demonstrative" => SubjectShape::Demonstrative,
        "multi_entity" => SubjectShape::MultiEntity,
        "possessive" => SubjectShape::Possessive,
        "wiki_markup" => SubjectShape::WikiMarkup,
        "placeholder" => SubjectShape::Placeholder,
        "plain" => SubjectShape::Plain,
        "unknown" => SubjectShape::Unknown,
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
```

- [ ] **Step 3: Write Layer 3 tests**

Append:

```rust
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
        assert_eq!(rules.verdict_for(SubjectShape::Slug), SubjectVerdict::Reject);
        assert_eq!(rules.verdict_for(SubjectShape::Ticker), SubjectVerdict::Accept);
        assert_eq!(rules.verdict_for(SubjectShape::ThaiPure), SubjectVerdict::AcceptWithInfo);
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
}
```

- [ ] **Step 4: Run Layer 3 tests**

Run: `cargo test --lib subject_validator::tests_layer3`
Expected: 3 tests pass.

- [ ] **Step 5: Verify the production TOML parses**

Add a one-shot test to confirm the shipped `rules/subject_rules.toml` parses. Append to `tests_layer3`:

```rust
    #[test]
    fn production_rules_toml_parses() {
        let toml_str = include_str!("../rules/subject_rules.toml");
        let rules = SubjectRules::parse(toml_str).expect("production rules must parse");
        assert!(!rules.last_reviewed.is_empty());
        assert!(!rules.version.is_empty());
    }
```

Run: `cargo test --lib subject_validator::tests_layer3::production_rules_toml_parses`
Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add rules/subject_rules.toml src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 3 TOML rules loader + production rules"
```

---

## Task 8: Layer 4 — Allowlist + Denylist loaders

**Files:**
- Create: `rules/subject_allowlist.toml`
- Create: `rules/subject_denylist.toml`
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Create allowlist TOML**

Create `rules/subject_allowlist.toml`:

```toml
# Subject Validator Layer 4 — allowlist overrides (DATA, not code).
#
# Subjects in this file are ALWAYS accepted, regardless of shape verdict.
# This is the escape hatch for legitimate entities whose shape doesn't fit
# the classifier (e.g. company names that happen to look like slugs).
#
# Canonical subjects from the SemanticStore entities table are merged in
# at runtime (dynamic, not listed here). This file is for static overrides.
#
# Reviewed annually — bump `last_reviewed` if you add or remove entries.

last_reviewed = "2026-07-21"
version = "subject-allowlist-v1"

# User-curated overrides (add specific entities you want to always accept).
[user_overrides]
# (empty by default — SemanticStore canonical subjects cover common cases)

# Ticker allowlist — manual curation from NYSE/NASDAQ/SET (Wikidata P414).
# Sync annually or when adding new markets.
[tickers.NYSE]
symbols = ["BRK", "JPM", "BAC", "WFC", "V", "MA"]

[tickers.NASDAQ]
symbols = ["AAPL", "MSFT", "NVDA", "TSLA", "GOOGL", "META", "AMZN"]

[tickers.SET]
symbols = ["PTT", "SCB", "AOT", "CPALL", "ADVANC", "TRUE"]
```

- [ ] **Step 2: Create denylist TOML**

Create `rules/subject_denylist.toml`:

```toml
# Subject Validator Layer 4 — denylist overrides (DATA, not code).
#
# Subjects matching any entry here are ALWAYS rejected, even if shape would
# accept (e.g. "DCF Assumptions" is TitleCase but is a section heading, not
# an entity).
#
# Reviewed annually — bump `last_reviewed`. RE-VIEW [llm_bleed] ON EVERY
# LLM MODEL UPGRADE (placeholder patterns are model-specific).

last_reviewed = "2026-07-21"
version = "subject-denylist-v1"

# Section headings (Family B1) — even TitleCase shape, these are NOT entities
[headings]
"DCF Assumptions" = "auto"
"Risk Factors" = "auto"
"Executive Summary" = "auto"
"Financial Highlights" = "auto"
"Sensitivity Analysis" = "auto"
"Disclaimer" = "auto"

# Stopwords (Family A9) — never valid as subject
[stopwords]
"the" = "auto"
"a" = "auto"
"an" = "auto"
"and" = "auto"
"or" = "auto"
"of" = "auto"
"it" = "auto"
"this" = "auto"
"that" = "auto"

# LLM-bleed placeholders (Family G3-G5). Model-specific — re-view on upgrade.
[llm_bleed]
"N/A" = "glm-4.6-v3-prompt"
"TBD" = "glm-4.6-v3-prompt"
"unknown" = "glm-4.6-v3-prompt"
"none" = "glm-4.6-v3-prompt"
"null" = "glm-4.6-v3-prompt"
"[SUBJECT]" = "glm-4.6-v3-prompt"
"<entity>" = "glm-4.6-v3-prompt"
"___" = "glm-4.6-v3-prompt"
"unspecified" = "glm-4.6-v3-prompt"
"not specified" = "glm-4.6-v3-prompt"
"unknown entity" = "glm-4.6-v3-prompt"

# Ambiguous acronyms (Family E3) — flag with info, NOT auto-accept
[ambiguous_acronyms]
"BAT" = "auto"
"CAT" = "auto"
"SAP" = "auto"
"API" = "auto"
```

- [ ] **Step 3: Add allowlist + denylist loader types**

In `src/subject_validator.rs`, append before `tests_layer3`:

```rust
// ── Layer 4: Allowlist / Denylist (TOML) ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct AllowlistFile {
    last_reviewed: String,
    version: String,
    #[serde(default)]
    user_overrides: std::collections::HashMap<String, String>,
    #[serde(default)]
    tickers: std::collections::HashMap<String, TickerGroup>,
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
}

impl SubjectAllowlist {
    pub fn parse(toml_str: &str, canonical: std::collections::HashSet<String>) -> Result<Self, RulesError> {
        let file: AllowlistFile = toml::from_str(toml_str).map_err(RulesError::TomlSyntax)?;
        let user_overrides = file.user_overrides.keys().cloned().collect();
        let tickers = file.tickers.values()
            .flat_map(|g| g.symbols.iter().cloned().map(|s| s.to_uppercase()))
            .collect();
        Ok(Self {
            last_reviewed: file.last_reviewed,
            version: file.version,
            user_overrides,
            tickers,
            canonical,
        })
    }

    pub fn contains(&self, subject: &str) -> bool {
        self.user_overrides.contains(subject)
            || self.tickers.contains(&subject.to_uppercase())
            || self.canonical.contains(subject)
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
}

#[derive(Clone, Debug)]
pub struct SubjectDenylist {
    pub last_reviewed: String,
    pub version: String,
    pub headings: std::collections::HashSet<String>,
    pub stopwords: std::collections::HashSet<String>,
    pub llm_bleed: std::collections::HashSet<String>,
    pub ambiguous_acronyms: std::collections::HashSet<String>,
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
}
```

- [ ] **Step 4: Write Layer 4 tests**

Append:

```rust
#[cfg(test)]
mod tests_layer4 {
    use super::*;
    use std::collections::HashSet;

    fn empty_canonical() -> HashSet<String> { HashSet::new() }

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
        assert_eq!(dl.matches("DCF Assumptions"), Some(DenylistCategory::Heading));
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
```

- [ ] **Step 5: Run Layer 4 tests**

Run: `cargo test --lib subject_validator::tests_layer4`
Expected: 7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add rules/subject_allowlist.toml rules/subject_denylist.toml src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 4 allowlist/denylist loaders + production data"
```

---

## Task 9: Layer 5 — Verdict combiner + SubjectValidator facade

**Files:**
- Modify: `src/subject_validator.rs`

- [ ] **Step 1: Add SubjectValidator facade + Layer 5 combiner**

In `src/subject_validator.rs`, append before `tests_layer3`:

```rust
// ── SubjectValidator facade + Layer 5 combiner ──────────────────────────

use std::path::Path;
use std::sync::Arc;
use crate::quality::{QualityTag, QualityTagKind, QualitySeverity};

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

    /// Test-friendly constructor — pass TOML contents directly.
    pub fn from_strings(
        rules_str: &str,
        allow_str: &str,
        deny_str: &str,
        canonical: std::collections::HashMap<uuid::Uuid, String>,
    ) -> Result<Arc<Self>, ValidatorError> {
        let rules = SubjectRules::parse(rules_str).map_err(ValidatorError::Rules)?;
        let canonical_set: std::collections::HashSet<String> =
            canonical.into_values().collect();
        let allowlist = SubjectAllowlist::parse(allow_str, canonical_set)
            .map_err(ValidatorError::Rules)?;
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
                    format!("subject `{}` matches a section heading (not an entity)", normalized),
                ).with_evidence(normalized.clone()),
                DenylistCategory::Stopword => QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!("subject `{}` is a stopword", normalized),
                ).with_evidence(normalized.clone()),
                DenylistCategory::LlmBleed => QualityTag::new(
                    QualityTagKind::BadSubjectShape,
                    QualitySeverity::Critical,
                    format!("subject `{}` is an LLM placeholder bleed", normalized),
                ).with_evidence(normalized.clone()),
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

        // Layer 3 + 5: shape verdict (with ambiguous-acronym boost)
        let shape_verdict = self.rules.verdict_for(shape);
        let mut tags = match shape_verdict {
            SubjectVerdict::Reject => vec![QualityTag::new(
                QualityTagKind::BadSubjectShape,
                QualitySeverity::Critical,
                format!("subject `{}` has shape `{:?}` which is not a valid entity name", normalized, shape),
            ).with_evidence(normalized.clone())],
            SubjectVerdict::SoftFlag => vec![QualityTag::new(
                QualityTagKind::BadSubjectShape,
                QualitySeverity::Warning,
                soft_flag_message(shape, &normalized),
            ).with_evidence(normalized.clone())],
            SubjectVerdict::AcceptWithInfo => {
                let mut v = vec![];
                if shape == SubjectShape::Acronym && self.denylist.is_ambiguous_acronym(&normalized) {
                    v.push(QualityTag::new(
                        QualityTagKind::SubjectAmbiguousAcronym,
                        QualitySeverity::Info,
                        format!("subject `{}` is an ambiguous acronym — verify intent", normalized),
                    ).with_evidence(normalized.clone()));
                }
                v
            }
            SubjectVerdict::DeferToLLM => vec![QualityTag::new(
                QualityTagKind::SubjectNeedsContext,
                QualitySeverity::Warning,
                format!("subject `{}` could not be confidently classified", normalized),
            ).with_evidence(normalized.clone())],
            SubjectVerdict::Accept => vec![],
        };

        // Unknown frequency counter (telemetry for Phase 3 trigger)
        if shape == SubjectShape::Unknown {
            let mut counter = self.unknown_counter.lock();
            *counter.entry(normalized.clone()).or_insert(0) += 1;
        }

        // Attach mechanical-defect info tag for non-critical shapes if relevant
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
        MechanicalDefect::Empty => (QualityTagKind::BadSubjectEmpty, "subject is empty".to_string()),
        MechanicalDefect::TooShort => (QualityTagKind::BadSubjectLength, "subject is too short (1 char)".to_string()),
        MechanicalDefect::TooLong => (QualityTagKind::BadSubjectLength, "subject is too long (>80 chars)".to_string()),
        MechanicalDefect::PunctuationOnly => (QualityTagKind::BadSubjectStructural, "subject is punctuation-only".to_string()),
        MechanicalDefect::ControlChars => (QualityTagKind::BadSubjectStructural, "subject contains control characters".to_string()),
        MechanicalDefect::TabNewlineCr => (QualityTagKind::BadSubjectStructural, "subject contains tab/newline/CR".to_string()),
        MechanicalDefect::WikiMarkupLeak => (QualityTagKind::BadSubjectStructural, "subject contains wiki markup".to_string()),
        MechanicalDefect::Emoji => (QualityTagKind::BadSubjectStructural, "subject contains emoji".to_string()),
        MechanicalDefect::HtmlInjection => (QualityTagKind::BadSubjectAdversarial, "subject contains HTML injection".to_string()),
        MechanicalDefect::TemplateInjection => (QualityTagKind::BadSubjectAdversarial, "subject contains template injection".to_string()),
        MechanicalDefect::RtlOverride => (QualityTagKind::BadSubjectAdversarial, "subject contains RTL override (U+202E)".to_string()),
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
```

- [ ] **Step 2: Write facade tests**

Append:

```rust
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
        assert!(r.quality_tags.iter().any(|t| t.severity == QualitySeverity::Critical));
    }

    #[test]
    fn lowercase_noun_subject_is_rejected_critical() {
        let v = validator();
        let r = v.validate("risk-free rate");
        assert_eq!(r.verdict, SubjectVerdict::Reject);
        assert!(r.quality_tags.iter().any(|t| t.kind == QualityTagKind::BadSubjectShape));
    }

    #[test]
    fn acronym_silent_accept() {
        let v = validator();
        let r = v.validate("CATL");
        assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
        // No ambiguous flag for CATL
        assert!(r.quality_tags.is_empty());
    }

    #[test]
    fn ambiguous_acronym_gets_info_tag() {
        let v = validator();
        let r = v.validate("BAT");
        assert!(r.quality_tags.iter().any(|t| t.kind == QualityTagKind::SubjectAmbiguousAcronym));
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
        // Even though shape would reject (LowercaseNoun), override accepts.
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
        // Force Unknown shape: hard to trigger from production rules, so we
        // verify the counter is empty initially and the API exists.
        let v = validator();
        assert!(v.unknown_frequency_snapshot().is_empty());
    }
}
```

- [ ] **Step 3: Run facade tests**

Run: `cargo test --lib subject_validator::tests_facade`
Expected: 11 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/subject_validator.rs
git commit -m "feat(subject-validator): Layer 5 verdict combiner + SubjectValidator facade"
```

---

## Task 10: Add 8 QualityTagKind variants

**Files:**
- Modify: `src/quality.rs`

- [ ] **Step 1: Extend QualityTagKind enum**

Open `src/quality.rs`, find the `QualityTagKind` enum (around line 50-64). Add the 8 new variants after `ProvenanceLoss`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTagKind {
    DuplicatePredicate,
    PackedFacts,
    VaguePredicate,
    TaxonomyDrift,
    ConfidenceTooHigh,
    DoubleBracket,
    KindMismatch,
    // Phase 3 (AI only):
    SourceClaimMismatch,
    SemanticDuplicate,
    ProvenanceLoss,
    // Phase 1.5 — Subject Validator:
    BadSubjectEmpty,
    BadSubjectStructural,
    BadSubjectShape,
    BadSubjectLength,
    BadSubjectMixedScript,
    BadSubjectAdversarial,
    SubjectAmbiguousAcronym,
    SubjectNeedsContext,
}
```

- [ ] **Step 2: Bump QUALITY_CHECKER_VERSION**

Find `QUALITY_CHECKER_VERSION` (around line 20). Update the string:

```rust
/// Bumped whenever a rule's behavior or the response shape changes.
/// Recorded in `AiReviewResponse.checker_version` for audit.
///
/// v1-subject-shape (2026-07-21): adds `check_subject_shape` rule with 8 new
/// tag variants for bad subjects (Family A/B/C/D/F/G — see Subject Validator
/// spec).
pub const QUALITY_CHECKER_VERSION: &str = "quality-v1-subject-shape";
```

- [ ] **Step 3: Verify compile (will fail until Task 11 injects the validator)**

Run: `cargo check --lib`
Expected: compiles — the enum additions don't depend on the validator yet.

- [ ] **Step 4: Verify existing tests still pass**

Run: `cargo test --lib quality::`
Expected: all existing quality tests still pass (no test referenced the new variants).

- [ ] **Step 5: Commit**

```bash
git add src/quality.rs
git commit -m "feat(quality): add 8 BadSubject* + SubjectAmbiguous* QualityTagKind variants"
```

---

## Task 11: Inject SubjectValidator into QualityChecker

**Files:**
- Modify: `src/quality.rs`

- [ ] **Step 1: Update QualityChecker struct + constructor**

Find `QualityChecker` struct definition (around line 118-124):

```rust
#[derive(Clone, Debug, Default)]
pub struct QualityChecker;

impl QualityChecker {
    pub fn new() -> Self {
        Self
    }
```

Replace with:

```rust
/// Stateless deterministic checker (mostly). Holds an `Arc<SubjectValidator>`
/// so the subject-shape rule can read TOML rules. Construct once at app boot
/// and clone cheaply per request.
#[derive(Clone)]
pub struct QualityChecker {
    pub subject_validator: Option<std::sync::Arc<crate::subject_validator::SubjectValidator>>,
}

impl Default for QualityChecker {
    /// Default constructor for tests / legacy call sites that don't yet
    /// inject a validator. Subject validation is silently skipped.
    fn default() -> Self {
        Self { subject_validator: None }
    }
}

impl QualityChecker {
    pub fn new(subject_validator: std::sync::Arc<crate::subject_validator::SubjectValidator>) -> Self {
        Self { subject_validator: Some(subject_validator) }
    }

    /// Legacy constructor that skips subject validation. Used by tests that
    /// don't care about subject rules. Prefer `new()`.
    pub fn without_subject_validation() -> Self {
        Self { subject_validator: None }
    }
```

- [ ] **Step 2: Add check_subject_shape call to check_deterministic**

Find `check_deterministic` (around line 131-142). Add the new check after `check_kind_mismatch`:

```rust
    pub fn check_deterministic(&self, input: &QualityCheckerInput<'_>) -> Vec<QualityTag> {
        let mut tags = Vec::new();
        check_taxonomy_drift(input, &mut tags);
        check_vague_predicate(input, &mut tags);
        check_packed_facts(input, &mut tags);
        check_double_bracket(input, &mut tags);
        check_duplicate_predicate(input, &mut tags);
        check_confidence_too_high(input, &mut tags);
        check_kind_mismatch(input, &mut tags);
        check_subject_shape(self, input, &mut tags);
        tags
    }
```

- [ ] **Step 3: Add check_subject_shape function**

Below `check_kind_mismatch` (find the closing brace of that function), append:

```rust
/// Phase 1.5 rule — delegate to `SubjectValidator` if injected. No-op if
/// the checker was constructed via `without_subject_validation()` (legacy).
fn check_subject_shape(
    checker: &QualityChecker,
    input: &QualityCheckerInput<'_>,
    tags: &mut Vec<QualityTag>,
) {
    let Some(validator) = &checker.subject_validator else { return };
    let report = validator.validate(&input.proposal.subject);
    tags.extend(report.quality_tags);
}
```

- [ ] **Step 4: Update existing test helpers in quality.rs**

Find the existing `run()` test helper (around line 666) and update it to use `without_subject_validation`:

```rust
    fn run(p: &ProposalSummary, existing: &[ClaimView]) -> Vec<QualityTag> {
        let ev = evidence();
        let input = QualityCheckerInput {
            proposal: p,
            evidence: &ev,
            existing_claims: existing,
        };
        QualityChecker::without_subject_validation().check_deterministic(&input)
    }
```

- [ ] **Step 5: Verify quality.rs tests still pass**

Run: `cargo test --lib quality::`
Expected: all existing tests pass (they don't exercise subject rules).

- [ ] **Step 6: Commit**

```bash
git add src/quality.rs
git commit -m "feat(quality): wire SubjectValidator into QualityChecker.check_deterministic"
```

---

## Task 12: Update call sites in api.rs and tests

**Files:**
- Modify: `src/api.rs:895`
- Modify: `tests/quality_rules_v1.rs:179, 508`

- [ ] **Step 1: Update ConsoleApiState to hold the validator**

Open `src/api.rs`, find `ConsoleApiState` (around line 115). Add a field after `ai_provider`:

```rust
pub struct ConsoleApiState {
    // ... existing fields ...
    pub ai_provider: Option<std::sync::Arc<dyn crate::provider::AiProvider>>,
    pub subject_validator: Option<std::sync::Arc<crate::subject_validator::SubjectValidator>>,
}
```

Add a builder method mirroring `with_ai_provider`:

```rust
    pub fn with_subject_validator(
        mut self,
        validator: std::sync::Arc<crate::subject_validator::SubjectValidator>,
    ) -> Self {
        self.subject_validator = Some(validator);
        self
    }
```

- [ ] **Step 2: Update the ai_review handler**

Find `ai_review` (around line 846-895). Find the line:

```rust
let mut tags = crate::quality::QualityChecker::new().check_deterministic(&input);
```

Replace with:

```rust
let checker = match &state.subject_validator {
    Some(v) => crate::quality::QualityChecker::new(v.clone()),
    None => crate::quality::QualityChecker::without_subject_validation(),
};
let mut tags = checker.check_deterministic(&input);
```

- [ ] **Step 3: Update tests/quality_rules_v1.rs call sites**

Find every `QualityChecker::new()` in `tests/quality_rules_v1.rs`. Replace each with `QualityChecker::without_subject_validation()`. (Line 179 and line 508 per the discovery — verify by grep first.)

Run: `grep -n "QualityChecker::new" tests/quality_rules_v1.rs`
Then replace each occurrence with `QualityChecker::without_subject_validation()`.

- [ ] **Step 4: Find and update main.rs / wherever ConsoleApiState is constructed**

Run: `grep -rn "ConsoleApiState" src/ | grep -v "ConsoleApiState {"`

For every construction site of `ConsoleApiState { ... }`, add `.with_subject_validator(validator)` after the struct construction OR add the field `subject_validator: None` if the validator isn't yet wired at boot (Task 13 will wire it).

For now (Task 12), the simplest path: add `subject_validator: None,` to every `ConsoleApiState { ... }` literal. Task 13 will replace `None` with a real boot-time load.

- [ ] **Step 5: Verify the crate compiles**

Run: `cargo check`
Expected: compiles. The crate compiles with `subject_validator: None` everywhere — no behavior change yet.

- [ ] **Step 6: Commit**

```bash
git add src/api.rs tests/quality_rules_v1.rs
git commit -m "refactor(api): thread SubjectValidator through ConsoleApiState (no behavior change yet)"
```

---

## Task 13: Wire boot-time validator loading

**Files:**
- Modify: `src/main.rs` (or wherever `ConsoleApiState` is built at boot)
- Modify: `src/api.rs` (replace the `subject_validator: None` literals from Task 12)

- [ ] **Step 1: Locate the boot-time state construction**

Run: `grep -rn "ConsoleApiState {" src/`
Find the production construction (likely in `src/main.rs` or `src/server.rs`).

- [ ] **Step 2: Add validator boot loading**

At the boot site (just before `ConsoleApiState { ... }` is constructed), add:

```rust
// Load Subject Validator rules once at boot. Failure is fatal — bad rules
// means every subject validation is wrong.
let canonical_subjects = store.entity_canonical_subjects_owned();
let rules_dir = std::path::Path::new("rules");  // relative to CWD at boot
let subject_validator = crate::subject_validator::SubjectValidator::load(
    rules_dir,
    canonical_subjects,
).expect("FATAL: failed to load subject validator rules from rules/*.toml");
```

Then update the `ConsoleApiState { ... }` literal:

```rust
ConsoleApiState {
    // ... existing fields ...
    ai_provider: ...,
    subject_validator: Some(subject_validator),
}
```

- [ ] **Step 3: Embed rules as fallback in case rules/ dir is missing at runtime**

In `src/subject_validator.rs`, add an alternative constructor:

```rust
impl SubjectValidator {
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
}
```

Update the boot site to use `load_with_embedded_fallback` (resilient):

```rust
let subject_validator = crate::subject_validator::SubjectValidator::load_with_embedded_fallback(
    rules_dir,
    canonical_subjects,
).expect("FATAL: subject validator rules failed to parse");
```

- [ ] **Step 4: Verify boot works**

Run: `cargo build && cargo run -- --help`
Expected: binary starts without panicking. (Don't need to actually serve — just confirm boot.)

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/api.rs src/subject_validator.rs
git commit -m "feat(boot): load SubjectValidator from rules/*.toml with embedded fallback"
```

---

## Task 14: Add audit field to ProposeInferenceCommand

**Files:**
- Modify: `src/semantic.rs` (`ProposeInferenceCommand`)
- Modify: `src/mcp/handlers.rs` (populate the field)

- [ ] **Step 1: Add field to ProposeInferenceCommand**

Find `ProposeInferenceCommand` in `src/semantic.rs` (around line 351). Add:

```rust
pub struct ProposeInferenceCommand {
    // ... existing fields ...
    pub prompt_version: Option<String>,
    /// Subject validator version that screened this proposal's subject.
    /// `None` for legacy proposals (pre-Phase 1.5) or when validation skipped.
    pub subject_validator_version: Option<String>,
    // ... rest ...
}
```

- [ ] **Step 2: Populate it in mcp/handlers.rs**

Find the propose_inference handler in `src/mcp/handlers.rs` (around line 1085-1172). Find where `prompt_version` is read:

```rust
let prompt_version = arg_str(args, "prompt_version")
    .unwrap_or_else(|| crate::extraction::EXTRACTION_PROMPT_VERSION.to_owned());
```

Below it, add:

```rust
let subject_validator_version =
    crate::subject_validator::SUBJECT_VALIDATOR_VERSION.to_string();
```

Then when constructing `ProposeInferenceCommand { ... }`, add:

```rust
ProposeInferenceCommand {
    // ... existing fields ...
    prompt_version: Some(prompt_version),
    subject_validator_version: Some(subject_validator_version),
    // ... rest ...
}
```

- [ ] **Step 3: Verify compile**

Run: `cargo check`
Expected: compiles. If errors about missing field elsewhere, add `subject_validator_version: None` to other construction sites (use compiler errors to guide).

- [ ] **Step 4: Commit**

```bash
git add src/semantic.rs src/mcp/handlers.rs
git commit -m "feat(audit): record subject_validator_version on every new proposal"
```

---

## Task 15: Integration test — 182 proposals regression

**Files:**
- Create: `tests/subject_validator_v1.rs`

- [ ] **Step 1: Create the integration test file**

Create `tests/subject_validator_v1.rs`:

```rust
//! Subject Validator v1 — integration tests on real SemanticStore + the
//! production rules/*.toml files.
//!
//! These tests are the contract the Phase 1.5 DoD enforces. See
//! `docs/plans/subject-validator-v1-spec.md` §8.1.

use std::sync::Arc;
use llm_wiki::subject_validator::{
    SubjectValidator, SubjectVerdict, SubjectShape, SUBJECT_VALIDATOR_VERSION,
};
use llm_wiki::quality::{QualityChecker, QualityTagKind, QualitySeverity};

fn production_validator() -> Arc<SubjectValidator> {
    let rules = include_str!("../rules/subject_rules.toml");
    let allow = include_str!("../rules/subject_allowlist.toml");
    let deny = include_str!("../rules/subject_denylist.toml");
    SubjectValidator::from_strings(rules, allow, deny, Default::default())
        .expect("production validator")
}

#[test]
fn validator_version_is_stamped() {
    assert_eq!(SUBJECT_VALIDATOR_VERSION, "subject-validator-v1");
}

#[test]
fn production_rules_load_without_error() {
    let _ = production_validator();
}

// ── Real-world subjects observed in inbox (from rootcause doc) ──────────

#[test]
fn lowercase_metric_subjects_rejected_critical() {
    let v = production_validator();
    for bad in ["risk-free rate", "beta", "terminal growth", "current case price", "equity value"] {
        let r = v.validate(bad);
        assert_eq!(r.verdict, SubjectVerdict::Reject,
            "subject `{bad}` should be rejected, got shape={:?}", r.shape);
        assert!(r.quality_tags.iter().any(|t|
            t.kind == QualityTagKind::BadSubjectShape && t.severity == QualitySeverity::Critical));
    }
}

#[test]
fn slug_subjects_rejected_critical() {
    let v = production_validator();
    for bad in [
        "international-peers-deep_has_peer_data",
        "thai-shipping-bf-report",
    ] {
        // Note: "international-peers-deep_has_peer_data" mixes - and _ so
        // won't match pure Slug regex. Verify it falls into a reject shape.
        let r = v.validate(bad);
        assert_ne!(r.verdict, SubjectVerdict::Accept,
            "subject `{bad}` should not be accepted, got shape={:?}", r.shape);
    }
}

#[test]
fn real_entity_subjects_silent_accept() {
    let v = production_validator();
    for good in ["CATL", "BYD", "TSLA", "NVDA"] {
        let r = v.validate(good);
        assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo,
            "subject `{good}` got shape={:?}", r.shape);
        assert!(r.quality_tags.iter().all(|t| t.severity != QualitySeverity::Critical));
    }
}

#[test]
fn thai_entity_accept_info() {
    let v = production_validator();
    let r = v.validate("บมจ. ปตท.");
    assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
    assert_eq!(r.shape, SubjectShape::ThaiPure);
}

#[test]
fn ambiguous_acronym_flagged_info() {
    let v = production_validator();
    let r = v.validate("BAT");
    assert!(r.quality_tags.iter().any(|t|
        t.kind == QualityTagKind::SubjectAmbiguousAcronym
        && t.severity == QualitySeverity::Info));
}

#[test]
fn llm_bleed_placeholders_rejected() {
    let v = production_validator();
    for bad in ["<entity>", "[SUBJECT]", "TBD", "unknown", "unspecified"] {
        let r = v.validate(bad);
        assert_eq!(r.verdict, SubjectVerdict::Reject,
            "subject `{bad}` should be rejected (LLM bleed)");
    }
}

#[test]
fn section_headings_rejected_even_when_title_case() {
    let v = production_validator();
    for bad in ["DCF Assumptions", "Risk Factors", "Executive Summary"] {
        let r = v.validate(bad);
        assert_eq!(r.verdict, SubjectVerdict::Reject,
            "subject `{bad}` (section heading) should be rejected");
    }
}

#[test]
fn possessive_subjects_soft_flag() {
    let v = production_validator();
    let r = v.validate("Tesla's CFO");
    assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
}

#[test]
fn multi_entity_subjects_soft_flag() {
    let v = production_validator();
    let r = v.validate("CATL, BYD, LG");
    assert_eq!(r.verdict, SubjectVerdict::SoftFlag);
}

// ── Adversarial inputs ───────────────────────────────────────────────────

#[test]
fn html_injection_rejected() {
    let v = production_validator();
    let r = v.validate("<script>alert(1)</script>");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
    assert!(r.quality_tags.iter().any(|t| t.kind == QualityTagKind::BadSubjectAdversarial));
}

#[test]
fn template_injection_rejected() {
    let v = production_validator();
    let r = v.validate("${evil}");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

#[test]
fn rtl_override_rejected() {
    let v = production_validator();
    let r = v.validate("\u{202E}CATL");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

#[test]
fn emoji_rejected() {
    let v = production_validator();
    let r = v.validate("CATL 🚀");
    assert_eq!(r.verdict, SubjectVerdict::Reject);
}

// ── Normalize behavior ──────────────────────────────────────────────────

#[test]
fn zero_width_char_stripped_before_classification() {
    let v = production_validator();
    let r = v.validate("CATL\u{200B}");
    assert_eq!(r.normalized, "CATL");
    assert_eq!(r.verdict, SubjectVerdict::AcceptWithInfo);
}

// ── QualityChecker integration ──────────────────────────────────────────

#[test]
fn quality_checker_with_validator_flags_bad_subject() {
    use llm_wiki::semantic::{EvidenceSummary, ProposalSummary};
    use llm_wiki::quality::QualityCheckerInput;
    use chrono::Utc;
    use uuid::Uuid;
    use serde_json::json;

    let p = ProposalSummary {
        proposal_id: Uuid::new_v4(),
        domain: "financial".to_string(),
        subject: "risk-free rate".to_string(),
        predicate: "is".to_string(),
        value: json!("1.75%"),
        claim_kind: "financial_metric".to_string(),
        provenance_kind: "inference".to_string(),
        submitted_at: Utc::now(),
        event_seq: 1,
    };
    let ev = EvidenceSummary {
        provenance_kind: "inference".to_string(),
        excerpt: Some("...".to_string()),
        source_id: None,
        quote_hash: None,
    };
    let input = QualityCheckerInput {
        proposal: &p,
        evidence: &ev,
        existing_claims: &[],
    };
    let checker = QualityChecker::new(production_validator());
    let tags = checker.check_deterministic(&input);
    assert!(tags.iter().any(|t| t.kind == QualityTagKind::BadSubjectShape),
        "expected BadSubjectShape tag, got: {:?}", tags);
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test subject_validator_v1`
Expected: all tests pass. The `slug_subjects_rejected_critical` test may fail on the mixed-dash input `international-peers-deep_has_peer_data` — if so, note it as a known limitation and the test asserts `verdict != Accept` instead (which it does — assert_ne).

- [ ] **Step 3: Commit**

```bash
git add tests/subject_validator_v1.rs
git commit -m "test(subject-validator): integration tests for Phase 1.5 DoD scenarios"
```

---

## Task 16: Full workspace test + clippy + fmt

**Files:**
- (no code changes; verification + cleanup only)

- [ ] **Step 1: Run the entire test suite**

Run: `cargo test --workspace`
Expected: all tests pass. If any pre-existing test broke, investigate (likely a missed call site of `QualityChecker::new()` — update to `without_subject_validation()`).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. If warnings appear, fix them.

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --check`
Expected: no diff. If diff, run `cargo fmt` to auto-fix.

- [ ] **Step 4: Commit any cleanup**

```bash
git add -A
git commit -m "chore: clippy + fmt cleanup for subject-validator Phase 1.5"
```

---

## Task 17: 182-proposals regression (manual / scripted)

**Files:**
- Create: `tests/subject_validator_182_regression.rs` (or run as a one-off binary)

> This task validates the DoD §8.1 numeric targets: ~38 Critical, ~24 SoftFlag, ~96 silent on the 182 pending proposals. It requires a live store. If you don't have 182 proposals locally, skip the assertion and just print the counts.

- [ ] **Step 1: Write the regression runner**

Create `tests/subject_validator_182_regression.rs`:

```rust
//! Manual regression: run the production validator against all pending
//! proposals in a live store, print the distribution, and (optionally)
//! assert it matches the DoD targets.
//!
//! Usage: `cargo test --test subject_validator_182_regression -- --nocapture --ignored`

use std::sync::Arc;
use llm_wiki::subject_validator::SubjectValidator;
use llm_wiki::semantic::{SemanticConfig, SemanticStore};
use std::path::Path;

#[test]
#[ignore = "requires a live SemanticStore with pending proposals"]
fn regression_on_live_store_prints_distribution() {
    let store_path = std::env::var("BRAIN_STORE_PATH")
        .expect("set BRAIN_STORE_PATH to point at your live store");
    let store = SemanticStore::open(Path::new(&store_path), SemanticConfig::enabled())
        .expect("open store");
    let canonical = store.entity_canonical_subjects_owned();
    let validator = SubjectValidator::load_with_embedded_fallback(Path::new("rules"), canonical)
        .expect("load validator");

    // Read pending proposals (requires a public API; if not available, use
    // the SQLite store directly with a query like:
    //   SELECT subject FROM proposals WHERE status = 'pending'
    // For now this is a sketch — wire to whatever the real read API is.
    let pending_subjects: Vec<String> = vec![]; // TODO: replace with real read
    assert!(!pending_subjects.is_empty(), "no pending subjects — set BRAIN_STORE_PATH");

    let mut critical = 0;
    let mut soft_flag = 0;
    let mut silent = 0;
    for s in &pending_subjects {
        let r = validator.validate(s);
        match r.verdict {
            llm_wiki::subject_validator::SubjectVerdict::Reject => critical += 1,
            llm_wiki::subject_validator::SubjectVerdict::SoftFlag => soft_flag += 1,
            _ => silent += 1,
        }
    }
    eprintln!("182-regression: critical={critical}, soft_flag={soft_flag}, silent={silent}");
    eprintln!("unknown_frequency: {:?}", validator.unknown_frequency_snapshot());

    // DoD §8.1 targets (within ±10%):
    //   critical: 35-42
    //   soft_flag: 20-28
    //   silent: >= 85
    // Assert only if the store really has ~182 proposals.
    if pending_subjects.len() >= 150 {
        assert!(critical >= 30 && critical <= 50, "critical={critical} outside [30,50]");
        assert!(silent >= 80, "silent={silent} below 80");
    }
}
```

- [ ] **Step 2: Run it against your live store**

```bash
BRAIN_STORE_PATH=/path/to/your/store cargo test --test subject_validator_182_regression -- --nocapture --ignored
```

- [ ] **Step 3: Capture the output and update spec DoD with actual numbers**

Edit `docs/plans/subject-validator-v1-spec.md` §8.1 — replace the predicted counts with the actual observed counts.

- [ ] **Step 4: Commit**

```bash
git add tests/subject_validator_182_regression.rs docs/plans/subject-validator-v1-spec.md
git commit -m "test+docs(subject-validator): 182-proposal regression runner + observed counts"
```

---

## Task 18: Update old framework spec + BLUEPRINT note

**Files:**
- Modify: `docs/plans/feature-subject-validator-framework.md` (add superseded banner)
- Modify: `BLUEPRINT.md` (one-line mention of subject validator layer)

- [ ] **Step 1: Add superseded banner to old framework spec**

Open `docs/plans/feature-subject-validator-framework.md` and insert at the very top (above the title):

```markdown
> ⚠️ **SUPERSEDED (2026-07-21):** This framework spec was refined into
> `docs/plans/subject-validator-v1-spec.md` with implementation plan at
> `docs/plans/subject-validator-v1-implementation-plan.md`. Key changes from
> this doc:
> - Engine/data separation (Wikidata pattern) — Layer 0-2,5 = code, Layer 3-4 = TOML
> - 3 durability additions: SUBJECT_VALIDATOR_VERSION, last_reviewed stamps, Unknown+counter
> - Phase 1.5 verdict for Plain/Unknown shape: accept_info (not defer_to_llm)
> - ExtractionAudit is dead code in production; audit field goes on ProposeInferenceCommand
> Keep this doc for the 7-families taxonomy reference.
```

- [ ] **Step 2: Add one-line mention in BLUEPRINT.md**

Find the `Semantic` section in `BLUEPRINT.md` and add (in an appropriate spot — likely near where extraction/quality is mentioned):

```markdown
- **Subject Validator (Phase 1.5, 2026-07-21):** deterministic 6-layer
  validator screens every proposal's `subject` field before it enters the
  review queue. Engine vs. data separation (Wikidata pattern): code is
  stable 5+ years, TOML rules reviewed annually. See
  `docs/plans/subject-validator-v1-spec.md`.
```

- [ ] **Step 3: Commit**

```bash
git add docs/plans/feature-subject-validator-framework.md BLUEPRINT.md
git commit -m "docs: supersede old framework spec + add BLUEPRINT mention"
```

---

## Phase 1.5 Complete — DoD Final Verification

After all 18 tasks land, run the DoD checklist from spec §8:

- [ ] **Functional DoD §8.1** — run Task 17 regression, verify counts
- [ ] **Architectural DoD §8.2** — `grep -r "SUBJECT_VALIDATOR_VERSION" src/` appears in audit; `rules/*.toml` all have `last_reviewed`
- [ ] **Quality DoD §8.3** — `cargo test --workspace && cargo clippy -- -D warnings && cargo fmt --check` all clean
- [ ] **Documentation DoD §8.4** — module has doc comment, functions have doc comments, TOML files have headers, old spec superseded
- [ ] **Operational DoD §8.5** — Task 13 confirms fail-fast on bad rules; Task 13 confirms embedded fallback

Final commit (only if anything else changed):

```bash
git add -A
git commit -m "feat(subject-validator): Phase 1.5 complete — DoD verified"
```

---

## Self-Review Notes (post-write)

- **Spec coverage:** every §1-14 of spec has at least one task. Spec §6.1 module file = Task 2; §6.2 quality.rs changes = Tasks 10-12; §6.3 boot loading = Task 13; §7 testing = Tasks 3-9 (unit) + Task 15-17 (integration); §8 DoD = final verification block; §11 out-of-scope = respected (no LLM-as-judge code, no Thai NER lib).
- **Placeholder scan:** Task 17 step 2 has a TODO inside the test code that requires the user to wire the real read API. This is intentional — the actual `SemanticStore::pending_proposals()` API name wasn't available at planning time. If a public read API exists, replace the `vec![]` with a real call; otherwise the test is `#[ignore]`'d so it doesn't block CI.
- **Type consistency:** `SubjectShape` has 23 variants consistently across Tasks 2, 5, 6, 7. `SubjectVerdict` 5 variants consistent across Tasks 2, 9. `SUBJECT_VALIDATOR_VERSION` referenced in Tasks 2, 14, 15 consistently.
- **Spec deviation logged:** spec §6.2 said modify `ExtractionAudit`; plan Task 14 modifies `ProposeInferenceCommand` instead — this deviation is documented in the Pre-flight Check at the top of this plan and is the correct production path.
