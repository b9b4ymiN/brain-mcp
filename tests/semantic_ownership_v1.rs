//! Task 2.2 — Human/agent/generated content ownership (RED stage).
//!
//! These tests encode the behaviour mandated by GOAL-vNext §13 Task 2.2 DoD:
//!
//! 1. file ownership เป็น `human-authored`, `agent-proposed`, `generated`
//! 2. human edit กลายเป็น authored event; generated edit ไม่เปลี่ยน state เงียบ
//! 3. entity rename/merge รักษา stable IDs และ backlinks
//!
//! They also close the provenance gap left open since Task 1.2: ADR Decision 6
//! names four provenance variants (`evidence`, `user_assertion`, `mechanical`,
//! `inference`) but only two were implemented. Task 2.2 adds the remaining two
//! and exposes provenance/origin on `ClaimView` so the projection layer can
//! distinguish human-authored from agent-proposed claims.
//!
//! This file is the RED checkpoint: every test compiles against the *target*
//! API (new commands, methods, and `ClaimView` fields) and therefore fails to
//! compile until `src/semantic.rs` implements them. That compile failure is the
//! intended RED state per GOAL-vNext §12.2.3.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, OriginClass, PrivacyLabel, ProposeCommand,
    ProposeInferenceCommand, SemanticConfig, SemanticError, SemanticStore, TrustedContext,
};
use serde_json::json;
use tempfile::TempDir;

fn enabled(parent: &Path) -> SemanticConfig {
    SemanticConfig::enabled_for(parent)
}

fn fixture() -> (TempDir, PathBuf, SemanticStore, TrustedContext) {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    let context = store.trusted_context();
    (parent, root, store, context)
}

fn capture(operation_id: &str, bytes: &[u8]) -> CaptureCommand {
    CaptureCommand {
        operation_id: operation_id.to_owned(),
        bytes: bytes.to_vec(),
        media_type: "text/plain; charset=utf-8".to_owned(),
    }
}

/// A user-assertion draft: a preference the owner stated directly (no external
/// evidence span required).
fn preference_draft(subject: &str, value: &str) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "preference".to_owned(),
        value: json!(value),
        claim_kind: "preference".to_owned(),
        domain: Some("projects".to_owned()),
        confidence_basis_points: 9_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn stock_draft(subject: &str, value: i64) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: "target_price".to_owned(),
        value: json!(value),
        claim_kind: "external_fact".to_owned(),
        domain: Some("stocks".to_owned()),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

fn at(value: &str) -> DateTime<Utc> {
    value.parse().expect("RFC 3339 test timestamp")
}

// =============================================================================
// Provenance gap — UserAssertion + Mechanical variants (ADR Decision 6)
// =============================================================================

/// A human-authored claim flows through `propose_user_assertion` + `confirm`,
/// producing a `claim_confirmed` event whose provenance is `user_assertion` —
/// NOT `evidence`. ADR Decision 6 mandates this as the path for "human edit
/// becomes an authored event" (Task 2.2 DoD bullet 2).
#[test]
fn user_assertion_propose_and_confirm_produces_a_user_assertion_claim() {
    let (_parent, _root, store, context) = fixture();

    let outcome = store
        .propose_user_assertion(
            &context,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "assert-1".to_owned(),
                utterance: b"use docker compose for deployment".to_vec(),
                draft: preference_draft("project:brain", "docker-compose"),
            },
        )
        .expect("propose_user_assertion");

    assert_eq!(outcome.event.event_type, "claim_proposed");
    assert!(outcome.generated.proposal_id.is_some());

    let confirmed = store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-1".to_owned(),
                proposal_operation_id: "assert-1".to_owned(),
            },
        )
        .expect("confirm");

    assert_eq!(confirmed.event.event_type, "claim_confirmed");
    let claim_id = confirmed.generated.claim_id.expect("claim id");

    let view = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "projects",
            "project:brain",
            "preference",
        )
        .expect("claims_current");
    assert_eq!(view.active.len(), 1);
    assert_eq!(view.active[0].claim_id, claim_id);
    // DoD 1 — origin classification
    assert_eq!(view.active[0].provenance_kind, "user_assertion");
    assert_eq!(view.active[0].origin, OriginClass::HumanAuthored);
}

