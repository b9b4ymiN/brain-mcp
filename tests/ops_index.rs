//! Unit tests for `ops::index_after_commit` — Layer 1 wrapper around
//! `WikiEngine::refresh_index`.
//!
//! Fixture pattern mirrors `tests/engine.rs::setup_wiki` (private there) so we
//! re-implement the minimal setup locally rather than reaching across test
//! binaries.

use std::fs;
use std::path::Path;

use llm_wiki::engine::WikiEngine;
use llm_wiki::git;
use llm_wiki::ops;
use llm_wiki::spaces;

/// Build a minimal wiki + config under `dir`. Returns `(config_path, repo_root)`.
///
/// Mirrors `tests/engine.rs::setup_wiki`:
///   - config lives at `<dir>/state/config.toml`
///   - repo root is `<dir>/<name>` (the wiki lives under `<dir>/<name>/wiki/`)
///   - seeds one page so the initial index has something to diff against
fn setup_wiki(dir: &Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_path = dir.join("state").join("config.toml");
    let repo_root = dir.join(name);

    spaces::create(&repo_root, name, None, false, true, &config_path, None).unwrap();

    let wiki_root = repo_root.join("wiki");
    fs::create_dir_all(wiki_root.join("concepts")).unwrap();
    fs::write(
        wiki_root.join("concepts/seed.md"),
        "---\ntitle: \"Seed\"\ntype: concept\nstatus: active\n---\n\nSeed page.\n",
    )
    .unwrap();
    git::commit(&repo_root, "seed page").unwrap();

    (config_path, repo_root)
}

fn write_page(wiki_root: &Path, rel: &str, body: &str) {
    let path = wiki_root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn index_after_commit_picks_up_new_page() {
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_wiki(dir.path(), "test");
    let wiki_root = repo_root.join("wiki");
    let manager = WikiEngine::build(&config_path).unwrap();

    // Establish a baseline index so last_commit is recorded.
    ops::index_rebuild(&manager, "test").unwrap();

    write_page(
        &wiki_root,
        "concepts/fresh.md",
        "---\ntitle: Fresh\ntype: concept\nstatus: active\n---\n\nfresh body\n",
    );
    git::commit(&repo_root, "add fresh page").unwrap();

    let report = ops::index_after_commit(&manager, "test").unwrap();
    assert!(
        report.updated >= 1,
        "expected at least 1 update, got {:?}",
        report
    );
}

#[test]
fn index_after_commit_is_safe_to_repeat() {
    // `index_manager.update` does not persist `state.toml` (only `rebuild`
    // does), so `last_commit` does not advance after an incremental update.
    // A second call may therefore re-observe the same diff. The contract we
    // care about is *safety*, not literal no-op-ness: calling repeatedly must
    // not corrupt the index or panic, and the indexed page count for the
    // changed slug must remain exactly 1 (no duplicates).
    let dir = tempfile::tempdir().unwrap();
    let (config_path, repo_root) = setup_wiki(dir.path(), "test");
    let wiki_root = repo_root.join("wiki");
    let manager = WikiEngine::build(&config_path).unwrap();
    ops::index_rebuild(&manager, "test").unwrap();

    write_page(
        &wiki_root,
        "concepts/fresh.md",
        "---\ntitle: Fresh\ntype: concept\nstatus: active\n---\n\nfresh body\n",
    );
    git::commit(&repo_root, "add fresh page").unwrap();

    let first = ops::index_after_commit(&manager, "test").unwrap();
    let _second = ops::index_after_commit(&manager, "test").unwrap();

    assert!(first.updated >= 1, "first call must pick up the new page");

    // Verify the index has exactly one doc for the fresh slug — no duplicates
    // from the repeated update.
    use llm_wiki::search::{self, SearchOptions};
    let engine = manager.state.read();
    let space = engine.space("test").unwrap();
    let searcher = space.index_manager.searcher().unwrap();
    let results = search::search(
        "Fresh",
        &SearchOptions::default(),
        &searcher,
        "test",
        &space.index_schema,
    )
    .unwrap();
    let fresh_hits = results
        .results
        .iter()
        .filter(|r| r.slug == "concepts/fresh")
        .count();
    assert_eq!(
        fresh_hits, 1,
        "repeated index_after_commit must not duplicate the page in the index"
    );
}
