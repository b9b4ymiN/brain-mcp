import { defineConfig, devices } from '@playwright/test'

// Two test modes share this config (Task E1.4):
//
// 1. `chromium` — unit-style specs (safetext/format/review/smoke). Runs the
//    production SPA bundle through `vite preview` on :4173 with NO backend.
//    These exercise pure TS functions and the boot shell only; they must keep
//    passing without a Rust server (the original E1.0–E1.3 contract).
//
// 2. `chromium-realbackend` — real-API E2E specs (`*.real.spec.ts`). Boots
//    the REAL Rust server on :8080 serving the built `dist/` (single-origin:
//    same server answers `/` static assets and `/api/v1/*`). The webServer
//    launcher (`scripts/serve_e2e.mjs`) writes a temp TOML config with
//    `console_dev_bootstrap_secret = "e2e-bootstrap-secret"` + the dist path,
//    seeds the store via `cargo run --example seed_console_e2e`, then starts
//    `llm-wiki serve`. Specs log in with that secret and drive every page.
//
// Routing is by filename (`testMatch`) so the two projects never collide:
//   chromium             → specs ending in `.spec.ts` but NOT `.real.spec.ts`
//   chromium-realbackend → specs ending in `.real.spec.ts`
//
// Running only unit specs without a Rust toolchain:
//   E2E_SKIP_REAL_BACKEND=1 npm run test:unit
//   (or just: npm run test:unit — the npm script sets the flag for you)
// Setting E2E_SKIP_REAL_BACKEND=1 drops the real-backend webServer from the
// config entirely so Playwright never tries to start it.

const PORT_STATIC = 4173
const BASE_URL_STATIC = `http://localhost:${PORT_STATIC}`

const PORT_REAL = 8080
const BASE_URL_REAL = `http://127.0.0.1:${PORT_REAL}`

const SKIP_REAL_BACKEND = process.env.E2E_SKIP_REAL_BACKEND === '1'

// Auto-skip the real-backend webServer when the user is clearly only running
// unit specs. Two signals:
//   1. Explicit env override (E2E_SKIP_REAL_BACKEND=1) — manual escape hatch.
//   2. The CLI filters to the unit project only, OR passes explicit test paths
//      that are all unit specs (no `*.real.spec.ts`). This lets
//      `npm run test:unit`, `npx playwright test smoke.spec.ts`, etc. boot
//      WITHOUT a Rust toolchain — the original E1.0–E1.3 contract.
// When the real-backend project IS selected, or no project/path filter is
// given (full run), the real server boots normally.
function realBackendSelected(): boolean {
  if (SKIP_REAL_BACKEND) return false
  const argv = process.argv.slice(2)
  const hasProjectFilter = argv.some((a) => a === '--project' || a.startsWith('--project='))
  const projectValues: string[] = []
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--project') {
      projectValues.push(argv[i + 1] ?? '')
    } else if (a.startsWith('--project=')) {
      projectValues.push(a.slice('--project='.length))
    }
  }
  // Any --project that isn't exactly 'chromium' implies real-backend needs the
  // server. If the only --project is 'chromium', skip.
  if (hasProjectFilter) {
    return projectValues.some((p) => p !== 'chromium')
  }
  // No --project filter: look at explicit positional test paths. If ALL of
  // them are non-real specs, skip the real server.
  const positionalPaths = argv.filter((a) => !a.startsWith('-') && a.endsWith('.ts'))
  if (positionalPaths.length > 0) {
    const hasRealPath = positionalPaths.some((p) => p.includes('.real.spec.ts'))
    return hasRealPath
  }
  // No filter at all → run everything, need the real server.
  return true
}

const WANT_REAL_BACKEND = realBackendSelected()

const realBackendWebServer = {
  // Real Rust server for `*.real.spec.ts`. The launcher builds nothing itself;
  // the unit webServer's `npm run build` (and Playwright's own project build)
  // already produced dist/, which this server serves via console_static_dir.
  //
  // NOTE: Playwright forbids specifying BOTH `port` and `url` on a webServer
  // (it throws "Either 'port' or 'url' should be specified"). We use `url`
  // alone so Playwright polls the real `/health` route to detect readiness —
  // a stronger signal than "port is bound" (the Rust HTTP listener accepts
  // connections only after the router is fully mounted).
  command: 'node scripts/serve_e2e.mjs',
  url: `http://127.0.0.1:${PORT_REAL}/health`,
  reuseExistingServer: !process.env.CI,
  // cargo can be slow on a cold first build (Rust + all deps). 180s gives
  // headroom on CI; a warm dev box finishes in ~5s.
  timeout: 180_000,
}

const webServers = [
  {
    // Static-SPA server for unit specs (no backend). Original E1.0 contract.
    command: `npm run build && npm run preview -- --port ${PORT_STATIC} --strictPort`,
    url: BASE_URL_STATIC,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
]
if (WANT_REAL_BACKEND) {
  webServers.push(realBackendWebServer)
}

export default defineConfig({
  testDir: './e2e',
  // `fullyParallel: false`: the real-backend specs share MUTABLE store state
  // (approve/reject/supersede consume proposals; the seed runs once at server
  // boot). Parallel runs would race on which test consumes which proposal and
  // on the throttled/aborted routes in states.real.spec.ts. Unit specs are
  // fast (<5s for all 31) so serial execution is no loss there. Each consuming
  // spec file additionally uses `test.describe.configure({ mode: 'serial' })`
  // to pin intra-file order.
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  // When real-backend specs are in the run, force a single worker: they share
  // MUTABLE store state (approve/reject consume proposals; the seed runs once
  // at server boot) and states.real.spec.ts aborts/throttles shared routes.
  // Single-worker + alphabetical file order gives deterministic reads-before-
  // writes. Unit-only runs keep the default (parallel) worker count.
  workers: WANT_REAL_BACKEND ? 1 : undefined,
  reporter: 'list',
  use: {
    // Per-project baseURL overrides this for the real-backend project.
    baseURL: BASE_URL_STATIC,
    trace: 'on-first-retry',
  },
  projects: WANT_REAL_BACKEND
    ? [
        {
          name: 'chromium',
          testMatch: /.*\.spec\.ts$/,
          testIgnore: /.*\.real\.spec\.ts$/,
          use: { ...devices['Desktop Chrome'] },
        },
        {
          name: 'chromium-realbackend',
          testMatch: /.*\.real\.spec\.ts$/,
          use: {
            ...devices['Desktop Chrome'],
            baseURL: BASE_URL_REAL,
            // Phase E1/E2 viewport + DPR baseline (matches the E2 benchmark
            // requirement and gives deterministic full-page screenshots).
            viewport: { width: 1920, height: 1080 },
            deviceScaleFactor: 1,
          },
        },
      ]
    : [
        // Unit-only mode: just the static-SPA project, no real backend.
        {
          name: 'chromium',
          testMatch: /.*\.spec\.ts$/,
          testIgnore: /.*\.real\.spec\.ts$/,
          use: { ...devices['Desktop Chrome'] },
        },
      ],
  webServer: webServers,
})
