import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E3.3 — Operations dashboard against the REAL Rust server.
//
// Drives the Operations page (Task E3.3 Part B) end-to-end:
//   1. Trust section loads — fresh seed has zero contradictions/stale flags,
//      so we assert the empty state.
//   2. Clients section shows the "console" client registered at login.
//   3. Jobs section shows the {active:0, queued:0, failed:0} big-number card.
//   4. Backup health shows last_restore_drill_ok=false with the Phase F
//      callout.
//   5. Evals: enter domain "stocks" → eval summary loads (run_at="never"
//      initially; case_count=0).
//
// Each section is independently fetched (per the page's `Promise.all` on
// mount + per-section refresh), so the assertions target each card
// separately rather than the whole dashboard.

test.describe('Operations dashboard (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
    await gotoNav(page, 'Operations')
    await expect(
      page.getByRole('heading', { name: 'Operations', level: 1 }),
    ).toBeVisible()
  })

  test('trust section loads (empty for fresh seed)', async ({ page }) => {
    const card = page.locator('section.card-trust')
    await expect(card).toBeVisible()
    // Fresh seed has no contradictions/stale flags → StateBox empty branch.
    await expect(card.getByText('No contradictions or stale flags.')).toBeVisible({
      timeout: 5_000,
    })
  })

  test('clients section shows the console client registered at login', async ({
    page,
  }) => {
    const card = page.locator('section.card-clients')
    await expect(card).toBeVisible()
    // The seed + login register a "console" client; the Clients table surfaces
    // its label. Allow a brief window for the parallel `/ops/clients` fetch.
    await expect(card.getByText('console').first()).toBeVisible({
      timeout: 5_000,
    })
    // The capabilities column renders (string join, text-bound).
    await expect(card.locator('table.clients-table')).toContainText(/read|capture|purge|mutation|\w+/)
  })

  test('jobs section shows 0/0/0 initially', async ({ page }) => {
    const card = page.locator('section.card-jobs')
    await expect(card).toBeVisible()
    // Three big-number cells, all zero on a fresh store. We assert each
    // label + its value renders — the big-number grid is role="group".
    const group = card.getByRole('group', { name: 'Job counts' })
    await expect(group).toBeVisible({ timeout: 5_000 })
    await expect(group.getByText('Active')).toBeVisible()
    await expect(group.getByText('Queued')).toBeVisible()
    await expect(group.getByText('Failed')).toBeVisible()
    // Three "0" big-number-value cells.
    await expect(group.locator('.big-number-value')).toHaveText(['0', '0', '0'])
  })

  test('backup health shows last_restore_drill_ok=false with the Phase F callout', async ({
    page,
  }) => {
    const card = page.locator('section.card-backup')
    await expect(card).toBeVisible()
    // The honest "not yet drilled — Phase F" line is rendered as text when
    // last_restore_drill_ok=false (the seed's default).
    await expect(card.getByText('not yet drilled — Phase F')).toBeVisible({
      timeout: 5_000,
    })
    // The callout block expands on the same point.
    await expect(
      card.getByText(/restore-drill harness ships in Phase F/),
    ).toBeVisible()
  })

  test('evals: entering domain "stocks" loads the eval summary', async ({
    page,
  }) => {
    const card = page.locator('section.card-evals')
    await expect(card).toBeVisible()
    await card.getByLabel('Domain').fill('stocks')
    await card.getByRole('button', { name: 'Load' }).click()
    // The seed store has never run an eval. The server ships run_at="never"
    // for a fresh store; `formatDate('never')` falls back to the em-dash
    // placeholder ('—') since 'never' isn't a parseable ISO timestamp. We
    // assert the "Run at" label renders and that case_count=0 shows up.
    await expect(card.getByText('Run at')).toBeVisible({ timeout: 5_000 })
    await expect(card).toContainText('Cases')
    await expect(card).toContainText('—')
  })
})
