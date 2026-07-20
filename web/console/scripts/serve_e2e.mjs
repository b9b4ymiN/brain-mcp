// @ts-check
/**
 * Phase E Task E1.4 — Playwright real-backend webServer launcher.
 *
 * Boots the REAL Rust server (`llm-wiki serve`) against a throwaway config so
 * the `*.real.spec.ts` E2E suite drives the genuine `/api/v1` + static-asset
 * pipeline (single-origin: the server serves both the built SPA and the API).
 *
 * Why a Node launcher instead of a shell one-liner in playwright.config.ts?
 *   1. Cross-platform: identical behaviour on Windows (dev) and Linux (CI).
 *      Playwright runs `webServer.command` through a shell, and the quoting /
 *      tempdir / path-joining rules differ enough between cmd.exe and bash
 *      that a shell one-liner is brittle here.
 *   2. Process-tree management: we spawn the seed (runs to completion) and
 *      then the server (long-lived). On SIGTERM/SIGINT we forward to the
 *      server child so Playwright's teardown actually releases port 8080.
 *   3. No new deps — pure Node `child_process`, `fs`, `os`, `path`.
 *
 * Pipeline:
 *   1. Ensure `dist/` exists (built by a prior `npm run build`). The Playwright
 *      webServer config runs `npm run build` BEFORE this script, so this is a
 *      guard, not a builder.
 *   2. mkdtemp → write `config.toml` with:
 *        [serve]
 *        console_dev_bootstrap_secret = "e2e-bootstrap-secret"
 *        console_static_dir = "<abs path to dist>"
 *      The server's `state_dir` is the temp dir's parent (config_path.parent()
 *      per `src/engine.rs`), so the semantic store auto-creates at
 *      `<tmp>/semantic-store` — exactly where the seed writes.
 *   3. Run `cargo run --example seed_console_e2e -- <tmp>` (one-shot, exits).
 *      Idempotent by operation_id; safe if the server already created the
 *      store (it'll `open` instead of `create`).
 *   4. Spawn `cargo run --bin llm-wiki -- --config <tmp>/config.toml serve
 *      --http :8080` and stay alive until killed. stdout/stderr pass through
 *      so Playwright's stdout shows server logs on failure.
 *
 * Usage (invoked by playwright.config.ts webServer, not run directly):
 *   node scripts/serve_e2e.mjs
 *
 * Env overrides:
 *   E2E_PORT        (default 8080) — the port the server binds.
 *   E2E_SECRET      (default e2e-bootstrap-secret) — must match the specs.
 */

import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CONSOLE_DIR = resolve(__dirname, '..')
const WORKSPACE_DIR = resolve(CONSOLE_DIR, '..', '..')
const DIST_DIR = join(CONSOLE_DIR, 'dist')

const E2E_PORT = process.env.E2E_PORT ?? '8080'
// Phase G (2026-07-20): login uses username+password env vars. The env-var
// NAMES are pinned via the temp TOML below (E2E_USERNAME/E2E_PASSWORD) so
// they don't collide with the Windows built-in `USERNAME`. The VALUES
// mirror e2e/helpers.ts (CONSOLE_USERNAME / CONSOLE_PASSWORD) so both sides
// stay in sync.
const E2E_USERNAME = process.env.E2E_USERNAME ?? 'e2e-admin'
const E2E_PASSWORD = process.env.E2E_PASSWORD ?? 'e2e-bootstrap-secret'

// ── 1. dist guard ─────────────────────────────────────────────────────────
if (!existsSync(join(DIST_DIR, 'index.html'))) {
  console.error(
    '[serve_e2e] dist/index.html missing — run `npm run build` first. The Playwright webServer config must build before invoking this script.',
  )
  process.exit(1)
}

