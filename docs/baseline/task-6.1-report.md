# Phase 6 — Production + recovery Report (Tasks 6.1 + 6.2 + 6.3)

Status: **PASS at contract level** — Independent Validator confirmed all three Tasks (6.1/6.2/6.3) on branch `vnext/phase-0`, with 4 INFO observations (all acknowledged deferrals, 0 CRITICAL/HIGH/MEDIUM). The Phase 6 Gate is **contract-cleared but not production-closed**: the three real-world gate items (actual `docker compose up` on clean hosts, external security review, real clean-host restore drill) are intentionally deferred to the production-acceptance phase, consistent with how Phase 5's Gate is held open for browser verification.

## Task 6.1 — Reproducible deployment (HEAD `977ac8d`)

**DoD coverage**: DeploymentManifest targets LinuxAmd64 + LinuxArm64; SecretRef is a reference (never raw key); ReadinessCheck all-gates-true before `is_ready()`; smoke checks (health + MCP + write/read round-trip). 6/6 tests pass.

## Task 6.2 — Observability (HEAD `0904e47`)

**DoD coverage**: MetricPoint (Counter/Gauge/Histogram + name + value); LogRedactor strips Bearer/sk-/access_token=/api_key= (delegates to provider::redact_secrets); IngestLimits (size + rate + max_tokens caps). 5/5 tests pass. Added `redact_secrets` + `redact_value_after_marker` to provider.rs (shared by egress gate + log redaction).

## Task 6.3 — Backup/restore/upgrade (HEAD `499280f`, test-strengthened this commit)

**DoD coverage**: BackupReport (encrypted, objects/ledger/Git/config + checksum); RecoveryDrillResult (fail-closed: `passed()` requires registry_synced AND checksum_matches); SchemaUpgradePlan (all steps reversible); RpoRto (recorded). 5/5 tests pass. Validator INFO-2 closed: `restore_drill_fails_closed_without_registry` now asserts `!result.passed()` (not just the precondition).

## Gates (re-verified independently by Validator)

- 3 Phase 6 test suites: 6 + 5 + 5 = 16/16 pass.
- `cargo test -j 2` (default): 0 failed.
- `cargo test --all-features -j 2`: 0 failed.
- `cargo clippy --all-targets --all-features -j 2`: clean.
- `cargo fmt --check`: clean.
- No new dependency: `Cargo.toml`/`Cargo.lock` unchanged.
- Isolation: `semantic_vertical_slice` 14/14; no `semantic::` in any new module.

## Phase 6 Gate status

**Contract-cleared.** All three Tasks meet their §13 DoD as Rust domain contracts. The Gate's three real-world requirements remain open for the production-acceptance phase:
- "restore ได้จริง" — real clean-host restore drill (contract models it).
- "external security review" — this Validator is read-only re-check, not the external review the Gate names.
- "production acceptance" — real `docker compose up` on clean amd64/arm64 hosts.

**Phase 7 (Eval-driven evolution) is clear to start** at the contract level pending its own Task Brief review.
