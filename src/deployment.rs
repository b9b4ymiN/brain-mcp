//! Reproducible deployment contract (Task 6.1).
//!
//! Domain types for GOAL-vNext §13 Task 6.1 + §9.3. Docker Compose is the
//! portable deployment contract; Oracle ARM is a reference host. Secrets are
//! references (never baked into image/compose/repo). Health/readiness does
//! not report ready before DB/migration/index checks pass.
//!
//! Contract-level: no real `docker compose up` in-env. These types model the
//! manifest a deployment runbook consumes.

use serde::{Deserialize, Serialize};

// ── Platforms ────────────────────────────────────────────────────────────────

/// Target platforms. §6.1 "images รองรับและทดสอบ linux/amd64 + linux/arm64".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    LinuxAmd64,
    LinuxArm64,
}

// ── Secrets ──────────────────────────────────────────────────────────────────

/// A secret reference — the env var to populate + the secret-manager source.
/// NEVER the raw key. §6.1 "secrets mount ผ่าน Docker secrets/secret files
/// และไม่ bake ใน image/compose/repo".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    /// The env var the service reads (e.g. `AI_API_KEY_REF`).
    pub env_var: String,
    /// The secret-manager source (e.g. `op://vault/zai/key`, `docker:zai_key`).
    pub source: String,
}

// ── Health / readiness ───────────────────────────────────────────────────────

/// High-level health for a load balancer. Distinct from readiness (a service
/// can be healthy but not yet ready to serve).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Readiness gates. §6.1 "health/readiness ไม่รายงานพร้อมก่อน DB/migration/
/// index checks ผ่าน". All three must be true before `is_ready()`. Defaults to
/// all-false (a fresh service is not ready until each gate is confirmed).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub db_reachable: bool,
    pub migrations_applied: bool,
    pub index_open: bool,
}

impl ReadinessCheck {
    /// True only when ALL gates pass. A service that reports ready before this
    /// would serve requests against an unmigrated/unindexed store.
    pub fn is_ready(&self) -> bool {
        self.db_reachable && self.migrations_applied && self.index_open
    }
}

// ── Service spec ─────────────────────────────────────────────────────────────

/// One service in the deployment manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub image: String,
    pub secrets: Vec<SecretRef>,
    pub health: ReadinessCheck,
}

// ── Manifest ─────────────────────────────────────────────────────────────────

/// The portable deployment manifest. §6.1 "Docker Compose เป็น deployment
/// contract แบบ portable; Oracle ARM เป็น reference host".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentManifest {
    pub services: Vec<ServiceSpec>,
    pub platforms: Vec<Platform>,
    /// Smoke checks the runbook verifies after `compose up`.
    pub smoke_checks: Vec<String>,
}

impl DeploymentManifest {
    /// The default brain deployment: brain service + the two platforms + the
    /// standard smoke checks.
    pub fn default_brain() -> Self {
        Self {
            services: vec![ServiceSpec {
                name: "brain".to_owned(),
                image: "brain:v0.5".to_owned(),
                secrets: vec![SecretRef {
                    env_var: "AI_API_KEY_REF".to_owned(),
                    source: "docker:ai_api_key".to_owned(),
                }],
                health: ReadinessCheck::default(),
            }],
            platforms: vec![Platform::LinuxAmd64, Platform::LinuxArm64],
            smoke_checks: vec![
                "health endpoint returns 200".to_owned(),
                "mcp list-tools returns ≥10 tools".to_owned(),
                "write test profile → read back → bytewise equal".to_owned(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_all_gates() {
        let mut r = ReadinessCheck::default();
        assert!(!r.is_ready());
        r.db_reachable = true;
        r.migrations_applied = true;
        r.index_open = true;
        assert!(r.is_ready());
    }

    #[test]
    fn manifest_defaults() {
        let m = DeploymentManifest::default_brain();
        assert!(m.platforms.contains(&Platform::LinuxArm64));
        assert!(!m.smoke_checks.is_empty());
    }
}
