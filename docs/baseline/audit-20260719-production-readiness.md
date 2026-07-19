# Production-Readiness Audit — 2026-07-19

> Scope: full-project audit at HEAD `b68c2df` (branch `vnext/phase-0`) ahead of loopback production bring-up on this machine (Docker Desktop, Windows 11).
> Method: 5 audit tasks — doc consistency (T1), full verification pass (T2), security posture (T3), dead-contract sweep (T4), Docker runtime smoke (T5). T1/T3/T4 by independent read-only agents; T2/T5 executed against an isolated HEAD worktree so the operator's in-flight F3.3 working-tree changes were never included. Audit wrote no repo files except this report.
> Out of scope: Task F3 closure (F3.3 upgrade/rollback + RPO/RTO + external review) — in progress by operator in a parallel session.

## 1. FACT — verification gates at HEAD b68c2df

| Gate | Result |
|---|---|
| cargo fmt --check | PASS |
| cargo clippy --all-features --all-targets | PASS (0 warnings) |
| cargo test --all-features -j 2 --no-fail-fast | **FAIL — 70 suites ok / 1 failed** (see Finding A) |
| web console svelte-check + tsc | PASS (524 files, 0 errors) |
| web console vite build | PASS (warn: main chunk 1.59 MB > 500 kB) |
| Playwright unit specs (chromium, mock) | PASS 41/41 |
| Docker compose smoke (scripts/docker_compose_smoke.sh @ HEAD worktree) | **PASS — all 12 checks green** (port 18082, HEAD worktree build) |

## 2. Finding A — full test suite not green since Phase E0.2 (verification-process gap)

FACT: `semantic_module_is_isolated_and_legacy_runtime_does_not_call_writer` (tests/semantic_vertical_slice.rs:1122) fails at HEAD. Guard allowlist (semantic.rs, lib.rs, projection.rs, server.rs — last updated `a544d5f`, Phase C) forbids `semantic::` elsewhere in src/; `src/api.rs` has contained `semantic::` since E0.2 `7784260`, whose own header comment declares the Console API is built on `SemanticStore` public methods.

ANALYSIS: the test has been failing for every commit since E0.2, through the E and F gates — meaning gates ran targeted suites, not `cargo test --all-features`. Not a runtime bug: api.rs's use of SemanticStore is the deliberate E0 design. The guard simply never learned about it.

Decision needed (operator): (a) add `"api.rs"` to the allowlist as an authorized bridge — 1 line, matches evolved intent; or (b) refactor api.rs behind an authorized layer. Then make full-suite green a standing gate.

## 3. FACT — documentation staleness (14 discrepancies, 10 misleading)

Verified by cross-checking all plan files + guides against git history (all 34 cited SHAs exist; all cited paths resolve). Core cluster: **Phase D closed at `899a489` but docs still say NEXT**, and Phase F progress (F1/F2 closed, F3.1/F3.2 done) unrecorded.

| # | File:line | Stale claim → truth |
|---|---|---|
| 1 | docs/robustness-plan.md:3 | "Phase D = NEXT" → D CLOSED `899a489` |
| 2 | docs/robustness-plan.md:18 | D row ⬜ NEXT → ✅ CLOSED |
| 3 | docs/robustness-plan.md:20 | F row ⬜ pending → F1+F2 CLOSED, F3.1/F3.2 done |
| 4 | docs/robustness-plan.md:34 | D3 deferred open → delivered `899a489` |
| 5 | docs/robustness-plan.md:36 | D5 deferred open → compose+restore drill done; external review open |
| 6 | docs/robustness-plan.md:63 | worker priv-sep risk open → shipped `899a489` |
| 7 | docs/robustness-plan.md:69-80 | progress log missing D close + all F entries |
| 8 | docs/plans/phase-D-zai-adapter.md:1 | header ⬜ NEXT, no ✅/SHA in file → D CLOSED (largest gap) |
| 9 | docs/plans/phase-F-production.md:1 | header ⬜ pending → self-contradicts lines 18/34 |
| 10 | docs/guides/deploy-docker.md:437 | "restore drill not yet implemented" → implemented `35314ec` |
| 11 | docs/guides/deploy-docker.md:508 | "What's next (Phase F2)" → F2 closed; items post-F |
| 12 | docs/plans/phase-E-console-galaxy.md:50 | `bench/…` path → actually `web/console/bench/…` |
| 13 | README.md:53-61 | tool list omits brain_capture/confirm/supersede/propose/ingest_source |
| 14 | CHANGELOG.md | nothing since 0.4.15 / 2026-05-29 (upstream stream; team decision) |

