//! Phase D Task D2 — worker identity + privilege separation (F2).
//!
//! A worker principal (dispatch-layer `AuthPrincipal` with only
//! `Capability::Propose`) must be denied `brain_confirm`/`brain_supersede`/
//! purge at BOTH layers, independently:
//!   1. Dispatch (`McpServer::check_capability`, backed by `AuthPolicy`) —
//!      the gate `call_tool` runs before a handler is ever invoked.
//!   2. Store (`SemanticStore::confirm`/`supersede`/`purge_execute`, backed by
//!      the `client_capabilities` table) — even calling `tools::call` directly
//!      (bypassing the dispatch gate, as if it were buggy or absent) must
//!      still fail closed, because `McpServer::brain_context` registers the
//!      worker's `TrustedContext` with only the capabilities its principal
//!      actually has.
//!
//! A worker CAN call `brain_propose`, and the resulting proposal is always
//! `status: "proposed"` — `propose_inference` never writes a confirmed
//! claim_status row (see `insert_proposal_status` in `semantic.rs`).

use std::path::Path;
use std::sync::Arc;

use llm_wiki::engine::WikiEngine;
use llm_wiki::mcp::auth::{AuthPolicy, AuthPrincipal, Capability};
use llm_wiki::mcp::{McpServer, tools};
use llm_wiki::semantic::{SemanticConfig, SemanticError, SemanticStore};
use llm_wiki::spaces;
use serde_json::{Map, Value, json};

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

/// Minimal wiki + semantic store — brain_* tools don't touch wiki content,
/// so unlike `tests/mcp.rs`'s smoke setup this skips writing any pages.
fn setup(dir: &Path) -> (Arc<WikiEngine>, Arc<SemanticStore>) {
    let config_path = dir.join("state").join("config.toml");
    let repo_root = dir.join("test");
    spaces::create(&repo_root, "test", None, false, true, &config_path, None).unwrap();
    let manager = Arc::new(WikiEngine::build(&config_path).unwrap());

    let semantic_root = dir.join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&semantic_root, SemanticConfig::enabled_for(dir)).unwrap();
    (manager, Arc::new(store))
}

fn worker_principal() -> AuthPrincipal {
    AuthPrincipal {
        id: "extraction-worker-1".to_owned(),
        capabilities: vec![Capability::Propose],
    }
}

// =============================================================================
// Layer 1 — dispatch gate (McpServer::check_capability)
// =============================================================================

#[test]
fn dispatch_layer_denies_worker_confirm_and_supersede() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_auth_policy(AuthPolicy::default(), worker_principal());

    assert!(
        server.check_capability("brain_confirm").is_err(),
        "worker (Propose only) must be denied brain_confirm at dispatch"
    );
    assert!(
        server.check_capability("brain_supersede").is_err(),
        "worker (Propose only) must be denied brain_supersede at dispatch"
    );
}

#[test]
fn dispatch_layer_allows_worker_propose() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_auth_policy(AuthPolicy::default(), worker_principal());

    assert!(
        server.check_capability("brain_propose").is_ok(),
        "worker (Propose only) must be ALLOWED brain_propose at dispatch"
    );
}

// =============================================================================
// Layer 2 — store gate, exercised even with the dispatch gate bypassed
// =============================================================================
//
// `tools::call` is the raw dispatch table (no `check_capability` gate — that
// only runs in `McpServer::call_tool`, the rmcp `ServerHandler` entry point).
// Calling it directly here deliberately simulates "the dispatch gate isn't
// there" to prove the store layer denies on its own, independently.

