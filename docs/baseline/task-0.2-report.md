# Task 0.2 Authority/Time/Privacy ADR Report

Status: **AWAITING INDEPENDENT VALIDATION**

## Outcome

Task 0.2 freezes the semantic authority, identity, bitemporal, privacy/purge, authorization, threat, and eval contracts without changing runtime product behavior or dependencies.

Key decisions:

- The append-only Event Ledger is the sole semantic-transition authority; the Purge Registry is the separate monotonic deny/decryption authority.
- Raw objects and authored Markdown are canonical content/evidence, while claim snapshots, generated wiki, Tantivy, Petgraph, and future vectors are rebuildable projections.
- UUIDv7 is the server-generated stable opaque ID; per-owner `event_seq` is the authoritative order.
- Valid time uses half-open `[from,to)` intervals; transaction-time/as-of selection uses ledger sequence, not client/Git/UUID time.
- Latest-user-wins is limited to a full `(owner, domain, subject, predicate, context)` scope for user preference/profile/project decisions. External facts and stock opinions preserve contradictions.
- Normal deletion is retract/archive. Hard purge is an irreversible, recent-reauthenticated, two-step, deny-first key/object/Git/index/backup saga.
- Keycloak 26.6.x is selected as the Authorization Server candidate, but remote MCP is hard-disabled until an exact image proves MCP 2025-11-25 RFC 8707 resource-to-audience conformance. Failure requires a replaceable compliant AS, not a waived audience check.

## Changed artifacts

- `docs/adr/0001-semantic-authority-time-privacy.md` — accepted decisions and consequences.
- `docs/security/threat-model-v1.md` — 24 owned threats with severity, controls, test IDs, and gates.
- `evals/v1/contracts/event-schema-v1.json` — JSON Schema Draft 2020-12 event envelope.
- `evals/v1/metrics.json` — hard invariants, formulas, thresholds, and critical regression.
- `evals/v1/cases/*.jsonl` — 30 stocks, 30 projects, 30 knowledge, and 30 adversarial cases with self-contained expected outputs.
- `evals/v1/manifest.json` — runner/environment, seed, counts, promotion rules, and SHA-256/byte locks.
- `tests-integration/governance/test_eval_contract.py` — executable contract tests.

No Rust source, existing MCP contract, storage schema, dependency, lockfile, workflow, deployment, or user data changed.

## RED/GREEN evidence

- Base: `1597b829ab15312da714992eb529e1f2acd599de`
- RED checkpoint: `004b5da4839ac2af0d2a346d95ce5b12c199226e`
- RED command: `uv run pytest governance/test_eval_contract.py -q`
- RED result: exit 1; four tests failed with `FileNotFoundError` for the absent ADR, event schema, manifest, and metrics. This was the intended missing-contract failure, not a syntax/environment failure.
- GREEN command: same focused pytest command.
- GREEN result: exit 0; 4 passed.
- Coverage: `coverage run --source=governance -m pytest governance/test_eval_contract.py -q` then `coverage report --fail-under=80`; exit 0, 86/86 statements, 100%.
- Event schema: `Draft202012Validator.check_schema`; exit 0.

## Regression verification

Running all Python directories in one pytest invocation produced 13 collection errors because identically named modules such as `test_graph.py` exist under ACP, engine, and MCP. This is a baseline test-layout limitation; the upstream CI contract runs them separately. Using that exact feedback, the suites were rerun separately:

| Suite | Result |
|---|---:|
| Governance | 4 passed |
| Engine | 63 passed |
| MCP | 76 passed |
| ACP | 26 passed, 2 known skips |
| Total | 169 passed, 2 skipped |

The separate-suite command exited 0 in 164.6 seconds.

## Eval contract inventory

| Corpus | Cases | SHA-256 |
|---|---:|---|
| Stocks | 30 | `3b7ecc42b307f87a55f1b13d1597745f970a67728cde03f37fa3f97bf3a89775` |
| Projects | 30 | `7e5c9c98cfc623ded6f5f83497ab916e4b08982608eac9016aa3fb84461cb7bb` |
| Knowledge | 30 | `21902c69f02893789f4e63e834afea75c05699c6afb1544f3f94fbee1c1758b3` |
| Adversarial | 30 | `7ef82e3d198511d94714da59a5baefd2b1c87a75864d385edbd5e79ec946d54e` |

The adversarial corpus has five cases each for prompt injection, secret/local-only egress, stored XSS, SSRF/path traversal, auth/re-auth, and retry/concurrency.

## Security findings and gates

- Current Keycloak documentation still calls MCP 2025-11-25 partially supported without Resource Indicators. The ADR therefore blocks remote exposure until exact conformance is proven and records AS replacement as the safe fallback.
- Task 0.1 RustSec findings remain open and are assigned to Phase 0 dependency remediation before Task 0.3 implementation.
- Mutable GitHub Action tags, ignored integration `uv.lock`, test fixture pollution, and Windows pagefile sensitivity remain explicit owned risks; Task 0.2 does not falsely claim remediation.
- Dynamic client registration and experimental client-metadata fetching default off because of registration/SSRF risk.

## Definition-of-Done mapping

- Authority/no dual truth: ADR Decisions 1–2.
- Event schema/stable IDs/time/actor/client: Decisions 3–6 plus locked event schema.
- Executable latest-wins/contradiction examples: projects/knowledge/stocks corpora.
- Retract versus purge and crash/restore behavior: Decision 7 plus threat/eval cases.
- Complete auth matrix and AS choice: Decision 8.
- Threat owner/severity/control/tests: threat model TM-001 through TM-024.
- Versioned/hash-locked evals with formulas/thresholds: `evals/v1` manifest and metrics.
- Independent Validator: pending.

## Rollback and data safety

Task 0.2 is documentation and synthetic test data only. Reverting its commits returns to Task 0.1 HEAD. No runtime database, object store, wiki, credential, identity provider, network listener, or user data was created or modified.

## Execution accounting

- Builder/orchestrator: Codex
- Validator: independent read-only agent, pending
- Production code changed: 0 lines
- Synthetic eval cases: 120
- TDD retries: one intended RED; one aggregate-pytest collection limitation corrected by using upstream separate-suite commands
- Governance coverage: 100%
- Token telemetry: unavailable from the execution environment
