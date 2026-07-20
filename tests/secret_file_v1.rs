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
//! Phase G (2026-07-20) added a parallel `*_username_env` / `*_password_env`
//! pair that resolves USERNAME/PASSWORD env vars; tests for that flow live
//! in the second half of this file.
//!
//! Field-level resolution lives on `ServeConfig::resolve_bootstrap_secret`
//! and `ServeConfig::resolve_bootstrap_credentials` (`src/config.rs`), so
//! these are unit tests — no server boot needed.

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

// ── Phase G (2026-07-20): username_env / password_env resolution ───────────
//
// These tests exercise `ServeConfig::resolve_bootstrap_credentials`. Each
// uses a unique env var name to avoid colliding with parallel test runs
// (cargo tests share a process; env mutation is visible to all threads).

// Helper: scope an env var to the test using RAII. The `unsafe` blocks are
// required by Rust 2024 edition — `set_var`/`remove_var` are unsafe because
// concurrent reads of the env from another thread can race. The Phase G
// credential resolver runs single-threaded inside these tests, and the
// unique-per-test key names prevent collisions with parallel runs.
struct EnvGuard {
    key: String,
    had_prev: bool,
    prev: String,
}

impl EnvGuard {
    fn set(key: &str, value: &str) -> Self {
        let had_prev = std::env::var(key).ok();
        let guard = EnvGuard {
            key: key.to_string(),
            had_prev: had_prev.is_some(),
            prev: had_prev.unwrap_or_default(),
        };
        // SAFETY: no other thread reads this key during the test (cargo
        // tests share a process but each test uses a unique key name).
        unsafe { std::env::set_var(key, value) };
        guard
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: see EnvGuard::set.
        unsafe {
            if self.had_prev {
                std::env::set_var(&self.key, &self.prev);
            } else {
                std::env::remove_var(&self.key);
            }
        }
    }
}

#[test]
fn password_env_overrides_legacy_secret_file() {
    let dir = tempfile::tempdir().unwrap();
    let secret_path = dir.path().join("bootstrap_secret");
    std::fs::write(&secret_path, "from-file").unwrap();

    // Unique env var name so this test is hermetic.
    let _pw = EnvGuard::set("BC_TEST_PASSWORD_G1", "from-env");
    let cfg = ServeConfig {
        console_dev_bootstrap_secret: Some("from-direct-string".into()),
        console_dev_bootstrap_secret_file: Some(secret_path),
        console_dev_bootstrap_username_env: "BC_UNUSED_G1".into(),
        console_dev_bootstrap_password_env: "BC_TEST_PASSWORD_G1".into(),
        ..Default::default()
    };

    let creds = cfg
        .resolve_bootstrap_credentials()
        .expect("resolution succeeds")
        .expect("creds present");
    // Env var wins over both legacy paths.
    assert_eq!(creds.password, "from-env");
    // Username env unset → None (legacy single-credential mode).
    assert!(creds.username.is_none());
}

#[test]
fn username_env_resolves_when_set() {
    let _pw = EnvGuard::set("BC_TEST_PASSWORD_G2", "secret-pw");
    let _user = EnvGuard::set("BC_TEST_USERNAME_G2", "console-admin");

    let cfg = ServeConfig {
        console_dev_bootstrap_username_env: "BC_TEST_USERNAME_G2".into(),
        console_dev_bootstrap_password_env: "BC_TEST_PASSWORD_G2".into(),
        ..Default::default()
    };

    let creds = cfg
        .resolve_bootstrap_credentials()
        .expect("resolution succeeds")
        .expect("creds present");
    assert_eq!(creds.username.as_deref(), Some("console-admin"));
    assert_eq!(creds.password, "secret-pw");
}

#[test]
fn empty_password_env_is_fatal_no_fallback() {
    // Operator named a password env var but it resolved empty. Fail-closed:
    // we do NOT silently fall back to the legacy secret.
    let _pw = EnvGuard::set("BC_TEST_PASSWORD_G3", "");
    let cfg = ServeConfig {
        console_dev_bootstrap_secret: Some("legacy-fallback".into()),
        console_dev_bootstrap_password_env: "BC_TEST_PASSWORD_G3".into(),
        ..Default::default()
    };
    let err = cfg.resolve_bootstrap_credentials().unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("resolved to an empty value"),
        "expected empty-value error, got: {msg}"
    );
}

#[test]
fn no_credentials_configured_returns_none() {
    // No env vars, no legacy fields → router stays unmounted.
    let cfg = ServeConfig {
        console_dev_bootstrap_username_env: "BC_UNUSED_G4".into(),
        console_dev_bootstrap_password_env: "BC_UNUSED_G4".into(),
        ..Default::default()
    };
    let creds = cfg
        .resolve_bootstrap_credentials()
        .expect("resolution succeeds");
    assert!(creds.is_none(), "no creds → None");
}

#[test]
fn legacy_secret_path_works_without_env_vars() {
    // Backward-compat: deployment that only set the legacy direct string.
    let cfg = ServeConfig {
        console_dev_bootstrap_secret: Some("just-legacy-secret".into()),
        console_dev_bootstrap_username_env: "BC_UNUSED_G5".into(),
        console_dev_bootstrap_password_env: "BC_UNUSED_G5".into(),
        ..Default::default()
    };
    let creds = cfg
        .resolve_bootstrap_credentials()
        .expect("resolution succeeds")
        .expect("creds present");
    assert_eq!(creds.password, "just-legacy-secret");
    assert!(creds.username.is_none());
}

#[test]
fn default_env_names_are_brain_namespaced() {
    // Defaults must match `.env.example` so a vanilla deployment works
    // without any TOML config. This guards against accidental drift AND
    // against re-introducing the bare USERNAME/PASSWORD collision with the
    // Windows built-in env var (found live 2026-07-20).
    let cfg = ServeConfig::default();
    assert_eq!(cfg.console_dev_bootstrap_username_env, "BRAIN_USERNAME");
    assert_eq!(cfg.console_dev_bootstrap_password_env, "BRAIN_PASSWORD");
}