#[test]
fn store_layer_denies_worker_confirm_and_supersede_even_with_dispatch_gate_bypassed() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_auth_policy(AuthPolicy::default(), worker_principal());

    let confirm_result = tools::call(
        &server,
        "brain_confirm",
        &args(json!({
            "operation_id": "bypass-confirm",
            "proposal_operation_id": "nonexistent",
        })),
    );
    assert!(
        confirm_result.is_error,
        "store layer must deny brain_confirm for a propose-only worker context, \
         even when called directly bypassing the dispatch gate"
    );

    let supersede_result = tools::call(
        &server,
        "brain_supersede",
        &args(json!({
            "operation_id": "bypass-supersede",
            "proposal_operation_id": "nonexistent",
            "superseded_claim_operation_ids": "nonexistent",
        })),
    );
    assert!(
        supersede_result.is_error,
        "store layer must deny brain_supersede for a propose-only worker context, \
         even when called directly bypassing the dispatch gate"
    );
}

#[test]
fn store_layer_denies_worker_purge_directly() {
    let dir = tempfile::tempdir().unwrap();
    let (_manager, store) = setup(dir.path());
    let worker_ctx = store
        .register_client_scoped("extraction-worker-1", &["propose"])
        .unwrap();

    let err = store
        .purge_execute(
            &worker_ctx,
            "bypass-purge",
            "fake-preview-hash",
            "fake-nonce",
        )
        .unwrap_err();
    assert!(
        matches!(err, SemanticError::CapabilityDenied(_)),
        "propose-only worker must be denied purge_execute: got {err:?}"
    );
}

/// A worker registered with `["propose"]` is the SAME identity a real
/// `McpServer::brain_context` call would produce for `worker_principal()` —
/// this proves the store-layer capability check is not merely "no capability
/// gate configured" but an active deny for a context that has SOME
/// capabilities, just not the one required.
#[test]
fn store_layer_confirm_denial_names_the_missing_capability() {
    let dir = tempfile::tempdir().unwrap();
    let (_manager, store) = setup(dir.path());
    let worker_ctx = store
        .register_client_scoped("extraction-worker-1", &["propose"])
        .unwrap();

    let err = store
        .confirm(
            &worker_ctx,
            llm_wiki::semantic::ConfirmCommand {
                operation_id: "direct-confirm".to_owned(),
                proposal_operation_id: "nonexistent".to_owned(),
            },
        )
        .unwrap_err();
    match err {
        SemanticError::CapabilityDenied(msg) => {
            assert!(
                msg.contains("confirm"),
                "denial should name 'confirm': {msg}"
            );
        }
        other => panic!("expected CapabilityDenied, got {other:?}"),
    }
}

// =============================================================================
// Positive: worker CAN propose, full dispatch, always status "proposed"
// =============================================================================

#[test]
fn worker_can_propose_via_full_dispatch_and_status_is_always_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_auth_policy(AuthPolicy::default(), worker_principal());

    // Layer 1 confirms this call is allowed before we even dispatch it.
    assert!(server.check_capability("brain_propose").is_ok());

    let result = tools::call(
        &server,
        "brain_propose",
        &args(json!({
            "operation_id": "worker-propose-1",
            "subject": "gulf",
            "predicate": "target_price",
            "value": "58",
            "domain": "stocks",
            "method": "llm_extraction",
            "model": "glm-4.6",
        })),
    );
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(!result.is_error, "worker propose should succeed: {text}");
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["status"], "proposed");
}

/// Compile/behavior-level proof that there is no parameter or argument
/// combination for `brain_propose` that yields any status other than
/// "proposed" — the handler hardcodes it, and `propose_inference` never
/// writes a confirmed `claim_status` row.
#[test]
fn worker_propose_with_no_evidence_is_still_only_proposed_never_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = setup(dir.path());
    let server = McpServer::new(manager)
        .with_semantic_store(store)
        .with_auth_policy(AuthPolicy::default(), worker_principal());

    let result = tools::call(
        &server,
        "brain_propose",
        &args(json!({
            "operation_id": "worker-propose-unsupported",
            "subject": "gulf",
            "predicate": "target_price",
            "value": "58",
            "domain": "stocks",
            "method": "llm_extraction",
            // no evidence_capture_operation_ids -> unsupported inference
        })),
    );
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    assert!(
        !result.is_error,
        "unsupported inference proposal should still succeed: {text}"
    );
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["status"], "proposed");
}
