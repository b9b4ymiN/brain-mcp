/**
 * Shared helpers for the real-backend E2E suite (`*.real.spec.ts`).
 *
 * Centralises the login flow so every spec starts from an authenticated
 * session against the Rust server at baseURL (http://127.0.0.1:8080). The
 * dev bootstrap secret is fixed by the webServer launcher
 * (`scripts/serve_e2e.mjs` → `console_dev_bootstrap_secret`); specs never
 * hardcode a per-run value.
 *
 * Do NOT use this from the unit specs — they run against `vite preview` with
 * no backend, and `loginAsConsole` would hit a non-existent `/api/v1`.
 */
import { expect, type Page } from '@playwright/test'

/** The dev bootstrap secret stamped into the temp TOML by serve_e2e.mjs. */
export const CONSOLE_SECRET = 'e2e-bootstrap-secret'

/**
 * Navigates to the shell and logs in via the real login form. Asserts the
 * post-login flash + the primary nav appears so the caller can assume a fully
 * authenticated, ready-to-drive shell. Returns nothing; the page fixture is
 * mutated in place (cookie is set in the browser context by the form submit).
 */
export async function loginAsConsole(page: Page): Promise<void> {
  await page.goto('/')
  // Login form is the unauthenticated shell — see App.svelte.
  await page.getByLabel('Bootstrap secret').fill(CONSOLE_SECRET)
  await page.getByRole('button', { name: 'Sign in' }).click()
  // Post-login: a success flash (the `.flash` banner in App.svelte — scoped
  // by class because StateBox's loading paragraph also uses role="status",
  // which would make a role-based selector ambiguous) + the primary nav.
  await expect(page.locator('.flash')).toContainText('Signed in')
  await expect(page.getByRole('navigation', { name: 'Primary' })).toBeVisible()
}

/**
 * Navigates to a primary page by its nav link label. The nav labels live in
 * `src/lib/router.ts` (PAGE_LABELS); passing the label keeps specs decoupled
 * from the internal page id.
 */
export async function gotoNav(page: Page, label: string): Promise<void> {
  await page.getByRole('link', { name: label }).click()
}
