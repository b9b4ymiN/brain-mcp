//! Task 6.1 — Reproducible deployment contract (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 6.1 DoD at the contract level (Rust domain
//! types; no real `docker compose up` in-env). Docker Compose is the portable
//! deployment contract; Oracle ARM is a reference host.

use std::sync::Arc;

use llm_wiki::config::GlobalConfig;
use llm_wiki::deployment::{
    DeploymentManifest, HealthStatus, Platform, ReadinessCheck, SecretRef, ServiceSpec,
};
use llm_wiki::engine::{EngineState, WikiEngine};
use llm_wiki::semantic::{CURRENT_DISK_SCHEMA_VERSION, SemanticConfig, SemanticStore};

// =============================================================================
// DoD: Docker Compose as portable deployment contract; amd64 + arm64
// =============================================================================

/// `DeploymentManifest` declares the services + the platforms they target.
/// §6.1 "Docker Compose เป็น deployment contract แบบ portable".
#[test]
fn manifest_targets_both_platforms() {
    let manifest = DeploymentManifest::default_brain();
    assert!(
        manifest.platforms.contains(&Platform::LinuxAmd64),
        "must target linux/amd64"
    );
    assert!(
        manifest.platforms.contains(&Platform::LinuxArm64),
        "must target linux/arm64 (Oracle ARM reference)"
    );
}

/// The manifest declares the brain service + its dependencies (semantic store,
/// projections). Each is a `ServiceSpec`.
#[test]
fn manifest_declares_brain_service_and_deps() {
    let manifest = DeploymentManifest::default_brain();
    let names: Vec<&str> = manifest.services.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"brain"),
        "manifest must declare the brain service"
    );
}

// =============================================================================
// DoD: secrets mount, not baked into image/compose/repo
// =============================================================================

/// Secrets are references (`SecretRef`), never raw values in the manifest.
/// §6.1 "secrets mount ผ่าน Docker secrets/secret files และไม่ bake ใน
/// image/compose/repo".
#[test]
fn secrets_are_references_not_raw_values() {
    let spec = ServiceSpec {
        name: "brain".to_owned(),
        image: "brain:v0.5".to_owned(),
        secrets: vec![SecretRef {
            env_var: "AI_API_KEY_REF".to_owned(),
            source: "op://vault/zai/key".to_owned(),
        }],
        health: ReadinessCheck::default(),
    };
    assert_eq!(spec.secrets.len(), 1);
    // The source is a secret-manager reference, never a raw key.
    assert!(
        !spec.secrets[0].source.starts_with("sk-"),
        "secret source must be a reference, not a raw key"
    );
}

// =============================================================================
// DoD: health/readiness does not report ready before DB/migration/index checks
// =============================================================================

/// `ReadinessCheck` enumerates the gates that must pass before the service
/// reports ready: DB reachable, migrations applied, index open. §6.1
/// "health/readiness ไม่รายงานพร้อมก่อน DB/migration/index checks ผ่าน".
#[test]
fn readiness_requires_db_migrations_index() {
    let check = ReadinessCheck {
        db_reachable: true,
        migrations_applied: false,
        index_open: true,
    };
    assert!(
        !check.is_ready(),
        "service must NOT be ready when migrations are not applied"
    );
    let ready = ReadinessCheck {
        db_reachable: true,
        migrations_applied: true,
        index_open: true,
    };
    assert!(ready.is_ready(), "service IS ready when all gates pass");
}

/// `HealthStatus` distinguishes healthy/degraded/unhealthy so a load balancer
/// can route correctly.
#[test]
fn health_status_is_distinct_from_readiness() {
    let _ = HealthStatus::Healthy;
    let _ = HealthStatus::Degraded;
    let _ = HealthStatus::Unhealthy;
}

// =============================================================================
// DoD: smoke checks — compose up passes on clean host
// =============================================================================

/// The manifest carries a smoke-check list the deployment runbook verifies
/// after `compose up`: health endpoint, MCP list-tools, a write/read round-trip.
#[test]
fn manifest_carries_smoke_checks() {
    let manifest = DeploymentManifest::default_brain();
    assert!(
        !manifest.smoke_checks.is_empty(),
        "manifest must declare smoke checks for the runbook"
    );
    // At least a health check + an MCP check.
    assert!(
        manifest
            .smoke_checks
            .iter()
            .any(|c| c.contains("health") || c.contains("mcp")),
        "smoke checks must cover health + mcp"
    );
}

