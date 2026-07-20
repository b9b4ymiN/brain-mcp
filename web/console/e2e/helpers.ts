/**
 * Shared helpers for the real-backend E2E suite (`*.real.spec.ts`).
 *
 * Centralises the login flow so every spec starts from an authenticated
 * session against the Rust server at baseURL (http://127.0.0.1:8080). The
 * dev bootstrap credentials are fixed by the webServer launcher
 * (`scripts/serve_e2e.mjs` → `console_dev_bootstrap_username_env` +
 * `console_dev_bootstrap_password_env`); specs never hardcode a per-run value.
 *
 * Do NOT use this from the unit specs — they run against `vite preview` with
 * no backend, and `loginAsConsole` would hit a non-existent `/api/v1`.
 */
import { expect, type Page } from '@playwright/test'

/**
 * The dev bootstrap credentials stamped into the env (USERNAME/PASSWORD) by
 * serve_e2e.mjs. Phase G (2026-07-20) replaced the single shared secret with
 * a username+password pair sourced from env vars.
 */
export const CONSOLE_USERNAME = 'e2e-admin'
export const CONSOLE_PASSWORD = 'e2e-bootstrap-secret'

/**
 * Navigates to the shell and logs in via the real login form. Asserts the
 * post-login flash + the primary nav appears so the caller can assume a fully
 * authenticated, ready-to-drive shell. Returns nothing; the page fixture is
 * mutated in place (cookie is set in the browser context by the form submit).
 */
export async function loginAsConsole(page: Page): Promise<void> {
  await page.goto('/')
  // Login form is the unauthenticated shell — see App.svelte.
  await page.getByLabel('Username').fill(CONSOLE_USERNAME)
  await page.getByLabel('Password').fill(CONSOLE_PASSWORD)
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
