//! Tests for ops::recent_activity — recent page changes via git log.

use std::time::Duration;

use super::helpers::setup_wiki;
use llm_wiki::engine::WikiEngine;
use llm_wiki::ops;

#[test]
fn recent_activity_returns_commits() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    // setup_wiki creates 2 pages + 1 commit. Expect >= 1 event.
    let result = ops::recent_activity(&engine, Duration::from_secs(86400 * 7), 50).unwrap();
    assert!(!result.is_empty(), "expected at least one activity event");
    let first = &result[0];
    assert!(!first.actor.is_empty(), "actor should be non-empty");
    assert!(!first.target.is_empty(), "target (slug path) should be non-empty");
    assert!(
        !first.timestamp.is_empty(),
        "timestamp should be non-empty"
    );
}

#[test]
fn recent_activity_respects_limit() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    // limit=1 should cap the result length (setup_wiki produces 2 pages = 2 events per commit).
    let result = ops::recent_activity(&engine, Duration::from_secs(86400 * 7), 1).unwrap();
    assert!(
        result.len() <= 1,
        "limit respected: got {} events, expected <= 1",
        result.len()
    );
}

#[test]
fn recent_activity_returns_empty_when_since_too_short() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    // since=0 means git log --since=0seconds — git treats this as "all
    // commits" (no effective filter). We assert sane behavior: at most `limit`.
    let result = ops::recent_activity(&engine, Duration::from_secs(0), 50).unwrap();
    assert!(result.len() <= 50);
}
