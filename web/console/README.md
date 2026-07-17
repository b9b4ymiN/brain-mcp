# Brain Console

The Svelte 5 single-page app for the `llm-wiki` semantic store. Reviews
pending proposals, searches the ledger, and explores entity timelines over
the real Rust HTTP API mounted at `/api/v1`.

This directory covers Phase E1 of the console rollout (foundation + the five
read/review pages + real-backend E2E). Production auth is OAuth (Phase F);
**dev uses a bootstrap secret** — see Security below.

## Prerequisites

- **Node 24+** (the dev toolchain targets Node 24).
- **Rust toolchain** (stable, matches `rust-version` in the workspace
  `Cargo.toml`). Required for `cargo run -- serve` and for the real-backend
  E2E suite.
- One initial `cargo build` so the `llm-wiki` binary exists:
  ```sh
  cargo build --bin llm-wiki
  ```

No global npm packages are required — everything is local to `node_modules`.

## Dev workflow (two terminals)

The Console is a Vite SPA. In dev it runs on `:5173` and proxies `/api` and
`/events` to the Rust server on `:8080` (see `vite.config.ts`), so the
`brain_console_session` cookie rides the same origin without CORS.

**Terminal 1 — Rust server (with a dev bootstrap secret):**

```sh
# Create a throwaway dev config (or wire it into ~/.llm-wiki/config.toml).
cat > /tmp/console-dev.toml <<'EOF'
[serve]
console_dev_bootstrap_secret = "dev-bootstrap-secret"
# Optional — serve the built bundle from this same origin once you've run
# `npm run build` once. Without this line, only /api/v1 + /health are served;
# use Vite (:5173) for the SPA itself during dev.
# console_static_dir = "<abs path>/web/console/dist"
EOF

cargo run --bin llm-wiki -- --config /tmp/console-dev.toml serve --http :8080
```

You should see:
```
Console HTTP API mounted at /api/v1 (dev bootstrap auth)
HTTP server listening addr=127.0.0.1:8080
```

**Terminal 2 — Vite dev server:**

```sh
cd web/console
npm install        # once
npm run dev        # http://localhost:5173
```

Open `http://localhost:5173`, sign in with `dev-bootstrap-secret`.

## Production single-origin

For a single-origin production deploy (no Vite proxy), point
`console_static_dir` at the built bundle. The Rust server then serves both
the SPA (`/`) and the API (`/api/v1`) from the same origin, with a strict
CSP on every static response.

```sh
cd web/console && npm run build          # produces dist/
cargo run --bin llm-wiki -- --config /path/to/prod.toml serve --http :8080
```

`prod.toml`:
```toml
[serve]
console_dev_bootstrap_secret = "<a real secret>"   # replaced by OAuth in Phase F
console_static_dir = "<abs path>/web/console/dist"
```

## Build

```sh
npm run build     # vite build → dist/
npm run check     # svelte-check 0/0 + tsc typecheck
```

## Tests

The Playwright config (`playwright.config.ts`) defines **two projects** that
share one runner:

| Project                  | Specs                     | Backend        |
| ------------------------ | ------------------------- | -------------- |
| `chromium`               | `*.spec.ts` (unit-style)  | none (vite preview of static bundle) |
| `chromium-realbackend`   | `*.real.spec.ts`          | real Rust server on :8080 |

**Unit-style specs** (`safetext.spec.ts`, `format.spec.ts`, `review.spec.ts`,
`smoke.spec.ts`) exercise pure TS functions and the boot shell. They need no
backend and no Rust toolchain.

**Real-backend specs** (`*.real.spec.ts`) boot the real Rust server via the
`webServer` launcher at `scripts/serve_e2e.mjs`. The launcher writes a temp
TOML config (`console_dev_bootstrap_secret = "e2e-bootstrap-secret"`,
`console_static_dir = <dist>`), seeds the store via
`cargo run --example seed_console_e2e`, then starts `llm-wiki serve`. Specs
log in with that secret and drive every page.

```sh
npm run test:e2e    # both projects, full pipeline
npm run test:unit   # unit specs only — NO Rust server booted
npm run test:real   # real-backend specs only
```

`npm run test:unit` is auto-detected by the Playwright config (via the
`--project=chromium` filter) and skips the real-backend webServer entirely,
so a unit-only run needs no Rust toolchain. You can also point a single
real spec at the runner: `npx playwright test home.real.spec.ts`.

### Seeding approach

The Console HTTP API only exposes READ and REVIEW routes — `capture` /
`propose` are NOT on the wire (§9: the Console may never write storage
directly). So seeding pending proposals for E2E cannot go through HTTP, and
`llm-wiki` has no CLI `capture`/`propose` subcommand. The seed is therefore a
small Rust **example** binary, `examples/seed_console_e2e.rs`, that links the
library and calls `SemanticStore` public methods directly against the exact
`<state_dir>/semantic-store` directory the server auto-creates/opens. It is
**opt-in**: only built by `cargo run --example seed_console_e2e`, never by
`cargo build` or `cargo test`, and adds no crate deps. It is test
infrastructure, not Console application code — the "no direct storage write"
rule applies to the Console app, not to a seed tool.

The seed is idempotent by `operation_id`; running it twice against an
already-seeded store is a no-op.

## Grep gate (Phase E1 DoD #1)

Every Console page must call the real API — no mock data, no leftover
`TODO`/`FIXME` markers ship in the production bundle.

```sh
# Windows (dev)
pwsh scripts/console_grep_gate.ps1

# Linux / CI
./scripts/console_grep_gate.sh
```

The gate builds `dist/` and greps the built `.js` for
`TODO|FIXME|MOCK_DATA|mock_` (case-insensitive). It exits non-zero with the
list of matches if any are found, `0` if clean. We grep the **built bundle**
(not the TS source) so the gate reflects what the browser actually runs.

## Security

- **Dev bootstrap secret is dev-only.** `console_dev_bootstrap_secret` is a
  single shared secret for local/loopback dev. Production auth is OAuth
  (Phase F). The Rust server binds loopback by default
  (`http_bind_all_interfaces = false`) — never set that to `true` behind
  only the bootstrap secret.
- **Strict CSP.** Every static response carries
  `default-src 'self'; script-src 'self'; style-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`
  (see `CONSOLE_CSP` in `src/api.rs`). No `unsafe-inline`, no `unsafe-eval`.
  Inline event handlers and injected `<script>` tags are blocked at the
  browser level.
- **XSS-safe rendering.** Every API value (subject, predicate, value,
  evidence excerpt, flash text, error codes) is bound via Svelte's `{value}`
  text syntax, which auto-escapes. There is no `{@html}` anywhere in the
  Console (enforced by grep in the DoD checklist). The real-backend E2E
  suite (`xss-csp.real.spec.ts`) seeds an XSS payload and asserts it renders
  as literal text, that no `alert` dialog opens, and that the CSP header is
  present and strict.
