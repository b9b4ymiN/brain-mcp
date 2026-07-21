//! Manual regression: run the production validator against all pending
//! proposals in a live store, print the distribution, and (optionally)
//! assert it matches the DoD targets.
//!
//! Usage: `cargo test --test subject_validator_182_regression -- --nocapture --ignored`
//!
//! DoD §8.1 targets (within ±10%):
//!   critical: 30-50
//!   soft_flag: 15-35
//!   silent: >= 80
//!
//! Set BRAIN_STORE_PATH env var to your live store directory before running.

#![cfg(feature = "live_regression")]

use std::path::Path;
use std::sync::Arc;

use llm_wiki::semantic::{SemanticConfig, SemanticStore};
use llm_wiki::subject_validator::{SubjectValidator, SubjectVerdict};

#[test]
#[ignore = "requires BRAIN_STORE_PATH + feature live_regression"]
fn regression_on_live_store_prints_distribution() {
    let store_path = std::env::var("BRAIN_STORE_PATH")
        .expect("set BRAIN_STORE_PATH to point at your live store");
    let store = SemanticStore::open(
        Path::new(&store_path),
        SemanticConfig::enabled_for(Path::new(&store_path)),
    )
    .expect("open store");
    let canonical = store.entity_canonical_subjects_owned();
    let validator: Arc<SubjectValidator> =
        SubjectValidator::load_with_embedded_fallback(Path::new("rules"), canonical)
            .expect("load validator");

    // TODO: wire to SemanticStore::pending_proposals_subjects() when that
    // API exists. For now, this is a sketch — the test is feature-gated
    // and ignored so it doesn't run in CI. To use:
    //   1. Add a public method on SemanticStore returning pending subjects.
    //   2. Replace the empty vec below with the real call.
    //   3. Run: cargo test --test subject_validator_182_regression \
    //               --features live_regression -- --nocapture --ignored
    let pending_subjects: Vec<String> = vec![];
    assert!(
        !pending_subjects.is_empty(),
        "no pending subjects — wire SemanticStore::pending_proposals_subjects()"
    );

    let mut critical = 0usize;
    let mut soft_flag = 0usize;
    let mut silent = 0usize;
    for s in &pending_subjects {
        let r = validator.validate(s);
        match r.verdict {
            SubjectVerdict::Reject => critical += 1,
            SubjectVerdict::SoftFlag => soft_flag += 1,
            _ => silent += 1,
        }
    }
    eprintln!(
        "182-regression: total={}, critical={}, soft_flag={}, silent={}",
        pending_subjects.len(),
        critical,
        soft_flag,
        silent
    );
    eprintln!(
        "unknown_frequency: {:?}",
        validator.unknown_frequency_snapshot()
    );

    // DoD §8.1 targets (within ±10%):
    assert!(
        (30..=50).contains(&critical),
        "critical={critical} outside [30,50]"
    );
    assert!(silent >= 80, "silent={silent} below 80");
}