/// A user assertion must not require an evidence byte span the way
/// evidence-backed proposals do. The utterance is the evidence.
#[test]
fn user_assertion_proposal_does_not_require_a_prior_capture() {
    let (_parent, _root, store, context) = fixture();

    let result = store.propose_user_assertion(
        &context,
        llm_wiki::semantic::ProposeUserAssertionCommand {
            operation_id: "assert-no-cap".to_owned(),
            utterance: b"prefer tabs over spaces".to_vec(),
            draft: preference_draft("editor", "tabs"),
        },
    );
    assert!(
        result.is_ok(),
        "user assertion needs no capture: {result:?}"
    );
}

/// Mechanically-derived metadata (hash/title/time) is its own provenance kind,
/// distinct from both evidence and inference. ADR Decision 6 names it
/// `mechanical` and §5 Memory Policy allows auto-confirm for "metadata ที่
/// ตรวจเชิงกลไกได้". Task 2.2 introduces the variant; confirm still follows the
/// normal path.
#[test]
fn mechanical_provenance_is_a_distinct_kind() {
    let (_parent, _root, store, context) = fixture();

    let outcome = store
        .propose_mechanical(
            &context,
            llm_wiki::semantic::ProposeMechanicalCommand {
                operation_id: "mech-1".to_owned(),
                method: "sha256_title_extract".to_owned(),
                method_version: "1.0.0".to_owned(),
                input_hashes: vec!["sha256:abc".to_owned()],
                output_hash: "sha256:def".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .expect("propose_mechanical");

    assert_eq!(outcome.event.event_type, "claim_proposed");

    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-mech".to_owned(),
                proposal_operation_id: "mech-1".to_owned(),
            },
        )
        .expect("confirm");

    let view = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .expect("claims_current");
    assert_eq!(view.active.len(), 1);
    assert_eq!(view.active[0].provenance_kind, "mechanical");
    assert_eq!(view.active[0].origin, OriginClass::HumanAuthored);
}

/// Inference-derived claims stay `AgentProposed` even after confirmation — the
/// provenance is preserved so the projection can always tell an AI-authored
/// claim apart from a human-authored one. This is the load-bearing distinction
/// for Task 2.2 DoD bullet 1 (`agent-proposed`).
#[test]
fn inference_claim_origin_is_agent_proposed_even_after_confirm() {
    let (_parent, _root, store, context) = fixture();

    store
        .propose_inference(
            &context,
            ProposeInferenceCommand {
                operation_id: "infer-1".to_owned(),
                evidence_capture_operation_ids: vec![],
                method: "llm_extract".to_owned(),
                model: Some("zai-glm".to_owned()),
                prompt_version: Some("v1".to_owned()),
                subject_validator_version: None,
                draft: stock_draft("GULF", 60),
            },
        )
        .expect("propose_inference");

    // Unsupported inference cannot be confirmed (Task 1.2 gate, TM-002).
    assert!(matches!(
        store.confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-infer".to_owned(),
                proposal_operation_id: "infer-1".to_owned(),
            },
        ),
        Err(SemanticError::UnsupportedInference)
    ));
}

/// Evidence-backed claims (the original path from Task 0.3) keep their
/// provenance kind. They are not `human-authored` in the Task 2.2 origin sense
/// — an external source is evidence, not an owner assertion. Origin therefore
/// stays `AgentProposed` (the source was ingested, not authored by the owner).
#[test]
fn evidence_backed_claim_origin_is_agent_proposed() {
    let (_parent, _root, store, context) = fixture();
    store
        .capture(&context, capture("cap", b"analyst note bytes"))
        .unwrap();
    store
        .propose(
            &context,
            ProposeCommand {
                operation_id: "prop".to_owned(),
                capture_operation_id: "cap".to_owned(),
                draft: stock_draft("GULF", 58),
            },
        )
        .unwrap();
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm".to_owned(),
                proposal_operation_id: "prop".to_owned(),
            },
        )
        .unwrap();

    let view = store
        .claims_current(
            store.ledger_head().unwrap(),
            Utc::now(),
            "stocks",
            "GULF",
            "target_price",
        )
        .unwrap();
    assert_eq!(view.active.len(), 1);
    assert_eq!(view.active[0].provenance_kind, "evidence");
    assert_eq!(view.active[0].origin, OriginClass::AgentProposed);
}

// =============================================================================
// Entity model — stable IDs, rename, merge, backlinks (DoD bullet 3)
// =============================================================================

