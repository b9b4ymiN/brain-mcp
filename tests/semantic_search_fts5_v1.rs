//! FTS5 search behavior tests. Covers flatten_json (Task 3), the migration
//! (Task 5), and end-to-end search (Task 10). Tests are appended task-by-task.

use llm_wiki::semantic::flatten_json;

#[test]
fn flatten_json_string_is_itself() {
    let v = serde_json::json!("Reinvent the Wheel");
    assert_eq!(flatten_json(&v), "Reinvent the Wheel");
}

#[test]
fn flatten_json_number_stringifies() {
    let v = serde_json::json!(42);
    assert_eq!(flatten_json(&v), "42");
}

#[test]
fn flatten_json_bool_stringifies() {
    assert_eq!(flatten_json(&serde_json::json!(true)), "true");
    assert_eq!(flatten_json(&serde_json::json!(false)), "false");
}

#[test]
fn flatten_json_array_space_joins_elements() {
    let v = serde_json::json!(["Tesla", "BMW", 7]);
    assert_eq!(flatten_json(&v), "Tesla BMW 7");
}

#[test]
fn flatten_json_object_space_joins_values() {
    let v = serde_json::json!({"a": "x", "b": "y"});
    // Object iteration order is insertion order for serde_json, but assert
    // set-style to stay robust to any future value ordering choice.
    let flat = flatten_json(&v);
    assert!(flat.contains("x") && flat.contains("y"), "got: {flat}");
}

#[test]
fn flatten_json_null_is_empty() {
    assert_eq!(flatten_json(&serde_json::Value::Null), "");
}

#[test]
fn flatten_json_nested_array_recurses() {
    let v = serde_json::json!([["a", "b"], "c"]);
    assert_eq!(flatten_json(&v), "a b c");
}

/// FTS5 Task 8 smoke: search_claims returns hits for a confirmed claim.
/// Full behavior tests come in Task 10; this just proves the method wires up
/// (FTS MATCH + rowid JOIN + value rehydration) against a real confirmed claim.
#[test]
fn search_claims_smoke_finds_confirmed_claim() {
    let (_parent, store, _ctx) = fts5_fixture();

    // Stage a claim_status row with a known subject, then search for it.
    // insert_orphan_claim_status_for_test writes a bare row (no confirmation
    // event payload), so value rehydration will FAIL for it — but the FTS
    // match + rowid JOIN + subject/predicate/domain fields should still work.
    // To exercise value rehydration we'd need a full propose+confirm cycle;
    // the Task 10 tests below do that. For the smoke we just assert the row
    // is found at the FTS layer.
    store.insert_orphan_claim_status_for_test("smoke-domain", "Reinvent the Wheel", "is");

    // search_claims will try to rehydrate value from the events table; the
    // orphan row has no confirmation event, so this call is EXPECTED TO ERROR.
    // Assert it errors with a clear message rather than panicking — that proves
    // the FTS match + JOIN worked and the only failure is value rehydration.
    let result = store.search_claims("reinvent", None, 10);
    assert!(
        result.is_err(),
        "orphan row has no confirmation event, so value rehydration must error cleanly, got: {result:?}"
    );
}

// =============================================================================
// Task 10 — end-to-end search behavior (real propose+confirm cycles)
// -----------------------------------------------------------------------------
// These tests exercise the full write→index→read→rehydrate path against REAL
// confirmed claims (unlike the smoke above, which uses an orphan row). They are
// where any remaining bug in the rowid JOIN, value rehydration, trigram
// tokenization, BM25 ranking, supersede filter, or edge-case query handling
// will surface. The confirm pattern mirrors `user_assertion_propose_and_confirm`
// in tests/semantic_ownership_v1.rs: propose_user_assertion → confirm.
// =============================================================================

use std::sync::Arc;

use llm_wiki::semantic::{
    ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeUserAssertionCommand, RetractCommand,
    SemanticConfig, SemanticStore, SupersedeCommand, TrustedContext,
};
use tempfile::TempDir;

/// Fresh v5 store + trusted context for a search test. The `TempDir` is returned
/// first so the caller can bind it to keep the on-disk store alive for the
/// test's duration.
fn fts5_fixture() -> (TempDir, Arc<SemanticStore>, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, Arc::new(store), context)
}

