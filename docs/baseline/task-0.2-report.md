# Task 0.2 Authority/Time/Privacy ADR Report

Status: **AWAITING INDEPENDENT REVALIDATION**

## Outcome

Task 0.2 freezes the semantic authority, identity, bitemporal, privacy/purge, authorization, threat, and eval contracts without changing runtime product behavior or dependencies.

Key decisions:

- The append-only Event Ledger is the sole semantic-transition authority; the Purge Registry is the separate monotonic deny/decryption authority.
- Raw objects and authored Markdown are canonical content/evidence, while claim snapshots, generated wiki, Tantivy, Petgraph, and future vectors are rebuildable projections.
- UUIDv7 is the server-generated stable opaque ID; per-owner `event_seq` is the authoritative order.
- Valid time uses half-open `[from,to)` intervals; transaction-time/as-of selection uses ledger sequence, not client/Git/UUID time.
- Latest-user-wins is limited to a full `(owner, domain, subject, predicate, context)` scope for user preference/profile/project decisions. External facts and stock opinions preserve contradictions.
- Normal deletion is retract/archive. Hard purge is an irreversible, recent-reauthenticated, two-step, deny-first key/object/Git/index/backup saga.
- Keycloak 26.7.0 is the decision-baseline Authorization Server candidate, but deployment must pin an exact image digest and remote MCP is hard-disabled until that image proves MCP 2025-11-25 RFC 8707 resource-to-audience conformance. Failure requires a replaceable compliant AS, not a waived audience check.

## Changed artifacts

- `docs/adr/0001-semantic-authority-time-privacy.md` — accepted decisions and consequences.
- `docs/security/threat-model-v1.md` — 24 owned threats with severity, controls, test IDs, and gates.
- `evals/v1/contracts/event-schema-v1.json` — JSON Schema Draft 2020-12 event envelope.
- `evals/v1/metrics.json` — hard invariants, formulas, thresholds, and critical regression.
- `evals/v1/cases/*.jsonl` — 30 stocks, 30 projects, 30 knowledge, and 34 adversarial cases with self-contained expected outputs.
- `evals/v1/run.py` — pure-standard-library executable reference oracle that verifies byte locks, derives actual outputs, compares expected outputs, computes metrics, and enforces thresholds.
- `evals/v1/manifest.json` — exact runner/environment command, seed, counts, promotion rules, and SHA-256/byte locks.
- `tests-integration/governance/test_eval_contract.py` — executable contract tests.

No Rust source, existing MCP contract, storage schema, dependency, lockfile, workflow, deployment, or user data changed.

## RED/GREEN evidence

- Base: `1597b829ab15312da714992eb529e1f2acd599de`
- RED checkpoint: `004b5da4839ac2af0d2a346d95ce5b12c199226e`
- RED command: `uv run pytest governance/test_eval_contract.py -q`
- RED result: exit 1; four tests failed with `FileNotFoundError` for the absent ADR, event schema, manifest, and metrics. This was the intended missing-contract failure, not a syntax/environment failure.
- GREEN command: same focused pytest command.
- Initial GREEN result: exit 0; 4 passed.
- Validator-feedback RED checkpoint: `9c4de17414ebc042601e83563bd3d5f455da2002`.
- Validator-feedback RED result: exit 1; three tests failed because the executable runner, its byte lock, and concrete purge/restore/public-bind/key-revocation cases did not yet exist.
- Final GREEN result: exit 0; 5 passed.
- Governance coverage: `coverage run --source=governance -m pytest governance/test_eval_contract.py -q` then `coverage report --fail-under=80`; exit 0, 102/102 statements, 100%.
- Eval-runner coverage on the locked Python 3.14.4 toolchain: 402 statements, 17 missed, 96%.
- Event schema: `Draft202012Validator.check_schema`; exit 0.

The locked reference command is:

```powershell
uv run --no-project --python 3.14.4 python evals/v1/run.py --manifest evals/v1/manifest.json --strict-environment
```

It exits 0 with 124/124 actual-versus-expected cases, all byte and environment locks valid, hard-invariant pass rate `1.0`, Recall@10 `1.0`, nDCG@10 `0.9197207891481876`, schema-valid rate `1.0`, evidence-span exactness `1.0`, supported-claim precision `1.0`, and zero confirmed unsupported claims. It has no project dependency or ignored lockfile dependency.

## Regression verification

Running all Python directories in one pytest invocation produced 13 collection errors because identically named modules such as `test_graph.py` exist under ACP, engine, and MCP. This is a baseline test-layout limitation; the upstream CI contract runs them separately. Using that exact feedback, the suites were rerun separately:

| Suite | Result |
|---|---:|
| Governance | 5 passed |
| Engine | 63 passed |
| MCP | 76 passed |
| ACP | 26 passed, 2 known skips |
| Total | 170 passed, 2 skipped |

The separate-suite command exited 0 in 164.6 seconds.

## Eval contract inventory

| Corpus | Cases | SHA-256 |
|---|---:|---|
| Stocks | 30 | `68be5e04fb8fec9627259f69781e9c685eb77d221000e3c532e07d156ff5eb77` |
| Projects | 30 | `7e5c9c98cfc623ded6f5f83497ab916e4b08982608eac9016aa3fb84461cb7bb` |
| Knowledge | 30 | `21902c69f02893789f4e63e834afea75c05699c6afb1544f3f94fbee1c1758b3` |
| Adversarial | 34 | `31c24b687e122f57fb97b6e9901a1383592914ad13a96cb5e2443405170aedae` |

The adversarial corpus retains the six original risk groups and adds concrete executable cases for purge-registry denial during cleanup, stale restore after a newer purge epoch, attempted public bind without authentication, and revoked-key cleanup retry.

## Independent-validator feedback loop

The first independent review returned `FAIL` with four blocking findings. Each exact finding was converted to a test or verification gate before implementation:

1. Structural fixtures did not execute actual-versus-expected evaluation. `evals/v1/run.py` now derives an actual result for every case, compares it to the expected oracle, mechanically verifies evidence spans, computes all declared metrics, and fails non-zero on any regression.
2. The runner contract depended on an ignored integration `uv.lock`. The locked command now uses `uv --no-project`, exact Python 3.14.4, and a standard-library-only runner; no project lock or third-party runtime package is required.
3. Critical threat rows referenced future suites rather than executable IDs. TM-011, TM-012, TM-013, and TM-023 now map to concrete current IDs, including a real `0.0.0.0` unauthenticated-bind refusal case.
4. `git diff --check` reported trailing blank lines. Both affected documents were normalized and the check now exits 0.

Revalidation remains mandatory before Task 0.3 starts.

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
- Independent Validator: first review failed with four blockers; fixes are complete and revalidation is pending.

## Rollback and data safety

Task 0.2 is documentation and synthetic test data only. Reverting its commits returns to Task 0.1 HEAD. No runtime database, object store, wiki, credential, identity provider, network listener, or user data was created or modified.

## Execution accounting

- Builder/orchestrator: Codex
- Validator: independent read-only agent; initial `FAIL`, revalidation pending
- Production code changed: 0 lines
- Synthetic eval cases: 124
- TDD retries: initial missing-artifact RED, validator-feedback runner/critical-case RED, one stocks provenance oracle failure corrected from its exact message, and one aggregate-pytest collection limitation corrected by using upstream separate-suite commands
- Governance coverage: 100%
- Eval-runner coverage: 96%
- Token telemetry: unavailable from the execution environment
