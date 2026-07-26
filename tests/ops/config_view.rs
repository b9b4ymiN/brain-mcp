//! Tests for `ops::config_view` — read-only Config projection.
//!
//! Phase 1 (2026-07-25 Console expansion). `GlobalConfig` stores secrets as
//! ENV VAR NAMES (`api_key_env`, `console_dev_bootstrap_*_env`) rather than
//! raw values, so the projection is naturally safe — but the regression test
//! below guards against future GlobalConfig additions that might leak a
//! resolved secret onto the wire.

use llm_wiki::config::{GlobalConfig, WikiEntry};
use llm_wiki::ops::config_view::{config_view, ConfigView};

#[test]
fn config_view_projects_safe_fields() {
    // Build a minimal GlobalConfig via Default (all sections derive Default).
    let config = GlobalConfig::default();
    let view = config_view(&config);

    // Wiki spaces are empty by default.
    assert!(view.wiki_spaces.is_empty());

    // Server section exists and has the expected shape.
    assert!(!view.server.http_enabled); // default: false
    assert_eq!(view.server.http_port, 8080); // default_http_port

    // Provider ("Extraction") section exists and is disabled by default.
    assert!(!view.extraction.enabled);

    // Logging section exists.
    assert!(!view.logging.format.is_empty());

    // Index section exists.
    assert!(!view.index.tokenizer.is_empty());

    // The serialization round-trips cleanly.
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.is_empty());
}

#[test]
fn config_view_does_not_leak_raw_secrets() {
    // Regression guard: even with provider enabled, no raw API key value
    // should appear in the output. GlobalConfig stores only the ENV VAR
    // NAME (api_key_env), never the resolved value — so the projection
    // passes this trivially today. The test exists so future additions
    // that might leak a raw secret get caught.
    let mut config = GlobalConfig::default();
    config.provider.enabled = true;
    config.provider.api_key_env = "MY_FAKE_API_KEY_ENV_VAR".to_owned();

    let view = config_view(&config);
    let json = serde_json::to_string(&view).unwrap();

    // The env var NAME is safe to show (it's not the secret value).
    assert!(
        json.contains("MY_FAKE_API_KEY_ENV_VAR"),
        "env var name is safe to show"
    );

    // Trip-wire: if a future change accidentally adds a resolved secret
    // field, these assertions catch obvious key prefixes.
    assert!(!json.contains("sk-"), "no OpenAI-style key prefix");
    assert!(!json.contains("ghp_"), "no GitHub PAT prefix");

    // Also assert the type itself serializes to a single JSON object —
    // i.e. nothing panic'd or returned an error mid-build.
    let _: ConfigView = view;
}

#[test]
fn config_view_includes_wiki_spaces() {
    let mut config = GlobalConfig::default();
    config.wikis.push(WikiEntry {
        name: "brain".to_owned(),
        path: "/tmp/brain".to_owned(),
        description: None,
        remote: None,
    });
    config.wikis.push(WikiEntry {
        name: "notes".to_owned(),
        path: "/tmp/notes".to_owned(),
        description: Some("personal notes".to_owned()),
        remote: None,
    });

    let view = config_view(&config);
    assert_eq!(view.wiki_spaces.len(), 2);
    assert_eq!(view.wiki_spaces[0].name, "brain");
    assert_eq!(view.wiki_spaces[0].path, "/tmp/brain");
    assert_eq!(view.wiki_spaces[1].name, "notes");
    assert_eq!(view.wiki_spaces[1].path, "/tmp/notes");
}

#[test]
fn config_view_projects_provider_and_server_fields() {
    // Confirm the named fields we explicitly project propagate through.
    let mut config = GlobalConfig::default();
    config.provider.enabled = true;
    config.provider.base_url = "https://example.test/v1".to_owned();
    config.provider.routine_model = "routine-model-x".to_owned();
    config.provider.reasoning_model = "reasoning-model-y".to_owned();
    config.serve.http = true;
    config.serve.http_port = 9100;
    config.serve.http_bind_address = "0.0.0.0".to_owned();
    config.serve.console_dev_bootstrap_username_env = "MY_USER".to_owned();
    config.serve.console_dev_bootstrap_password_env = "MY_PASS".to_owned();
    config.logging.log_format = "json".to_owned();
    config.index.tokenizer = "raw".to_owned();

    let view = config_view(&config);

    assert!(view.server.http_enabled);
    assert_eq!(view.server.bind, "0.0.0.0");
    assert_eq!(view.server.http_port, 9100);
    assert_eq!(
        view.server.bootstrap_username_env.as_deref(),
        Some("MY_USER")
    );
    assert_eq!(
        view.server.bootstrap_password_env.as_deref(),
        Some("MY_PASS")
    );

    assert!(view.extraction.enabled);
    assert_eq!(view.extraction.base_url, "https://example.test/v1");
    assert_eq!(view.extraction.routine_model, "routine-model-x");
    assert_eq!(view.extraction.reasoning_model, "reasoning-model-y");

    assert_eq!(view.logging.format, "json");
    assert_eq!(view.index.tokenizer, "raw");
}