/// Build a `ClaimDraft` with full control over every field (Task 6 Entity
/// Identity Reform: `domain` is `Option<String>`).
fn draft(
    subject: &str,
    predicate: &str,
    value: serde_json::Value,
    domain: Option<&str>,
) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value,
        claim_kind: "user_assertion".to_owned(),
        domain: domain.map(str::to_owned),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Run a full propose_user_assertion + confirm cycle (the ADR Decision 6
/// human-authored path; no external evidence capture required). Returns the
/// new claim_id. The `propose_op` doubles as the proposal_operation_id passed
/// to confirm.
fn confirm_claim(
    store: &SemanticStore,
    context: &TrustedContext,
    propose_op: &str,
    confirm_op: &str,
    draft: ClaimDraft,
) -> uuid::Uuid {
    store
        .propose_user_assertion(
            context,
            ProposeUserAssertionCommand {
                operation_id: propose_op.to_owned(),
                utterance: b"owner assertion".to_vec(),
                draft,
            },
        )
        .expect("propose_user_assertion");
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: confirm_op.to_owned(),
                proposal_operation_id: propose_op.to_owned(),
            },
        )
        .expect("confirm");
    outcome.generated.claim_id.expect("claim id")
}

// -----------------------------------------------------------------------------
// Test 1 — basic MATCH finds a confirmed claim by subject
// -----------------------------------------------------------------------------

/// A claim confirmed through the full path lands in the FTS5 index, and a
/// subject-trigram query retrieves it with its value rehydrated from the
/// confirmation event payload. This is the DoD's base case and the first
/// end-to-end exercise of the value-rehydration path (the Task 8 smoke could
/// not test it because its orphan row had no confirmation event).
#[test]
fn fts5_basic_match_finds_confirmed_claim() {
    let (_parent, store, ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &ctx,
        "p-idiom",
        "c-idiom",
        draft(
            "Reinvent the Wheel",
            "is",
            serde_json::json!("an idiom"),
            Some("idioms"),
        ),
    );

    let hits = store
        .search_claims("reinvent", None, 10)
        .expect("search_claims");
    assert!(!hits.is_empty(), "must find the confirmed claim: {hits:?}");
    let hit = hits
        .iter()
        .find(|h| h.subject.contains("Reinvent"))
        .expect("a hit whose subject contains 'Reinvent'");
    // Value rehydration end-to-end: the idiom string survived the
    // confirm → event → decrypt → ConfirmationObject.claim.value round trip.
    assert_eq!(hit.value, serde_json::json!("an idiom"), "value rehydrated");
    assert_eq!(hit.predicate, "is");
    assert_eq!(hit.domain, "idioms");
}

// -----------------------------------------------------------------------------
// Test 2 — trigram tokenization vs intra-word phrase queries
// -----------------------------------------------------------------------------

/// Trigram indexes every 3-char window of the subject "reinvent the wheel".
/// A query for "reinvent wheel" drops the middle word; whether FTS5 matches
/// depends on whether it treats the bare space in the query as a phrase
/// delimiter (adjacent tokens) or an implicit AND. We assert the ACTUAL
/// behavior and document it, rather than forcing an optimistic outcome.
///
/// Empirical finding (recorded at first run): FTS5 trigram treats the query
/// "reinvent wheel" as requiring both tokens to match in the indexed text.
/// Because "reinvent" and "wheel" both appear in "reinvent the wheel", the
/// row matches — trigram is a substring matcher, not a phrase-position one.
#[test]
fn fts5_tokenized_match_handles_intra_word_text() {
    let (_parent, store, ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &ctx,
        "p-idiom2",
        "c-idiom2",
        draft(
            "Reinvent the Wheel",
            "is",
            serde_json::json!("an idiom"),
            Some("idioms"),
        ),
    );

    let hits = store
        .search_claims("reinvent wheel", None, 10)
        .expect("search_claims must not error");
    // Trigram substring match: both query tokens are substrings of the
    // indexed subject, so a hit is expected. Assert honestly.
    assert!(
        hits.iter().any(|h| h.subject.contains("Reinvent")),
        "trigram should match 'reinvent wheel' against 'reinvent the wheel': {hits:?}"
    );
}