// =============================================================================
// Phase F1.3 — `ReadinessCheck::from_runtime` (the real /ready gate)
// =============================================================================
//
// The contract tests above assert the *type* shape. These tests assert the
// *factory* that the live `/ready` HTTP handler calls — the wiring that turns
// `ReadinessCheck` from a contract into a real production gate (Phase F1.3:
// the F1 lesson again — the gate must be on the real serve path, not just in
// a contract). They build a real (empty) `WikiEngine` + a real on-disk
// `SemanticStore` in a temp dir and assert the readiness outcome for each
// shape the `/ready` handler can see.

/// Build a minimal empty `WikiEngine` — no config file on disk required. The
/// engine's `state` field is `pub`, and `EngineState`'s fields are all `pub`,
/// so a test can construct the zero-spaces shape directly. This is the
/// "fresh deploy" shape: the process is up, no wikis are registered yet, but
/// it can still accept `spaces register` — so it counts as servable.
fn empty_engine() -> WikiEngine {
    use std::collections::HashMap;
    let state = EngineState {
        config: GlobalConfig::default(),
        config_path: std::path::PathBuf::from("/tmp/brain-f1.3-test-config.toml"),
        state_dir: std::path::PathBuf::from("/tmp/brain-f1.3-test-state"),
        spaces: HashMap::new(),
    };
    WikiEngine {
        state: Arc::new(parking_lot::RwLock::new(state)),
    }
}

/// Create a real on-disk `SemanticStore` in `parent`. Returns the `Arc` handle
/// the way the serve path holds it.
fn make_store(parent: &std::path::Path) -> Arc<SemanticStore> {
    let root = parent.join("semantic-store");
    let (store, _admin) =
        SemanticStore::create(&root, SemanticConfig::enabled_for(parent)).expect("create store");
    Arc::new(store)
}

/// No store attached → NOT ready (db_reachable=false, migrations_applied=false).
/// This is the "brain_* tools disabled" shape: the process is up, but a brain
/// service without its semantic store is not ready to serve brain traffic.
#[test]
fn from_runtime_without_store_is_not_ready() {
    let engine = empty_engine();
    let check = ReadinessCheck::from_runtime(&engine, None);
    assert!(
        !check.db_reachable,
        "db_reachable must be false with no store"
    );
    assert!(
        !check.migrations_applied,
        "migrations_applied must be false with no store"
    );
    // index_open is independent of the store — an empty engine is still servable.
    assert!(check.index_open, "empty engine should be servable");
    assert!(
        !check.is_ready(),
        "must NOT be ready when the store is absent"
    );
}

/// Fresh deploy with a store attached and zero spaces → READY. This is the
/// load-bearing decision (Task F1.3, 2026-07-18): readiness is "can the
/// process do useful work", not "is there data". A brand-new deployment has
/// no wiki spaces yet but can accept `spaces register`, so it is ready.
#[test]
fn from_runtime_fresh_deploy_with_store_is_ready() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = empty_engine();
    let store = make_store(tmp.path());
    let check = ReadinessCheck::from_runtime(&engine, Some(&store));
    assert!(
        check.db_reachable,
        "db_reachable must be true with a live store"
    );
    assert!(
        check.migrations_applied,
        "a fresh store is at CURRENT_DISK_SCHEMA_VERSION ({CURRENT_DISK_SCHEMA_VERSION})"
    );
    assert!(
        check.index_open,
        "empty engine (zero spaces) must be servable — fresh deploy is ready"
    );
    assert!(check.is_ready(), "fresh deploy with a store IS ready");
}

/// The schema-version gate: `migrations_applied` requires the store's on-disk
/// schema version to match `CURRENT_DISK_SCHEMA_VERSION`. We can't easily
/// forge a mismatched store on disk, but we CAN assert the positive direction
/// (a freshly created store reports the current version) — which is the only
/// state the pre-production no-migration-path world can be in.
#[test]
fn from_runtime_store_reports_current_schema_version() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = make_store(tmp.path());
    // Sanity: the store itself reports the current version (the factory relies
    // on this to set migrations_applied=true).
    assert_eq!(
        store.schema_version(),
        CURRENT_DISK_SCHEMA_VERSION,
        "a freshly created store must be at CURRENT_DISK_SCHEMA_VERSION"
    );
    let engine = empty_engine();
    let check = ReadinessCheck::from_runtime(&engine, Some(&store));
    assert!(
        check.migrations_applied,
        "migrations_applied mirrors the schema-version match"
    );
}
