//! Phase F1.2 — `console_dev_bootstrap_secret_file` resolution contract.
//!
//! Three behaviours the Docker secrets / systemd LoadCredential wiring relies
//! on:
//!   1. **File > direct string** — when both are set, the file wins (more
//!      secure; matches the F1.2 spec).
//!   2. **Missing file → error** — fail-closed. We never silently fall back
//!      to the direct string or to an empty secret; if the operator pointed
//!      at a secret file that isn't there, the server MUST NOT start.
//!   3. **Trimmed** — a trailing newline (`echo $SECRET > file`) or other
//!      surrounding ASCII whitespace is stripped before comparison.
//!
//! Field-level resolution lives on `ServeConfig::resolve_bootstrap_secret`
//! (`src/config.rs`), so these are unit tests — no server boot needed.

use llm_wiki::config::ServeConfig;

// ── 1. file overrides direct string ──────────────────────────────────────────

#[test]
fn secret_file_overrides_direct_secret() {
    let dir = tempfile::tempdir().unwrap();
    let secret_path = dir.path().join("bootstrap_secret");
    std::fs::write(&secret_path, "from-file-secret").unwrap();

    let cfg = ServeConfig {
        console_dev_bootstrap_secret: Some("from-direct-string".into()),
        console_dev_bootstrap_secret_file: Some(secret_path.clone()),
        ..Default::default()
    };

    let resolved = cfg.resolve_bootstrap_secret().expect("resolution succeeds");
    assert_eq!(resolved.as_deref(), Some("from-file-secret"));
    assert_ne!(
        resolved.as_deref(),
        Some("from-direct-string"),
        "file MUST win over direct string"
    );
}

// ── 2. missing file fails closed ─────────────────────────────────────────────

#[test]
fn secret_file_missing_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");

    let cfg = ServeConfig {
        // Even with a direct string set, a *configured* file path that can't
        // be read must error — no silent fallback.
        console_dev_bootstrap_secret: Some("from-direct-string".into()),
        console_dev_bootstrap_secret_file: Some(missing.clone()),
        ..Default::default()
    };

    let err = cfg
        .resolve_bootstrap_secret()
        .expect_err("missing secret file must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("console_dev_bootstrap_secret_file"),
        "error should name the field, got: {msg}"
    );
}

// ── 3. trims surrounding whitespace ──────────────────────────────────────────

#[test]
fn secret_file_trims_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let secret_path = dir.path().join("bootstrap_secret");
    // Mirrors `printf 'real-secret\n' > file` (typical operator write).
    std::fs::write(&secret_path, "  real-secret\n").unwrap();

    let cfg = ServeConfig {
        console_dev_bootstrap_secret_file: Some(secret_path),
        ..Default::default()
    };

    let resolved = cfg.resolve_bootstrap_secret().expect("resolution succeeds");
    assert_eq!(
        resolved.as_deref(),
        Some("real-secret"),
        "surrounding whitespace + trailing newline must be trimmed"
    );
}

// ── 4. direct string still works when no file is set ─────────────────────────
//
// Sanity: the F1.1 path (direct string only) continues to work.

#[test]
fn direct_secret_used_when_no_file_set() {
    let cfg = ServeConfig {
        console_dev_bootstrap_secret: Some("just-a-string".into()),
        console_dev_bootstrap_secret_file: None,
        ..Default::default()
    };
    let resolved = cfg.resolve_bootstrap_secret().expect("resolution succeeds");
    assert_eq!(resolved.as_deref(), Some("just-a-string"));
}

// ── 5. neither set → None (Console API router not mounted) ───────────────────

#[test]
fn no_secret_returns_none() {
    let cfg = ServeConfig::default();
    let resolved = cfg.resolve_bootstrap_secret().expect("resolution succeeds");
    assert!(
        resolved.is_none(),
        "no secret configured → None (Console API stays unmounted)"
    );
}

// ── 6. Default impl: file field is None ──────────────────────────────────────

#[test]
fn serve_config_default_has_no_secret_file() {
    let cfg = ServeConfig::default();
    assert!(
        cfg.console_dev_bootstrap_secret_file.is_none(),
        "Default ServeConfig must not point at a secret file"
    );
    assert!(cfg.console_dev_bootstrap_secret.is_none());
}

// ── 7. file loaded end-to-end from a TOML config ─────────────────────────────
//
// Verifies the serde attribute round-trips (field name + PathBuf
// deserialization). This is what `examples/config.docker.toml` exercises in
// the compose smoke.

#[test]
fn secret_file_parses_from_toml() {
    let dir = tempfile::tempdir().unwrap();
    let secret_path = dir.path().join("bootstrap_secret");
    std::fs::write(&secret_path, "toml-loaded-secret").unwrap();

    let toml = format!(
        r#"
[serve]
console_dev_bootstrap_secret_file = "{}"
"#,
        secret_path.display().to_string().replace('\\', "\\\\")
    );

    #[derive(serde::Deserialize)]
    struct Wrapper {
        serve: ServeConfig,
    }
    let parsed: Wrapper = toml::from_str(&toml).expect("TOML parses");
    let resolved = parsed
        .serve
        .resolve_bootstrap_secret()
        .expect("resolution succeeds");
    assert_eq!(resolved.as_deref(), Some("toml-loaded-secret"));
}