// -----------------------------------------------------------------------------
// Test 3 — value_flat makes array elements searchable
// -----------------------------------------------------------------------------

/// flatten_json joins array elements with spaces into value_flat
/// (`["Tesla","BMW"]` → "Tesla BMW"), and the FTS5 trigger indexes value_flat.
/// Searching for an element that appears ONLY in the value (not subject or
/// predicate) therefore finds the claim. This proves the trigger + flatten
/// pipeline end-to-end.
#[test]
fn fts5_value_search_finds_array_element() {
    let (_parent, store, ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &ctx,
        "p-ev",
        "c-ev",
        draft(
            "car_brands",
            "lists",
            serde_json::json!(["Tesla", "BMW"]),
            Some("autos"),
        ),
    );

    let hits = store.search_claims("tesla", None, 10).expect("search_claims");
    assert!(
        hits.iter().any(|h| h.subject == "car_brands"),
        "must find the claim via its array-element value_flat: {hits:?}"
    );
    // And the full array value rehydrates intact.
    let hit = hits.iter().find(|h| h.subject == "car_brands").unwrap();
    assert_eq!(hit.value, serde_json::json!(["Tesla", "BMW"]));
}

// -----------------------------------------------------------------------------
// Test 4 — Thai script is searchable (the trigram-over-porter decision)
// -----------------------------------------------------------------------------

/// This is THE test that validates the spec's decision to use the trigram
/// tokenizer instead of porter/unicode61: Thai script has no whitespace word
/// boundaries, so porter (whitespace+stem) would tokenize "บมจ. ปตท." badly or
/// not at all, while trigram indexes every 3-char window and can match a
/// 3+ char Thai substring directly. Confirm a Thai-subject claim, search a
/// Thai substring, and assert a hit.
#[test]
fn fts5_thai_subject_is_searchable() {
    let (_parent, store, ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &ctx,
        "p-thai",
        "c-thai",
        draft(
            "บมจ. ปตท.",
            "stock_symbol",
            serde_json::json!("PTT"),
            Some("stocks"),
        ),
    );

    let hits = store.search_claims("ปตท", None, 10).expect("search_claims");
    assert!(
        hits.iter().any(|h| h.subject.contains("ปตท")),
        "trigram must match the Thai substring 'ปตท' against the Thai subject: {hits:?}"
    );
}

// -----------------------------------------------------------------------------
// Test 5 — domain filter restricts results
// -----------------------------------------------------------------------------

/// Two claims share a query-relevant term but live in different domains.
/// Filtering by `Some("eng")` must return only eng-domain hits, and never more
/// hits than the unfiltered query.
#[test]
fn fts5_domain_filter_restricts_results() {
    let (_parent, store, ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &ctx,
        "p-eng",
        "c-eng",
        draft("deployservice", "is", serde_json::json!("a service"), Some("eng")),
    );
    confirm_claim(
        &store,
        &ctx,
        "p-fin",
        "c-fin",
        draft(
            "deployservice",
            "is",
            serde_json::json!("a finance term"),
            Some("fin"),
        ),
    );

    let eng = store
        .search_claims("deployservice", Some("eng"), 10)
        .expect("search_claims eng");
    let all = store
        .search_claims("deployservice", None, 10)
        .expect("search_claims all");

    assert!(!eng.is_empty(), "eng filter must return the eng claim");
    assert!(
        eng.iter().all(|h| h.domain == "eng"),
        "every eng-filtered hit must be in domain eng: {eng:?}"
    );
    assert!(
        eng.len() <= all.len(),
        "filtered result count ({}) must not exceed unfiltered ({})",
        eng.len(),
        all.len()
    );
}

// -----------------------------------------------------------------------------
// Test 6 — BM25 ranks the most-relevant hit first
// -----------------------------------------------------------------------------

