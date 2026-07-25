use super::helpers::setup_wiki;
use llm_wiki::engine::WikiEngine;
use llm_wiki::ops;

// ── Content ───────────────────────────────────────────────────────────────────

#[test]
fn content_read_page() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    match ops::content_read(&engine, "concepts/moe", None, false, false).unwrap() {
        ops::ContentReadResult::Page(content) => {
            assert!(content.contains("Mixture of Experts"));
        }
        _ => panic!("expected Page"),
    }
}

#[test]
fn content_read_no_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    match ops::content_read(&engine, "concepts/moe", None, true, false).unwrap() {
        ops::ContentReadResult::Page(content) => {
            assert!(!content.contains("title:"));
            assert!(content.contains("Mixture of Experts"));
        }
        _ => panic!("expected Page"),
    }
}

#[test]
fn content_read_bare_slug_falls_back_to_blueprint_sections() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    match ops::content_read(&engine, "moe", None, true, false).unwrap() {
        ops::ContentReadResult::Page(content) => {
            assert!(content.contains("Mixture of Experts"));
        }
        _ => panic!("expected Page"),
    }
}

#[test]
fn content_write_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let body = "---\ntitle: \"New\"\ntype: page\n---\n\nHello.\n";
    let result =
        ops::content_write(&engine, &manager, "new-page", None, body, false, false).unwrap();
    assert_eq!(result.bytes_written, body.len());

    match ops::content_read(&engine, "new-page", None, false, false).unwrap() {
        ops::ContentReadResult::Page(content) => assert!(content.contains("Hello.")),
        _ => panic!("expected Page"),
    }
}

#[test]
fn content_commit_rehomes_bare_concept_slug() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();
    let space = engine.space("test").unwrap();

    let root_file = space.wiki_root.join("thai-tts-voice-cloning.md");
    std::fs::write(
        &root_file,
        "---\ntitle: \"Thai TTS\"\ntype: concept\nstatus: active\n---\n\nBody.\n",
    )
    .unwrap();

    let hash = ops::content_commit(
        &engine,
        "test",
        &["thai-tts-voice-cloning".to_string()],
        false,
        Some("commit thai tts"),
    )
    .unwrap();

    assert!(!hash.is_empty());
    assert!(!root_file.exists());
    assert!(
        space
            .wiki_root
            .join("concepts/thai-tts-voice-cloning.md")
            .exists()
    );
}

#[test]
fn content_new_page() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let result = ops::content_new(
        &engine,
        "concepts/new-concept",
        None,
        false,
        false,
        None,
        None,
    )
    .unwrap();
    assert!(result.uri.starts_with("wiki://test/concepts/new-concept"));
    assert_eq!(result.slug, "concepts/new-concept");
    assert!(!result.bundle);
    assert!(result.path.exists());
    assert!(result.path.to_string_lossy().ends_with(".md"));
}

#[test]
fn content_new_section() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let result = ops::content_new(&engine, "topics", None, true, false, None, None).unwrap();
    assert!(result.uri.contains("topics"));
}

#[test]
fn content_new_bundle_result_has_path_and_wiki_root() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let result =
        ops::content_new(&engine, "concepts/bundled", None, false, true, None, None).unwrap();
    assert!(result.bundle);
    assert!(result.path.ends_with("index.md"));
    assert!(result.path.exists());
    assert!(result.wiki_root.is_dir());
}

#[test]
fn content_commit_all() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    // Write a new file so there's something to commit
    ops::content_write(
        &engine,
        &manager,
        "scratch",
        None,
        "---\ntitle: \"Scratch\"\ntype: page\n---\n\ntemp\n",
        false,
        false,
    )
    .unwrap();

    let hash = ops::content_commit(&engine, "test", &[], true, Some("test commit")).unwrap();
    assert!(!hash.is_empty());
}

