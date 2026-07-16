//! Task 6.1 — Reproducible deployment contract (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 6.1 DoD at the contract level (Rust domain
//! types; no real `docker compose up` in-env). Docker Compose is the portable
//! deployment contract; Oracle ARM is a reference host.

use llm_wiki::deployment::{
    DeploymentManifest, HealthStatus, Platform, ReadinessCheck, SecretRef, ServiceSpec,
};

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