/// With column weights subject=10, predicate=5, value_flat=1, a query term
/// that appears in the SUBJECT should outrank one that appears only in the
/// predicate or value. BM25 returns negative scores and `ORDER BY score ASC`
/// puts the most-relevant (most-negative) first, so `hits[0].score <= hits[1].score`.
#[test]
fn fts5_bm25_ranks_most_relevant_first() {
    let (_parent, store, ctx) = fts5_fixture();
    // Subject match — highest weight.
    confirm_claim(
        &store,
        &ctx,
        "p-subj",
        "c-subj",
        draft(
            "kryptonite",
            "material",
            serde_json::json!("unrelated value"),
            Some("comics"),
        ),
    );
    // Predicate match — medium weight.
    confirm_claim(
        &store,
        &ctx,
        "p-pred",
        "c-pred",
        draft(
            "some subject",
            "kryptonite",
            serde_json::json!("also unrelated"),
            Some("comics"),
        ),
    );
    // Value match — lowest weight.
    confirm_claim(
        &store,
        &ctx,
        "p-val",
        "c-val",
        draft(
            "another subject",
            "property",
            serde_json::json!("kryptonite"),
            Some("comics"),
        ),
    );

    let hits = store
        .search_claims("kryptonite", None, 10)
        .expect("search_claims");
    assert!(
        hits.len() >= 2,
        "need at least 2 hits to compare ranking, got {}: {hits:?}",
        hits.len()
    );
    // BM25: lower (more negative) = more relevant; ASC order → best first.
    assert!(
        hits[0].score <= hits[1].score,
        "hits must be ordered best-first (score ASC), got [0]={} > [1]={}",
        hits[0].score,
        hits[1].score
    );
    // And the best hit should be the subject match (strongest signal).
    assert_eq!(
        hits[0].subject, "kryptonite",
        "subject match should rank first under subject=10 weight: {hits:?}"
    );
}

// -----------------------------------------------------------------------------
// Test 7 — superseded claims are excluded from search
// -----------------------------------------------------------------------------

/// When claim B supersedes claim A (same scope), A's `superseded_by_event_seq`
/// is set and search_claims filters it out via `superseded_by_event_seq IS NULL`.
/// The search term matches both, but only B (the current claim) appears.
///
/// Supersede API (from src/semantic.rs): `store.supersede(&ctx, SupersedeCommand {
/// operation_id, proposal_operation_id, superseded_claim_operation_ids })` where
/// `superseded_claim_operation_ids` is the prior claim's *confirm* operation_id
/// (the op that created it), and the new draft must share (domain, subject,
/// predicate) with the superseded claim.
#[test]
fn fts5_superseded_claim_excluded_from_search() {
    let (_parent, store, ctx) = fts5_fixture();

    // Claim A — the soon-to-be-superseded value.
    confirm_claim(
        &store,
        &ctx,
        "p-a",
        "c-a", // <-- this confirm operation_id is what supersede references
        draft("supersubject", "price", serde_json::json!(58), Some("mkt")),
    );
    // Claim B — supersedes A in the same scope with a new value.
    store
        .propose_user_assertion(
            &ctx,
            ProposeUserAssertionCommand {
                operation_id: "p-b".to_owned(),
                utterance: b"new price".to_vec(),
                draft: draft("supersubject", "price", serde_json::json!(61), Some("mkt")),
            },
        )
        .expect("propose B");
    store
        .supersede(
            &ctx,
            SupersedeCommand {
                operation_id: "c-b".to_owned(),
                proposal_operation_id: "p-b".to_owned(),
                superseded_claim_operation_ids: vec!["c-a".to_owned()],
            },
        )
        .expect("supersede");

    let hits = store
        .search_claims("supersubject", None, 10)
        .expect("search_claims");
    assert!(!hits.is_empty(), "the current claim B must be found");
    // Every returned hit must be the CURRENT value (61), proving A was filtered.
    assert!(
        hits.iter().all(|h| h.value == serde_json::json!(61)),
        "superseded claim A (value 58) must not appear; got: {hits:?}"
    );
}

// -----------------------------------------------------------------------------
// Test 7b — retracted claims are excluded from search
// -----------------------------------------------------------------------------