#[test]
fn content_write_commit_true_returns_commit_sha_and_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let body = "---\ntitle: \"Commit Me\"\ntype: concept\nstatus: active\n---\n\nBody.\n";
    let result = ops::content_write(
        &engine, &manager, "commit-me", None, body, /* commit */ true, /* redact */ false,
    )
    .unwrap();

    let sha = result.commit_sha.expect("commit_sha should be Some when commit=true");
    assert!(!sha.is_empty(), "commit_sha should not be empty for a fresh page");

    let report = result.index_report.expect("index_report should be Some when commit=true");
    assert!(report.updated >= 1, "index_report.updated should be >= 1");

    // The page must exist in git history.
    let space = engine.space("test").unwrap();
    let head = std::process::Command::new("git")
        .args(["-C", space.repo_root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success(), "git rev-parse HEAD failed");
}

#[test]
fn content_write_commit_false_leaves_file_uncommitted_and_unindexed() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let body = "---\ntitle: \"No Commit\"\ntype: concept\nstatus: active\n---\n\nBody.\n";
    let result = ops::content_write(
        &engine, &manager, "no-commit", None, body, /* commit */ false, /* redact */ false,
    )
    .unwrap();

    assert!(result.commit_sha.is_none(), "commit_sha should be None when commit=false");
    assert!(result.index_report.is_none(), "index_report should be None when commit=false");
    assert!(result.path.exists(), "file should still be written to disk");

    // The page should appear as untracked / unstaged in git status.
    let space = engine.space("test").unwrap();
    let status = std::process::Command::new("git")
        .args(["-C", space.repo_root.to_str().unwrap(), "status", "--porcelain"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        stdout.contains("no-commit") || stdout.contains("concepts/no-commit"),
        "file should show up in git status as uncommitted; got: {stdout}"
    );
}

#[test]
fn content_write_commit_true_redact_true_replaces_known_secret_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    // AKIAIOSFODNN7EXAMPLE matches the built-in `aws-access-key` pattern
    // (AKIA + 16 base32 chars), verified by tests/redact.rs::aws_access_key_is_redacted.
    // The default test wiki has no `[redact]` section, so RedactConfig::default()
    // applies — all built-ins active.
    let body = "---\ntitle: \"Has Secret\"\ntype: concept\nstatus: active\n---\n\nMy key is AKIAIOSFODNN7EXAMPLE.\n";
    let result = ops::content_write(
        &engine, &manager, "has-secret", None, body, /* commit */ true, /* redact */ true,
    )
    .unwrap();

    // The committed file on disk should no longer contain the raw secret.
    let committed = std::fs::read_to_string(&result.path).unwrap();
    assert!(
        !committed.contains("AKIAIOSFODNN7EXAMPLE"),
        "raw secret should be redacted in the committed file; got: {committed}"
    );

    let sha = result.commit_sha.expect("commit_sha should be Some");
    assert!(!sha.is_empty());
}

#[test]
fn content_write_commit_true_forces_commit_when_auto_commit_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");

    // Disable auto_commit in the wiki.toml that setup_wiki created.
    // setup_wiki writes to dir/test/wiki.toml via generate_wiki_toml, which by
    // default emits only `name = "..."` (no `[ingest]` section), so we always
    // append a fresh `[ingest]` block.
    let wiki_toml = dir.path().join("test").join("wiki.toml");
    let existing = std::fs::read_to_string(&wiki_toml).unwrap_or_default();
    let updated = if existing.contains("[ingest]") {
        format!("{existing}\nauto_commit = false\n")
    } else {
        format!("{existing}\n[ingest]\nauto_commit = false\n")
    };
    std::fs::write(&wiki_toml, updated).unwrap();

    let manager = WikiEngine::build(&config_path).unwrap();
    let engine = manager.state.read();

    let body = "---\ntitle: \"Forced\"\ntype: concept\nstatus: active\n---\n\nBody.\n";
    let result = ops::content_write(
        &engine, &manager, "forced", None, body, /* commit */ true, /* redact */ false,
    )
    .unwrap();

    let sha = result
        .commit_sha
        .expect("commit_sha should be Some even when auto_commit=false (fallback fires)");
    assert!(!sha.is_empty(), "fallback commit should produce a non-empty sha");
}
