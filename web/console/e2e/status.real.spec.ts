import { test, expect } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'

// Status page (real backend) — Task 9 of the Console Expansion plan.
//
// The Rust `/status` endpoint (Task 6) returns `WikiStats`; this spec proves
// the Status page renders the hero band (severity + key metrics) and at least
// one detail panel after a real login. The second test covers the "Reindex →"
// link to /config — re-enabled once Task 17 landed the Config page.

test.beforeEach(async ({ page }) => {
  await loginAsConsole(page)
})

test('status page renders hero + detail grid', async ({ page }) => {
  await page.goto('/#/status')
  // Hero band — the page's own h1 (scoped to main to avoid the shell banner).
  await expect(
    page.getByRole('main').getByRole('heading', { name: 'Status', level: 1 }),
  ).toBeVisible()
  // Severity word present (one of three states). Word-boundary anchors keep
  // the regex from matching substrings of other text on the page.
  await expect(page.getByText(/\b(Nominal|Stale|Degraded)\b/)).toBeVisible()
  // Detail grid — at least one panel heading.
  await expect(page.getByText('Staleness', { exact: false })).toBeVisible()
})

// Config page now exists (Task 17) — Reindex navigates to /config.
test('status page Reindex link navigates to config', async ({ page }) => {
  await page.goto('/#/status')
  await page.getByRole('button', { name: /Reindex/ }).click()
  await expect(page).toHaveURL(/#\/config$/)
})