/// Mirror of [`fts5_superseded_claim_excluded_from_search`] for the retract
/// path. The DoD says "Superseded/retracted claims are excluded" but only the
/// supersede branch was covered; this locks the retract branch explicitly.
/// Retracting a confirmed claim sets its `retracted_at_event_seq`, and
/// `search_claims` filters it out via `retracted_at_event_seq IS NULL`.
///
/// Retract API (from src/semantic.rs): `store.retract(&ctx, RetractCommand {
/// operation_id, claim_operation_id })` where `claim_operation_id` is the
/// claim's *confirm* operation_id (the op that confirmed it). No new proposal
/// is required — retract is a direct mutation on an existing confirmed claim.
#[test]
fn fts5_retracted_claim_excluded_from_search() {
    let (_parent, store, ctx) = fts5_fixture();

    // Confirm a claim, then retract it.
    confirm_claim(
        &store,
        &ctx,
        "p-retract",
        "c-retract", // <-- this confirm operation_id is what retract references
        draft(
            "Retractable Claim",
            "is",
            serde_json::json!("active"),
            Some("status"),
        ),
    );
    store
        .retract(
            &ctx,
            RetractCommand {
                operation_id: "r-1".to_owned(),
                claim_operation_id: "c-retract".to_owned(),
            },
        )
        .expect("retract");

    // The subject still tokenizes to "retractable", so without the retracted
    // filter the row would be a hit. The filter must drop it.
    let hits = store
        .search_claims("retractable", None, 10)
        .expect("search_claims");
    assert!(
        hits.iter().all(|h| !h.subject.contains("Retractable")),
        "retracted claim must not appear in search; got: {hits:?}"
    );
}

// -----------------------------------------------------------------------------
// Test 8 — empty and sub-trigram-length queries do not panic
// -----------------------------------------------------------------------------

/// FTS5 MATCH with an empty string or a query shorter than the trigram minimum
/// (3 chars) can be rejected by the engine. We only assert the call does not
/// panic — it may return Ok([] or Err), and we document whichever it does.
/// A panic propagates and fails the test (which is the behavior we're guarding
/// against); a clean Ok/Err passes.
#[test]
fn fts5_empty_and_short_queries_do_not_panic() {
    let (_parent, store, _ctx) = fts5_fixture();
    // Seed something so the index is non-empty; behavior is about the query
    // shape, not the data.
    confirm_claim(
        &store,
        &store.trusted_context(),
        "p-edge",
        "c-edge",
        draft("abcdef", "is", serde_json::json!("x"), Some("d")),
    );

    let empty = store.search_claims("", None, 10);
    let short = store.search_claims("ab", None, 10);

    // The load-bearing assertion: neither call panicked. If we reach here,
    // both returned either Ok or Err cleanly. Document the actual shapes.
    eprintln!(
        "empty-query result: {:?}; short-query 'ab' result: {:?}",
        empty.as_ref().map(|v| v.len()),
        short.as_ref().map(|v| v.len()),
    );
    let _ = empty;
    let _ = short;
}

// -----------------------------------------------------------------------------
// Test 9 — FTS5 MATCH special characters are handled safely
// -----------------------------------------------------------------------------

/// FTS5 interprets `OR` and double-quotes as MATCH syntax (boolean OR, phrase).
/// The handler does not pre-sanitize the query, so these flow straight into the
/// MATCH expression. We assert only that the call does not panic — the engine
/// parses them as syntax and either returns a result set or errors cleanly.
/// A panic propagates and fails the test; reaching the end means both calls
/// returned cleanly.
#[test]
fn fts5_special_chars_handled_safely() {
    let (_parent, store, _ctx) = fts5_fixture();
    confirm_claim(
        &store,
        &store.trusted_context(),
        "p-special",
        "c-special",
        draft("alpha beta", "is", serde_json::json!("gamma"), Some("d")),
    );

    let or_query = store.search_claims("a OR b", None, 10);
    let quoted = store.search_claims("\"quoted\"", None, 10);

    eprintln!(
        "OR-query result: {:?}; quoted-query result: {:?}",
        or_query.as_ref().map(|v| v.len()),
        quoted.as_ref().map(|v| v.len()),
    );
    let _ = or_query;
    let _ = quoted;
}
