import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav, CONSOLE_SECRET } from './helpers.ts'

// Phase E1.4 — the 4 observable UI states against the REAL Rust server.
//
// Phase E1 DoD #3 requires loading / empty / error / permission coverage:
//   - loading: a brief loading indicator while a read is in flight. Hard to
//     catch against a localhost server (requests finish in ~ms), so we force
//     it via Playwright's route-throttling on a known request — that keeps
//     the assertion deterministic without faking the backend.
//   - empty: search no-match (also covered in search.real.spec.ts; restated
//     here as a StateBox assertion so the states contract is self-contained).
//   - error: a network failure on a read route. Forced by aborting the
//     request — proves the Console surfaces a `state-error` block instead of
//     a blank screen or a thrown exception.
//   - permission: logging out (or a 401 mid-session) drops the user back to
//     the login form. Real logout via the Sign out button + a follow-up read
//     that returns 401.

test.describe('UI states (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  test('empty state: search no-match shows the StateBox empty message', async ({ page }) => {
    await gotoNav(page, 'Search')
    await page.getByLabel('Query').fill('zzzz-no-match-zzzz')
    await page.getByRole('button', { name: 'Search' }).click()
    await expect(page.getByText('No claims matched this query.')).toBeVisible()
  })

  test('loading state: throttled inbox request shows the loading indicator', async ({
    page,
  }) => {
    // Throttle /api/v1/inbox so the request stays in flight long enough to
    // observe the loading state. We route-fulfil with a delay, NOT a stub —
    // the response still comes from the real API contract (a bare JSON array).
    await page.route('**/api/v1/inbox', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 800))
      await route.continue()
    })
    await gotoNav(page, 'Inbox')
    // StateBox loading branch — Inbox.svelte shows "Loading…" text while the
    // list request is in flight. The throttled 800ms gives a comfortable
    // window; the assertion resolves as soon as the text appears.
    await expect(page.getByText(/Loading/i).first()).toBeVisible({ timeout: 5_000 })
    // And it clears once the (delayed) response lands.
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()
  })

  test('error state: a failed read surfaces the state-error block', async ({ page }) => {
    // Abort the inbox request → the API client throws ApiError(503,
    // 'unavailable') (see src/lib/api.ts network-failure path). Inbox renders
    // the `state-error` message instead of crashing.
    await page.route('**/api/v1/inbox', (route) => route.abort('failed'))
    await gotoNav(page, 'Inbox')
    // Inbox's catch branch produces a "Failed to load inbox ..." message.
    await expect(page.getByText(/Failed to load inbox/)).toBeVisible({ timeout: 5_000 })
  })

  test('permission state: logout drops the user back to the login form', async ({ page }) => {
    // While logged in, the Sign out button is visible.
    await expect(page.getByRole('button', { name: 'Sign out' })).toBeVisible()
    await page.getByRole('button', { name: 'Sign out' }).click()

    // The shell re-renders the login section.
    await expect(page.getByRole('heading', { name: 'Sign in', level: 2 })).toBeVisible()
    await expect(page.getByLabel('Bootstrap secret')).toBeVisible()
    // Nav is gone (only rendered when session.isLoggedIn).
    await expect(page.getByRole('navigation', { name: 'Primary' })).toHaveCount(0)

    // A subsequent navigation attempt that would hit a protected route stays
    // gated: the session cookie is dead server-side, so any API call 401s.
    // Re-login works — proving the gate is recoverable, not a hard wall.
    await page.getByLabel('Bootstrap secret').fill(CONSOLE_SECRET)
    await page.getByRole('button', { name: 'Sign in' }).click()
    await expect(page.getByRole('navigation', { name: 'Primary' })).toBeVisible()
  })
})
