# Phase F — Production Deployment (Docker) + Recovery ⬜ pending

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> เป้าหมาย: ปิด **Phase 6 Gate** — `docker compose up` จริงบน clean host + restore drill จริง + external security review

## Decision (2026-07-16)
- **Docker ต้องรันได้ทั้ง 2 arch จริง** — `docker buildx` multi-arch + QEMU emulation รัน arm64 container smoke บนเครื่องนี้ (ไม่ carry ไป Oracle)

## Decision (2026-07-18, Task F1.3) — arm64 DEFERRED
- **arm64 build/smoke deferred** per user direction. The one-time QEMU binfmt
  setup (`docker run --rm --privileged multiarch/qemu-user-static --reset -p yes`)
  was skipped, and no Oracle ARM host or CI runner is wired yet. F1.3 ships
  **amd64 only** via `scripts/docker_buildx_multiarch.sh`; the arm64 block is
  preserved commented-out in that script for a one-line re-enable. DoD #1/#2
  amended below to "amd64 (arm64 DEFERRED)". Re-enable when an Oracle ARM host
  or CI runner is available.

## Task F1 — Reproducible deployment (Docker Compose, multi-arch) ✅ CLOSED amd64 (`9e3d6ba`, `41a1406`, `e5f7b41`)
- Dockerfile multi-stage (pinned base + toolchain) + `docker-compose.yml` (contract `DeploymentManifest` Task 6.1)
- `docker buildx build --platform linux/amd64` (F1.3); arm64 DEFERRED — see 2026-07-18 decision above
- Secrets ผ่าน Docker secrets/secret files — ไม่ bake ใน image/compose/repo
- Health/readiness: ไม่ ready ก่อน DB+migration+index checks ผ่าน (contract `ReadinessCheck`) — F1.3 wired the real `ReadinessCheck::from_runtime` behind `/ready` (liveness `/health` kept cheap)
- TLS + bind: default loopback (Phase B); production 0.0.0.0 เฉพาะหลัง reverse proxy + auth (ผูก Phase D worker auth)

**DoD (§13 Task 6.1):**
1. ✅ `docker compose up` บน clean host ผ่าน smoke/health/MCP/Console checks — **amd64 (arm64 DEFERRED — QEMU binfmt setup skipped per user 2026-07-18; re-enable when Oracle ARM host or CI runner available)**
2. ✅ images build+test `linux/amd64` (arm64 DEFERRED per 2026-07-18 decision); deps/toolchain/base pinned (rust:1.95-bookworm, node:20-bookworm, debian:bookworm-slim)
3. ✅ HTTPS/domain/secrets/volumes/migrations/backup จาก runbook บน clean host ได้ (`docs/guides/deploy-docker.md`)
4. ✅ health ไม่รายงาน ready ก่อน checks ผ่าน (negative test) — F1.3: `ReadinessCheck::is_ready()` returns false when any gate fails (9 tests in `tests/deployment_contract_v1.rs`); `from_runtime` real producer
5. ✅ ไม่มี secret ใน image layers (scan gate) — `docker history` clean, `docker inspect Config.Env` clean, secret via `_file:` Docker secret

Sub-tasks: F1.1 (`9e3d6ba`) · F1.2 (`41a1406`) · F1.3 (`e5f7b41`) — ผ่าน combined spec+quality review (APPROVED/APPROVED/APPROVED); cargo test 9+7+44 green, fmt+clippy clean, docker smoke green (health + login 401/200 + console index + secret-scan + /ready)

## Task F2 — Observability + operations wiring *(ปิด contract-only Task 6.2)* ✅ CLOSED (`6acd45a`, `12b9bb6`, `ec579c4`, `f5dbc96`)
- Wire `LogRedactor` เข้า log path จริง; structured logs/metrics/traces/queue-depth/projection-lag/auth-failures/storage alerts
- Rate/size/time limits บังคับ runaway ingest + provider cost/quota จริง

**DoD (§13 Task 6.2):**
1. ✅ Log redaction: token/source secret ไม่รั่วใน log จริง — `RedactingMakeWriter` wrap ทุก writer path (stderr + file + json/compact), 6 integration tests (`tests/observability_integration_v1.rs`) ผ่าน
2. ✅ Metrics endpoint expose ครบ; projection lag/auth failure นับได้จริง — `GET /metrics` Prometheus text format; `console_auth_failures_total`/`console_logins_total`/`ingest_total`/`mcp_calls_total{tool}`/`projection_lag_seconds`/`job_queue_depth{state}` counters ตามจริง; 5 integration tests (`tests/metrics_integration_v1.rs`) ผ่าน
3. ✅ Ingest limits บังคับจริง — `IngestRateLimiter` (sliding window, full-sweep eviction, bounded memory) + size check บน MCP `brain_ingest_source`/`brain_capture`/`wiki_ingest`; `PAYLOAD_TOO_LARGE`/`RATE_LIMITED` errors; 9 integration + 5 unit tests ผ่าน

Sub-tasks: F2.1 (`6acd45a`) · F2.2 (`12b9bb6`) · F2.3 (`ec579c4`) + memory-bound fix (`f5dbc96`) — ผ่าน combined spec+quality review (APPROVED ×3 + APPROVED_WITH_NITS ×1); cargo test 27+ green, fmt+clippy clean, /metrics smoke green, /metrics endpoint returns Prometheus text + counter increments verified live

## Task F3 — Backup/restore/upgrade + external security review
- Encrypted automated backup (objects/ledger/Git/config); **clean-host restore drill จริงใน Docker**
- Restore sync PurgeRegistry + key revocations ก่อน decrypt; registry stale/unavailable → **fail closed**
- Composite checksum (`ledger_head+purge_epoch+schema_version`) + event/object/Git/referential/projection manifests ตรง
- Schema upgrade + app rollback rehearsal; RPO/RTO บันทึก + monitor
- External security review (security-reviewer agent แยก — เทียบเท่า external)

**DoD (§13 Task 6.3):**
1. Restore drill บน clean Docker host ผ่าน + composite checksum ตรง + registry fail-closed พิสูจน์
2. Upgrade + rollback rehearsal ผ่านโดยข้อมูลไม่หาย
3. RPO/RTO บันทึกและ monitor ได้
4. Security review ไม่มี critical/high unresolved

**Phase F Gate = Phase 6 Gate ปิด:** production acceptance + external review clear + restore ได้จริง → **Independent Validator PASS** → **ระบบ production-ready**
