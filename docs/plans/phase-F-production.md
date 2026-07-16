# Phase F — Production Deployment (Docker) + Recovery ⬜ pending

> Index: [`../robustness-plan.md`](../robustness-plan.md) · Base: `vnext/phase-0`
> เป้าหมาย: ปิด **Phase 6 Gate** — `docker compose up` จริงบน clean host + restore drill จริง + external security review

## Decision (2026-07-16)
- **Docker ต้องรันได้ทั้ง 2 arch จริง** — `docker buildx` multi-arch + QEMU emulation รัน arm64 container smoke บนเครื่องนี้ (ไม่ carry ไป Oracle)

## Task F1 — Reproducible deployment (Docker Compose, multi-arch)
- Dockerfile multi-stage (pinned base + toolchain) + `docker-compose.yml` (contract `DeploymentManifest` Task 6.1)
- `docker buildx build --platform linux/amd64,linux/arm64`; รัน arm64 smoke ผ่าน QEMU
- Secrets ผ่าน Docker secrets/secret files — ไม่ bake ใน image/compose/repo
- Health/readiness: ไม่ ready ก่อน DB+migration+index checks ผ่าน (contract `ReadinessCheck`)
- TLS + bind: default loopback (Phase B); production 0.0.0.0 เฉพาะหลัง reverse proxy + auth (ผูก Phase D worker auth)

**DoD (§13 Task 6.1):**
1. `docker compose up` บน clean host ผ่าน smoke/health/MCP/Console checks — **amd64 และ arm64 (QEMU)**
2. images build+test ทั้ง `linux/amd64` + `linux/arm64`; deps/toolchain/base pinned
3. HTTPS/domain/secrets/volumes/migrations/backup จาก runbook บน clean host ได้
4. health ไม่รายงาน ready ก่อน checks ผ่าน (negative test)
5. ไม่มี secret ใน image layers (scan gate)

## Task F2 — Observability + operations wiring *(ปิด contract-only Task 6.2)*
- Wire `LogRedactor` เข้า log path จริง; structured logs/metrics/traces/queue-depth/projection-lag/auth-failures/storage alerts
- Rate/size/time limits บังคับ runaway ingest + provider cost/quota จริง

**DoD (§13 Task 6.2):**
1. Log redaction: token/source secret ไม่รั่วใน log จริง (integration test ยิง log แล้ว grep)
2. Metrics endpoint expose ครบ; projection lag/auth failure นับได้จริง
3. Ingest limits บังคับจริง (เกิน limit → reject)

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
