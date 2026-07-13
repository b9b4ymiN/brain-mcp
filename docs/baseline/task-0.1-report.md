# Task 0.1 Baseline Report

Status: **AWAITING INDEPENDENT REVALIDATION**

## Outcome

The canonical production repository is a fresh clone at `C:\Programing\AI2.0\jarvis\brain-mcp-vnext`, based on upstream `v0.4.15` commit `c49e7b30705f0055402dea2b2ec3b1471c1e29b3`. Product behavior, existing contracts, schemas, and dependencies were not changed. The only implementation is an isolated restore drill plus its baseline evidence.

The local immutable tag `vnext-upstream-baseline-20260713` and upstream tag `v0.4.15` both point to the upstream baseline commit. vNext work is on `vnext/phase-0`.

## Verification matrix

| Gate | Result | Evidence |
|---|---:|---|
| Rust tests, Windows MSVC | PASS | 565 passed, 0 failed, 0 ignored; `cargo +1.95-x86_64-pc-windows-msvc test --locked -j 1` |
| Rust formatting | PASS | `cargo fmt -- --check` |
| Rust clippy | PASS | `cargo +1.95-x86_64-pc-windows-msvc clippy --all-targets --locked -j 1 -- -D warnings` |
| Engine integration | PASS | 63 passed |
| MCP integration | PASS | 76 passed |
| ACP integration | PASS WITH KNOWN SKIPS | 26 passed, 2 persistent-session tests skipped |
| Total Python integration | PASS WITH KNOWN SKIPS | 165 passed, 2 skipped |
| Restore drill | PASS | Git HEAD, 28-file manifest/hash, semantic search, and graph parity under Windows PowerShell 5.1 and PowerShell 7.5.8 |
| CodeGraph | PASS | 138 files, 2,197 nodes, 6,038 edges |
| Linux upstream CI | PASS, HISTORICAL EXACT COMMIT | CI run 26642899431 and integration run 26642898659 on exact upstream SHA |
| Current RustSec audit | FAIL, OPEN SECURITY RISK | 1 vulnerability plus 3 warnings; see `security-audit-20260713.json` |

Linux evidence:

- CI: https://github.com/hataichanokpan-dev/brain-mcp/actions/runs/26642899431
- Integration Tests: https://github.com/hataichanokpan-dev/brain-mcp/actions/runs/26642898659

Both runs completed successfully on Ubuntu for exact SHA `c49e7b30705f0055402dea2b2ec3b1471c1e29b3` on 2026-05-29. There is no claim that the local evidence commits ran on Linux, and no claim that the historical security job covers later advisories.

## Failure-feedback loop

1. The repository-pinned GNU Windows toolchain failed because `dlltool.exe` was absent. Classification: environment/toolchain mismatch.
2. The first explicit MSVC parallel test failed with Windows `os error 1455` (paging file too small). The rerun set `CARGO_INCREMENTAL=0`, used a repository-local target directory, and limited Cargo to one job; all 565 tests then passed.
3. The first restore drill failed because JSON object key ordering differed after restore even though records, facets, scores, hashes, and graph output matched. RED checkpoint `ecca7d4` captures this. GREEN checkpoint `efed49b` recursively canonicalizes object keys while preserving array order; the drill then passed.
4. `cargo audit` initially was unavailable. `cargo-audit 0.22.2` was installed with the pinned MSVC toolchain and one build job. The current audit then returned exit 1 with real findings; they are not reclassified as environment failures.
5. Independent validation found that the documented restore command failed under Windows PowerShell 5.1: successful `git clone` progress on stderr became `NativeCommandError` under `ErrorActionPreference=Stop`, and the script used newer .NET path/hash helpers. The script now captures the native exit code before restoring the preference and uses compatible relative-path/SHA-256 APIs. Exact reruns passed on Windows PowerShell 5.1.26100.8737 and PowerShell 7.5.8.

## Risks and follow-up

- **Security:** `crossbeam-epoch 0.9.18` is vulnerable under RUSTSEC-2026-0204. `anyhow 1.0.102` and `memmap2 0.9.10` have unsoundness warnings; `bincode 2.0.1` is unmaintained. Dependency changes are forbidden in Task 0.1, so Phase 0 must remediate or explicitly accept these before implementation proceeds beyond its risk gate.
- **Reproducibility:** `tests-integration/uv.lock` exists locally but is ignored and untracked. The integration environment is therefore not repository-reproducible yet.
- **Test isolation:** the full Rust suite generated untracked JSON files in a tracked fixture directory. They were identified from the initially clean clone and removed. Future test runs need the same cleanup until the isolation defect is fixed in a permitted task.
- **Windows resource sensitivity:** parallel clean builds can exceed the current pagefile. The reproducible Windows command uses MSVC, `CARGO_INCREMENTAL=0`, and `-j 1`.
- **CI supply chain:** upstream workflows use mutable action tags (`actions/checkout@v6`, `Swatinem/rust-cache@v2`, and `astral-sh/setup-uv@v7`). Pin them to immutable commit SHAs in a task that permits CI changes.

## Rollback and data safety

No user data was read or migrated. Restore tests used generated temporary directories. Removing the two restore-drill commits and the baseline evidence returns the branch to the immutable upstream tag; upstream history and the baseline tag are untouched.

## Execution accounting

- Builder/orchestrator: Codex
- Validator: independent read-only agent; first review returned one restore-shell compatibility finding, revalidation pending
- Product files changed: 0
- Eval/script files changed: 1
- Evidence files changed: 7
- RED/GREEN retries: 1 restore-oracle retry; 1 validator restore-shell compatibility retry; 2 Windows toolchain/resource retries; 1 security-tool availability retry
- Token telemetry: unavailable from the execution environment
