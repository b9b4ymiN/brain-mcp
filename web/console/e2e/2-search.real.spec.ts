import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E1.4 — Search page against the REAL Rust server.
//
// Exercises the three observable states of `api.search()`:
//   - results appear (the seed confirms GULF target_price=55; searching
//     "gulf" must surface it as a hit card)
//   - empty state (a query that matches nothing shows the StateBox empty
//     message, not an error)
//   - validation/no-op (an empty query never fires the request — the form
//     guards `if (!trimmed) return`)
//
// DoD coverage: Phase E1 DoD #1 (real-API wiring for Search) + #3 (empty
// state).

test.describe('Search (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
    await gotoNav(page, 'Search')
    await expect(page.getByRole('heading', { name: 'Search', level: 1 })).toBeVisible()
  })

  test('returns the confirmed GULF claim for query "gulf"', async ({ page }) => {
    await page.getByLabel('Query').fill('gulf')
    await page.getByRole('button', { name: 'Search' }).click()
    // Result card renders with the subject (text-bound, not raw HTML).
    const gulfCard = page.getByRole('link', { name: /Open entity GULF/i })
    await expect(gulfCard).toBeVisible()
    await expect(gulfCard).toContainText('GULF')
    await expect(gulfCard).toContainText('target_price')
    // The confirmed value 55 must render as the Value field.
    await expect(gulfCard).toContainText('55')
  })

  test('no-match query shows the empty state, not an error', async ({ page }) => {
    await page.getByLabel('Query').fill('zzzzz-no-such-subject-xyz')
    await page.getByRole('button', { name: 'Search' }).click()
    // StateBox emptyText — exact wording from Search.svelte.
    await expect(page.getByText('No claims matched this query.')).toBeVisible()
    // Crucially NOT the error state.
    await expect(page.getByText(/Search failed/)).toHaveCount(0)
  })

  test('empty query is a no-op (no search fires)', async ({ page }) => {
    // Pre-search placeholder is visible before any submit.
    await expect(page.getByText('Run a search to see matching claims.')).toBeVisible()
    // The search input is `required` — clicking submit on an empty field
    // triggers the browser's constraint validation, not our handler. Either
    // way, no results section renders.
    await page.getByRole('button', { name: 'Search' }).click()
    await expect(page.getByText('No claims matched this query.')).toHaveCount(0)
    // Still on the pre-search placeholder.
    await expect(page.getByText('Run a search to see matching claims.')).toBeVisible()
  })
})