Recommendation: fix rows 1–11 in one docs commit when F3 closes (F3 close touches the same files).

## 4. FACT — security posture (audited at HEAD; no Critical/High)

Verdict: **no Critical/High blockers for loopback go-live.** Two Medium items to record as conscious trust-boundary decisions in the go-live sign-off:

- **SEC-1 (Medium)** `/mcp` HTTP grants full-capability owner principal to any TCP caller (src/server.rs:501-532, src/mcp/mod.rs:69-137); AuthPolicy framework is enforced for non-owner principals but the HTTP path never derives a caller principal. Safe ONLY because compose publishes `127.0.0.1:8082→8080` and default bind is loopback. **Widening bind/publish without adding MCP auth = immediate unauthenticated full write/purge.**
- **SEC-2 (Medium)** no rate limit/lockout on `/api/v1/auth/login` (src/api.rs ~459; `rate_limited()` exists unwired). Mitigated by 256-bit operator secret; wire before any non-loopback exposure.
- Low/Info: bincode 2.0.1 unmaintained advisory (via petgraph-live; only cargo-audit warning, 0 CVEs) · Docker base images pinned by tag not digest · `console_dev_bootstrap_secret` named "dev" but IS the production auth (rename/annotate) · optional security headers (nosniff/Referrer-Policy) · SECURITY.md contact appears inherited from upstream (SECURITY.md:11) — confirm.
- Verified good: secrets hygiene clean (no tracked secrets; test fixtures synthetic) · .gitignore/.dockerignore exclude secrets/data/backups · Docker secret via `secrets:` file (absent from Config.Env — verified in smoke) · default bind 127.0.0.1, `0.0.0.0` requires explicit opt-in + UNAUTHENTICATED warning · console session auth solid (HttpOnly/SameSite=Strict, double-submit CSRF, constant-time compares, fail-closed mount) · purge requires 300 s re-auth · F3.1 backups AES-256-GCM, fresh nonce per call, envelope DEK/KEK · no shell-injection surface, ServeDir traversal-safe · worker principal denied at dispatch AND store layers · npm audit 0 vulns.

## 5. FACT — runtime wiring (dead-contract sweep at HEAD, verified at call sites)

Wired for real: all 39 MCP tools dispatch to real handlers (advertised == dispatch table, no stubs) · `/ready` = real `ReadinessCheck::from_runtime` 200/503 · backup/restore (BackupReport, RecoveryDrillResult, RestoreReceipt) · `/ops/backup-health`, `/ops/jobs`, trust/ops console routes · 11 JSON schemas registered.

Gaps (none block loopback bring-up):

| Gap | Where | Impact |
|---|---|---|
| **GAP-1** eval pipeline has no producer — `record_eval_run()` never called; `/ops/evals` + console panel permanently `case_count:0` | semantic.rs:5159/5115, api.rs:183,885 | §11 eval gate never actually runs; panel looks broken in prod |
| **GAP-2** governance layer contract-only (AutoApprovePolicy, RegressionReport, DomainEvalReport, Consolidation*) — tests only | consolidation.rs:37-151 | §8 auto-approve/§8.3 regression absent at runtime; fail-safe (manual confirm fallback) |
| **GAP-3** `SemanticStore::recover()` real impl, no operator surface (no CLI/MCP/API trigger) | semantic.rs:3842 | self-repair unreachable by operator |
| GAP-4 | `index.auto_recovery` + `defaults.search_sections` config flags are no-ops (config.rs:86,41) | cosmetic |
| GAP-5 | DeploymentManifest family tests-only (deployment.rs) — compose is the real contract | cosmetic |
| (F3.3) | RpoRto + SchemaUpgradePlan producers | in operator's in-flight F3.3 work |

## 6. Docker runtime smoke (T5)

FACT (scripts/docker_compose_smoke.sh run against a clean HEAD worktree, port 127.0.0.1:18082, per-run random secret, isolated scratch tree, `docker compose down -v` cleanup):