/// `resolve_or_create_entity` is idempotent for the same (domain, subject):
/// the first call mints a UUIDv7 entity_id, the second returns the same id.
/// This is the "stable ID" invariant from ADR Decision 3.
#[test]
fn resolve_or_create_entity_is_idempotent_per_domain_subject() {
    let (_parent, _root, store, context) = fixture();

    let first = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("first resolve");
    let second = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("second resolve");
    assert_eq!(
        first, second,
        "same (domain, subject) must resolve to one entity"
    );

    // A different subject in the same domain is a different entity.
    let other = store
        .resolve_or_create_entity(&context, "PTT")
        .expect("other resolve");
    assert_ne!(first, other);

    // Same subject string in a DIFFERENT domain is also a different entity —
    // domain is part of the entity scope key.
    let cross = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("cross-domain resolve");
    assert_ne!(first, cross);
}

/// `rename_entity` updates the canonical subject but preserves the entity_id.
/// The old subject string remains resolvable as a backlink (alias), so claims
/// that still reference the old name continue to resolve to the same entity.
/// This is the "rename preserves stable IDs and backlinks" DoD.
#[test]
fn rename_entity_preserves_id_and_keeps_old_subject_as_backlink() {
    let (_parent, _root, store, context) = fixture();

    let entity_id = store
        .resolve_or_create_entity(&context, "GULF-old")
        .expect("resolve");

    store
        .rename_entity(
            &context,
            llm_wiki::semantic::RenameEntityCommand {
                operation_id: "rename-1".to_owned(),
                entity_id,
                new_subject: "GULF".to_owned(),
            },
        )
        .expect("rename");

    // Canonical subject is now the new name.
    let after = store
        .entity_by_id(&context, entity_id)
        .expect("entity_by_id");
    assert_eq!(after.canonical_subject, "GULF");
    assert_eq!(after.entity_id, entity_id);

    // The OLD subject still resolves to the same entity_id — backlink preserved.
    let backlink = store
        .resolve_entity(&context, "GULF-old")
        .expect("resolve old subject");
    assert_eq!(backlink, entity_id);

    // The NEW subject also resolves.
    let direct = store
        .resolve_entity(&context, "GULF")
        .expect("resolve new subject");
    assert_eq!(direct, entity_id);
}

/// `merge_entities` moves every claim attached to the source entity onto the
/// target entity, and turns the source's subject (and any prior aliases) into
/// backlinks pointing at the target. The source entity_id is no longer
/// canonical, but no claim loses its entity reference. This is the "merge
/// preserves stable IDs and backlinks" DoD.
#[test]
fn merge_entities_moves_claims_and_keeps_aliases_as_backlinks() {
    let (_parent, _root, store, context) = fixture();

    // Two entities, each with one confirmed claim.
    let source = store
        .resolve_or_create_entity(&context, "GULF-dup")
        .expect("source entity");
    let target = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("target entity");

    store
        .propose_user_assertion(
            &context,
            llm_wiki::semantic::ProposeUserAssertionCommand {
                operation_id: "assert-source".to_owned(),
                utterance: b"source claim".to_vec(),
                draft: ClaimDraft {
                    subject: "GULF-dup".to_owned(),
                    predicate: "target_price".to_owned(),
                    value: json!(58),
                    claim_kind: "user_assertion".to_owned(),
                    domain: Some("stocks".to_owned()),
                    confidence_basis_points: 9_000,
                    privacy_label: PrivacyLabel::LocalOnly,
                    valid_from: None,
                    valid_to: None,
                },
            },
        )
        .expect("propose source");
    store
        .confirm(
            &context,
            ConfirmCommand {
                operation_id: "confirm-source".to_owned(),
                proposal_operation_id: "assert-source".to_owned(),
            },
        )
        .expect("confirm source");

    store
        .merge_entities(
            &context,
            llm_wiki::semantic::MergeEntitiesCommand {
                operation_id: "merge-1".to_owned(),
                source_entity_id: source,
                target_entity_id: target,
            },
        )
        .expect("merge");

    // The source subject now resolves to the TARGET entity (backlink).
    let resolved = store
        .resolve_entity(&context, "GULF-dup")
        .expect("resolve merged-away subject");
    assert_eq!(resolved, target);

    // The claim's entity_id was rewritten to the target (no orphan claims).
    let claims = store
        .claims_for_entity(&context, target)
        .expect("claims_for_entity");
    assert!(
        claims.iter().any(|c| c.subject == "GULF-dup"),
        "merged claim must remain reachable via target entity"
    );
    let source_claims = store
        .claims_for_entity(&context, source)
        .expect("source claims");
    assert!(
        source_claims.is_empty(),
        "source entity must hold no claims after merge"
    );
}

