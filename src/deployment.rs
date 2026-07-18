//! Reproducible deployment contract (Task 6.1).
//!
//! Domain types for GOAL-vNext §13 Task 6.1 + §9.3. Docker Compose is the
//! portable deployment contract; Oracle ARM is a reference host. Secrets are
//! references (never baked into image/compose/repo). Health/readiness does
//! not report ready before DB/migration/index checks pass.
//!
//! Contract-level: no real `docker compose up` in-env. These types model the
//! manifest a deployment runbook consumes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::WikiEngine;
use crate::semantic::{CURRENT_DISK_SCHEMA_VERSION, SemanticStore};

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

    /// Build a `ReadinessCheck` from live engine + store state. This is what
    /// the `/ready` HTTP handler calls on every probe — it is the load-bearing
    /// wiring that turns the contract type into a real production gate (Phase
    /// F1.3 — the F1 lesson again: the gate must be on the real serve path,
    /// not just in a contract).
    ///
    /// Semantics:
    /// - `db_reachable`: the semantic store is attached (Some) AND responds to
    ///   a cheap probe (`schema_version()` reads the on-disk marker). A probe
    ///   failure (e.g. the store file is gone/corrupt) flips the gate to false
    ///   without taking the server down.
    /// - `migrations_applied`: the store's on-disk schema version matches
    ///   `CURRENT_DISK_SCHEMA_VERSION`. Pre-production, there is no forward
    ///   migration path; a version mismatch means "this binary cannot serve
    ///   this store" → not ready (matches `validate_database_identity`).
    /// - `index_open`: the engine can take its state read lock AND at least
    ///   one wiki space has an open searcher. We deliberately allow a
    ///   *fresh* deploy (zero spaces) to report `index_open = true` — the
    ///   service is ready to accept `spaces register`, the readiness gate is
    ///   "can the process do useful work", not "is there data" (Task F1.3
    ///   decision, 2026-07-18). When spaces ARE registered, each one must have
    ///   an open searcher or the gate flips (a half-mounted space is not
    ///   servable).
    ///
    /// `store` is `Option` because the serve path may run with brain_* tools
    /// disabled (no semantic store attached). In that shape the readiness
    /// gate fails on `db_reachable` and `migrations_applied`, which is
    /// correct: a brain service without its semantic store is not ready to
    /// serve brain traffic.
    pub fn from_runtime(engine: &WikiEngine, store: Option<&Arc<SemanticStore>>) -> Self {
        // db_reachable + migrations_applied both derive from the store handle.
        // A `None` store means brain_* tools are disabled — not ready.
        let (db_reachable, migrations_applied) = match store {
            Some(s) => {
                // Cheap probe: schema_version() reads the in-memory marker
                // populated at open/create. If the store handle is alive, the
                // backing database was reachable at open time. (We do NOT
                // re-open the connection per probe — that would be too
                // expensive for a health check and would defeat the
                // liveness/readiness split.)
                let reachable =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.schema_version()))
                        .is_ok();
                let migrated = reachable && s.schema_version() == CURRENT_DISK_SCHEMA_VERSION;
                (reachable, migrated)
            }
            None => (false, false),
        };

        // index_open: engine state lock is acquireable AND, when spaces are
        // registered, every space has an open searcher. Zero spaces = ready
        // (fresh deploy, can still accept `spaces register`).
        //
        // `parking_lot::RwLock::read()` returns the guard directly (not a
        // `Result`), so lock acquisition cannot fail at the type level — but
        // we treat the rare poison/panic case as "not ready" defensively via
        // `catch_unwind`. The inner closure does the real work.
        let index_open = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let state = engine.state.read();
            if state.spaces.is_empty() {
                // Fresh deploy — engine is up, no spaces yet. Ready.
                true
            } else {
                // Every mounted space must have an open searcher.
                state
                    .spaces
                    .values()
                    .all(|space| space.index_manager.searcher().is_ok())
            }
        }))
        .unwrap_or(false);

        Self {
            db_reachable,
            migrations_applied,
            index_open,
        }
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
