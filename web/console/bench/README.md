# Galaxy benchmark — `web/console/bench/`

This directory holds the **Galaxy component benchmark** artefacts for Phase E2
Task E2.3. The benchmark itself lives in
[`../e2e/galaxy-benchmark.real.spec.ts`](../e2e/galaxy-benchmark.real.spec.ts);
it writes its outputs into this directory on every run.

## What's tracked (committed)

- [`environment.schema.json`](./environment.schema.json) — JSON Schema for the
  `environment.json` record the benchmark writes at run start. Reviewers can
  validate any committed environment snapshot against this schema.

This `README.md` is also tracked.

## What's NOT tracked (gitignored — machine-specific)

- `environment.json` — captured per-machine on each run. Contains
  `navigator.userAgent`, `navigator.hardwareConcurrency`,
  `navigator.deviceMemory`, the WebGL renderer string via
  `WEBGL_debug_renderer_info`, plus the free-text `machine_label`. Never
  commit: it identifies the dev machine and is meaningless on another box.
- `results-dev-<date>.json` and any other `results-*.json` — the recorded
  numbers (FPS / click p95 / search-to-focus p95 / heap growth) from one run.
  These belong in a baseline report (attached to a PR or filed in the
  planning doc), NOT in the repo.

The `.gitignore` entries that enforce this live at the **repository root**
(`brain-mcp-vnext/.gitignore`), appended by E2.3:
```
web/console/bench/environment.json
web/console/bench/results-*.json
```

## Dev-bench flag policy (Phase E2 Gate)

The Phase E2 Task 5.2 DoD sets thresholds:

| metric                      | threshold            |
| --------------------------- | -------------------- |
| 1k-node graph FPS           | ≥ 45 FPS             |
| 5k-node graph FPS           | ≥ 30 FPS             |
| node-click latency (p95)    | < 100 ms             |
| search-to-focus latency p95 | < 300 ms             |
| heap growth over 20 cycles  | ≤ 10 % of initial    |

These thresholds were calibrated for the **baseline hardware** (the Phase F
re-bench machine). On a **dev machine** (typically no GPU in headless
Chromium → SwiftShader software WebGL, throttled rAF under background-tab /
battery-saver, etc.) the FPS numbers in particular are routinely below
threshold without indicating a real regression.

The benchmark spec therefore does **NOT hard-fail** when a threshold is
missed. Instead it:

1. Records the actual measured value into `bench/results-dev-<date>.json`.
2. Sets `threshold_met: false` on the missed metric.
3. Prints a clear `[BENCH]` log line per metric (visible in Playwright's
   stdout) so the gap is visible at a glance.
4. Continues / passes the test.

**The Phase E2 Gate explicitly accepts** "recorded on dev, flagged, re-bench
on baseline in Phase F" per the user-approved resolution. A green CI run on a
dev box means "the harness works and produced numbers" — NOT "we hit the
ship threshold". The Phase F baseline re-bench is the gating event for the
ship thresholds.

## When to re-bench

- Phase F entry — on the baseline hardware.
- After any change to:
  - `web/console/src/components/GalaxyGraph.svelte` (the component lifecycle),
  - `web/console/src/lib/galaxyRenderer*.ts` (the renderers),
  - `3d-force-graph` / `force-graph` / `three` version bumps in
    `web/console/package.json`.
- After the Carry 1 unmount-guard / Carry 3 middot fixes (this commit) —
  these don't touch the hot render path, but a sanity re-bench confirms no
  accidental regression.

## Running the benchmark

```bash
cd web/console
# Generate fixtures once (committed; idempotent).
node scripts/gen-graph-fixtures.mjs

# Run the full real-backend suite (boots Rust server on :8080):
npm run test:real
# Or just the Galaxy benchmark:
npx playwright test --project=chromium-realbackend galaxy-benchmark
```

The benchmark captures the environment, runs the 1k + 5k fixtures, measures
FPS / click p95 / search-to-focus p95, writes `bench/environment.json` +
`bench/results-dev-<date>.json`, and prints a `[BENCH]` summary.