// ── 2. temp config ─────────────────────────────────────────────────────────
// mkdtemp gives a unique per-run dir; Playwright reuses the server across
// tests in a run, then tears it down (and the dir leaks until OS cleanup —
// acceptable for a dev/CI scratch dir, and preferable to racing on a fixed
// path when two Playwright runs overlap).
const tmpRoot = mkdtempSync(join(tmpdir(), 'console-e2e-'))
const configPath = join(tmpRoot, 'config.toml')
// TOML string values must be double-quoted; backslashes (Windows paths) are
// fine inside double quotes. Absolute path so the server resolves it
// regardless of its own cwd.
const distAbs = DIST_DIR.replace(/\\/g, '\\\\')
// Phase G: env-var names default to "BRAIN_USERNAME"/"BRAIN_PASSWORD"
// (namespaced to avoid colliding with the Windows built-in `USERNAME` env
// var). We pin them to E2E_* here for clarity + so the file is hermetic.
const configToml = [
  '[serve]',
  'console_dev_bootstrap_username_env = "E2E_USERNAME"',
  'console_dev_bootstrap_password_env = "E2E_PASSWORD"',
  `console_static_dir = "${distAbs}"`,
  // Loopback + default port; explicit so the test is reproducible. We do NOT
  // set http_bind_all_interfaces (default false = loopback only — the secure
  // default; never expose the dev-bootstrap-credential server publicly).
  '',
].join('\n')
writeFileSync(configPath, configToml, 'utf8')
console.log(`[serve_e2e] temp config at ${configPath}`)
console.log(`[serve_e2e] console_static_dir = ${DIST_DIR}`)
// Seed the env vars for the child cargo process (the server reads them at
// startup via std::env::var). Setting on process.env means the spawn below
// inherits them automatically.
process.env.E2E_USERNAME = E2E_USERNAME
process.env.E2E_PASSWORD = E2E_PASSWORD
console.log(`[serve_e2e] E2E_USERNAME set (${E2E_USERNAME.length} chars)`)

// Helper: run a cargo command, inherit stdio, throw on non-zero exit.
function runCargo(args, label) {
  return new Promise((resolveP, rejectP) => {
    console.log(`[serve_e2e] cargo ${args.join(' ')}`)
    const child = spawn('cargo', args, {
      cwd: WORKSPACE_DIR,
      stdio: 'inherit',
      // On Windows, `cargo` is cargo.exe — shell:true lets PATH resolve it
      // uniformly; without it Node tries to exec `cargo` literally.
      shell: true,
    })
    child.on('error', (err) => rejectP(new Error(`${label} spawn failed: ${err.message}`)))
    child.on('exit', (code, signal) => {
      if (code === 0) resolveP(code)
      else rejectP(new Error(`${label} exited code=${code} signal=${signal}`))
    })
  })
}

// ── 3. seed ────────────────────────────────────────────────────────────────
// The seed example targets <tmpRoot> as state_dir; the server (next step) and
// the seed share <tmpRoot>/semantic-store. One-shot — completes before we boot
// the server so there's no startup race for the inbox assertions.
try {
  await runCargo(
    ['run', '--example', 'seed_console_e2e', '--', tmpRoot],
    'seed_console_e2e',
  )
} catch (err) {
  console.error(`[serve_e2e] ${err.message}`)
  process.exit(1)
}

// ── 4. boot server (long-lived) ────────────────────────────────────────────
const server = spawn(
  'cargo',
  ['run', '--bin', 'llm-wiki', '--', '--config', configPath, 'serve', '--http', `:${E2E_PORT}`],
  {
    cwd: WORKSPACE_DIR,
    stdio: 'inherit',
    shell: true,
  },
)

// Forward teardown signals to the server child so Playwright's webServer
// teardown (SIGTERM) actually releases the port. Without this, on Windows the
// `cargo run` wrapper can orphan the real llm-wiki.exe and leave :8080 bound.
function killServer(signal) {
  try {
    // treekill on the cargo wrapper: kill the whole process group. On Unix
    // the negative-PID kill works because we'd need detached:true for a new
    // group; here we just signal the child and rely on cargo forwarding to
    // the llm-wiki binary. Playwright also force-kills after a grace period.
    process.kill(server.pid, signal)
  } catch {
    // already gone
  }
}
process.on('SIGTERM', () => killServer('SIGTERM'))
process.on('SIGINT', () => killServer('SIGINT'))
server.on('exit', (code, signal) => {
  console.log(`[serve_e2e] server exited code=${code} signal=${signal}`)
  process.exit(code ?? 0)
})
server.on('error', (err) => {
  console.error(`[serve_e2e] server spawn failed: ${err.message}`)
  process.exit(1)
})