| Check | Result |
|---|---|
| image build (multi-stage, BuildKit cache) + compose up | PASS |
| /health liveness | PASS (healthy after 2 s, `uptime_secs`/`wikis` shape) |
| /ready readiness gate | PASS (`db_reachable`/`migrations_applied`/`index_open` all true) |
| login wrong secret → 401 | PASS |
| /metrics Prometheus text + `console_auth_failures_total` incremented by the 401 | PASS |
| login correct secret → 200 + csrf_token | PASS |
| console index `<title>Brain Console</title>` | PASS |
| SECURITY: secret absent from `docker inspect Config.Env` | PASS |
| SECURITY: `/run/secrets/bootstrap_secret` mount present, correct source | PASS |
| image history free of secret | PASS |
| non-root container (`uid=1000(brain)`) | PASS |

Note: an operator-side `brain-compose-smoke:dev` image existed from a parallel F3 session ~45 min before this run; this audit rebuilt from the HEAD worktree (BuildKit dedup applies) — results above are from the audit's own run.

## 7. RISK register (ranked)

| # | Risk | Level | Mitigation |
|---|---|---|---|
| R1 | Widening network exposure without MCP auth (SEC-1) | HIGH if exposed / accepted at loopback | record decision; add MCP auth before any non-loopback bind |
| R2 | Full-suite test gate not enforced (Finding A) → future regressions can slip like the guard test did | MED | fix guard allowlist, add full-suite run to gate checklist |
| R3 | Eval/governance §8/§11 absent at runtime (GAP-1/2) → quality gates advertised by spec don't run | MED | schedule post-F work or de-scope explicitly |
| R4 | Disk pressure on C: (was 99 % full during audit; 41 GB free after cleanup) — backups/data/docker images share the drive | MED | monitor; prune docker build cache; consider moving backups |
| R5 | Doc staleness misleads future sessions/operators | LOW-MED | one docs commit (Section 3) |
| R6 | bincode unmaintained; images tag-pinned | LOW | track; optional digest pin |

## 8. Go/No-Go — loopback production on this machine

GO criteria met at audit time: image builds from clean HEAD · compose stack healthy (`/health`, `/ready` real gates) · auth 401/200 correct · secret never in env/image (verified in smoke gates 9-11) · security: no Critical/High · runtime surfaces wired.

Conditions on GO (operator accepts):
1. Phase F Gate NOT yet closed — F3.3 (upgrade/rollback rehearsal, RPO/RTO) + Independent Validator PASS outstanding; running now is a pre-gate deployment by operator decision.
2. SEC-1/SEC-2 accepted as loopback-only trust decisions (do not widen exposure without revisiting).
3. Finding A resolved or explicitly waived until F3 close.
4. Eval panel will show empty (GAP-1) — known, not a malfunction.

## 9. Evidence

- T1–T5 raw results: session scratchpad (`t1-doc-audit-result.md`, `t2-verify-result.md`, `t3-security-result.md`, `t4-sweep-result.md`, `t5-smoke.log`)
- Verification worktree: detached checkout of `b68c2df`, separate CARGO_TARGET_DIR (24 GB, deleted after run)
- Windows quirks confirmed: `-j 2` required; worktree needs explicit `RUSTUP_TOOLCHAIN=1.95-x86_64-pc-windows-msvc` (channel-only resolution picked GNU → dlltool error)
- Hygiene note: stray dir `scripts/docker-smoke-config.toml;C/` in working tree (untracked artifact — safe to delete)

## 10. Production bring-up record (2026-07-19, post-audit)

Operator directed pre-gate loopback bring-up (Phase F Gate still open — condition 1 of Section 8 accepted).

- Image: `brain:v0.5` ← tagged from the HEAD-worktree smoke build (`brain-compose-smoke:dev`, commit `b68c2df`); WIP working tree never entered the image.
- Operator layout created per runbook: `config/config.toml` (from examples/config.docker.toml), `secrets/bootstrap_secret.txt` (openssl rand -hex 32, 0600, CR/LF-stripped, gitignored), `data/`, `backups/`.
- `docker compose up -d --no-build`: /health 200 (2 s) · /ready `{db_reachable,migrations_applied,index_open}` all true (1 s) · login 401 wrong / 200 correct + csrf_token · console index served.
- MCP over HTTP at `127.0.0.1:8080/mcp`: initialize OK (protocol 2025-11-25) · tools/list OK · `brain_status` → `{"status":"healthy","schema_version":2,"active_claims":0}`.
- Restart survival: `docker restart brain` → ready again in 1 s, MCP healthy, `restart: unless-stopped` in effect.
- Backup path proven live: `recovery backup` (encrypted, checksum emitted) → `recovery drill` (`composite_checksum_matches:true`, `purge_registry_synced:true`) → `/ops/backup-health` reports `last_restore_drill_ok:true`. Test backup dir removed after drill.