/// Merging an entity into itself is a no-op error, not a silent success — it
/// would otherwise delete the only canonical subject without a target.
#[test]
fn merge_entity_into_itself_is_rejected() {
    let (_parent, _root, store, context) = fixture();
    let entity = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("resolve");

    let result = store.merge_entities(
        &context,
        llm_wiki::semantic::MergeEntitiesCommand {
            operation_id: "self-merge".to_owned(),
            source_entity_id: entity,
            target_entity_id: entity,
        },
    );
    assert!(
        matches!(result, Err(SemanticError::InvalidTransition(_))),
        "self-merge must be rejected: {result:?}"
    );
}

/// Renaming to a subject that already exists as a *different* entity in the
/// same domain is rejected — that is a merge, not a rename, and silently
/// aliasing two distinct entities onto one id would lose the distinction.
#[test]
fn rename_to_an_existing_subject_in_same_domain_is_rejected() {
    let (_parent, _root, store, context) = fixture();
    let a = store
        .resolve_or_create_entity(&context, "GULF")
        .expect("a");
    let _b = store
        .resolve_or_create_entity(&context, "PTT")
        .expect("b");

    let result = store.rename_entity(
        &context,
        llm_wiki::semantic::RenameEntityCommand {
            operation_id: "rename-collision".to_owned(),
            entity_id: a,
            new_subject: "PTT".to_owned(),
        },
    );
    assert!(
        matches!(result, Err(SemanticError::InvalidTransition(_))),
        "rename onto another entity must be rejected: {result:?}"
    );
}

// =============================================================================
// Schema version gate — fail closed on a v1 store (DoD: no silent open)
// =============================================================================

/// A store created at schema_version 1 must refuse to open once the binary
/// expects schema_version 2. This prevents a stale store from silently running
/// against a DDL it was not initialised with (no migration path exists yet —
/// pre-production break, recorded like Task 1.1's clients-table precedent).
///
/// We craft a v1 marker on disk by writing the marker JSON directly with
/// schema_version: 1, then attempt `SemanticStore::open`. The open must fail
/// closed, not silently downgrade/upgrade.
#[test]
fn opening_a_schema_v1_store_under_a_v2_binary_fails_closed() {
    let parent = tempfile::tempdir().expect("fixture parent");
    let root = parent.path().join("semantic-store");

    // Create a legitimate v2 store first (so the schema, tables, and marker
    // are all valid under the current binary), then tamper ONLY the marker's
    // schema_version back to 1 to simulate a legacy store.
    let (store, _admin) = SemanticStore::create(&root, enabled(parent.path())).expect("create");
    drop(store);

    let marker_path = root.join("store.marker.json");
    let mut marker: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&marker_path).unwrap()).unwrap();
    marker["schema_version"] = json!(1);
    std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();

    let result = SemanticStore::open(&root, enabled(parent.path()));
    assert!(
        result.is_err(),
        "a schema-v1 marker must fail closed under a v2 binary, not silently open"
    );
}

// =============================================================================
// Idempotency — rename/merge honour operation_id replay (§4 rule 6)
// =============================================================================

/// Replaying the same rename operation_id returns the original outcome without
/// creating a second alias row or mutating the entity again. Mirrors the
/// ledger's existing idempotency contract for capture/propose/confirm.
#[test]
fn rename_entity_is_idempotent_under_operation_id_replay() {
    let (_parent, _root, store, context) = fixture();
    let entity = store
        .resolve_or_create_entity(&context, "GULF-old")
        .expect("resolve");

    let first = store
        .rename_entity(
            &context,
            llm_wiki::semantic::RenameEntityCommand {
                operation_id: "rename-idem".to_owned(),
                entity_id: entity,
                new_subject: "GULF".to_owned(),
            },
        )
        .expect("first rename");

    let replay = store
        .rename_entity(
            &context,
            llm_wiki::semantic::RenameEntityCommand {
                operation_id: "rename-idem".to_owned(),
                entity_id: entity,
                new_subject: "GULF".to_owned(),
            },
        )
        .expect("replay rename");

    assert_eq!(first.event.event_seq, replay.event.event_seq);
    assert_eq!(
        store.resolve_entity(&context, "GULF").unwrap(),
        entity
    );
    assert_eq!(
        store
            .resolve_entity(&context, "GULF-old")
            .unwrap(),
        entity
    );
}

/// `Utc::now()` placeholder import so the helper compiles even if no test in
/// this file uses it yet. Keeps the module self-contained.
#[allow(dead_code)]
fn _ensure_at_helper_compiles() -> DateTime<Utc> {
    at("2026-07-15T00:00:00Z")
}
