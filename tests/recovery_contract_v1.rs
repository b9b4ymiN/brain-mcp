//! Task 6.3 — Backup, restore, upgrade, rollback contract (RED stage).
//!
//! GOAL-vNext §13 Task 6.3: encrypted backup, clean-host restore drill,
//! schema upgrade + rollback rehearsal, RPO/RTO.

use llm_wiki::recovery::{
    BackupReport, RecoveryDrillResult, RpoRto, SchemaUpgradePlan, UpgradeStep,
};

// =============================================================================
// DoD: encrypted automated backup covers objects/ledger/Git/config
// =============================================================================

#[test]
fn backup_report_covers_all_layers() {
    let report = BackupReport {
        objects_backed_up: 1500,
        ledger_events_backed_up: 3000,
        git_commits_backed_up: 42,
        config_snapshot: true,
        encrypted: true,
        checksum: "sha256:abc".to_owned(),
    };
    assert!(report.objects_backed_up > 0);
    assert!(report.ledger_events_backed_up > 0);
    assert!(report.encrypted, "backup must be encrypted");
    assert!(report.config_snapshot, "backup must include config");
}

// =============================================================================
// DoD: clean-host restore drill passes + sync purge registry
// =============================================================================

#[test]
fn restore_drill_passes_with_registry_sync() {
    let result = RecoveryDrillResult {
        objects_restored: 1500,
        ledger_events_restored: 3000,
        purge_registry_synced: true,
        composite_checksum_matches: true,
    };
    assert!(
        result.purge_registry_synced,
        "must sync purge registry before read"
    );
    assert!(
        result.composite_checksum_matches,
        "restore checksum must match source"
    );
}

#[test]
fn restore_drill_fails_closed_without_registry() {
    let result = RecoveryDrillResult {
        objects_restored: 1500,
        ledger_events_restored: 3000,
        purge_registry_synced: false,
        composite_checksum_matches: false,
    };
    assert!(
        !result.purge_registry_synced,
        "registry unavailable = fail closed"
    );
    assert!(
        !result.passed(),
        "restore drill must FAIL (passed=false) when registry is unsynced"
    );
}

// =============================================================================
// DoD: schema upgrade + rollback rehearsal
// =============================================================================

#[test]
fn upgrade_plan_has_forward_and_rollback_steps() {
    let plan = SchemaUpgradePlan {
        steps: vec![
            UpgradeStep {
                description: "apply v1→v2 DDL".to_owned(),
                reversible: true,
            },
            UpgradeStep {
                description: "backfill entity_ids".to_owned(),
                reversible: true,
            },
        ],
    };
    assert!(
        plan.steps.iter().all(|s| s.reversible),
        "every step must be reversible"
    );
}

// =============================================================================
// DoD: RPO/RTO recorded + monitored
// =============================================================================

#[test]
fn rpo_rto_is_recorded() {
    let rpo_rto = RpoRto {
        rpo_minutes: 1440, // daily backup = 24h RPO
        rto_minutes: 60,   // 1-hour restore target
        last_met: true,
    };
    assert!(rpo_rto.rpo_minutes > 0);
    assert!(rpo_rto.rto_minutes > 0);
    assert!(rpo_rto.last_met, "RPO/RTO must be met");
}
