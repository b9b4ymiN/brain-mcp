import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E1.4 — Entity page against the REAL Rust server.
//
// The seed plants a CONFIRMED prior claim GULF target_price=55 in domain
// "stocks". The Entity page calls `api.getSubject({subject})` and renders a
// claims table; each row expands inline to fetch the timeline for that
// subject+predicate via `api.timeline(...)`. We assert both the table and
// the expanded timeline populate from real storage.
//
// DoD coverage: Phase E1 DoD #1 (real-API wiring for Entity) — table + the
// expand-to-timeline micro flow.

test.describe('Entity (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
    await gotoNav(page, 'Entity')
    await expect(page.getByRole('heading', { name: 'Entity', level: 1 })).toBeVisible()
  })

  test('shows the confirmed GULF claim row in the claims table', async ({ page }) => {
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()

    const table = page.locator('table.claims-table')
    await expect(table).toBeVisible()
    // The GULF row's accessible name is its aria-label ("Toggle timeline for
    // GULF target_price"); the subject cell text is what we actually want to
    // pin (text-bound, not raw HTML). Assert via the table's text content.
    await expect(table).toContainText('GULF')
    await expect(table).toContainText('target_price')
    await expect(table).toContainText('55')
  })

  test('expanding a row renders timeline entries from the real API', async ({ page }) => {
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()

    // The row is a focusable button (role="button" on the <tr>). Click to
    // expand the inline timeline.
    const row = page.locator('table.claims-table tbody tr.row').first()
    await row.click()

    // Timeline panel renders a list with Status / Kind / Confirmed seq dl
    // entries. The seed's confirmed prior has status "confirmed".
    const timeline = page.locator('.timeline-list')
    await expect(timeline).toBeVisible()
    await expect(timeline).toContainText('confirmed')
    await expect(timeline).toContainText('Confirmed seq')
  })

  test('unknown subject shows the empty state', async ({ page }) => {
    await page.getByLabel('Subject').fill('NO-SUCH-SUBJECT-XYZ')
    await page.getByRole('button', { name: 'View' }).click()
    // Entity StateBox emptyText interpolates the active subject.
    await expect(
      page.getByText('No claims found for subject "NO-SUCH-SUBJECT-XYZ".'),
    ).toBeVisible()
  })
})
