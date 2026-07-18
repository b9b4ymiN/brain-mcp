import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E3.4 — UAT spec for the "knowledge" domain (real backend).
//
// Walks Home → Search → Entity → Inbox-review → Galaxy → Operations, answering
// the four provenance questions on the general-factual domain. The seed
// (`examples/seed_console_e2e.rs`) plants:
//   * Confirmed: rust type_system="static_strong", svelte-5 paradigm="runes",
//     sqlite concurrency_model="wal" — drives Entity timeline + Galaxy.
//   * Pending:   sqlite default_isolation="wal" — drives Inbox review.
//
// Provenance-questions coverage on the rust entity:
//   1. WHAT        — "type_system = static_strong".
//   2. SOURCE      — provenance kind renders.
//   3. WHEN        — facet renders.
//   4. CONNECTIONS — Galaxy sub-view shows knowledge entities.
//   5. CLIENT      — "console" client facet.

test.describe.configure({ mode: 'serial' })

test.describe('UAT — knowledge domain (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  // ── Home → recent claims load (knowledge proposal is seeded, but may be
  //    beyond the top-5 slice Home caps the list at — `inbox()` returns
  //    proposals in event_seq ASC order and Home takes the first 5; with the
  //    stocks + XSS + project rows seeded earlier, sqlite lands at #6. So
  //    we assert the list is populated from the real API rather than the
  //    specific sqlite row — the Search → Entity test below proves the
  //    knowledge row is in storage).
  test('Home surfaces pending proposals from the real API', async ({ page }) => {
    await expect(page).toHaveURL(/#\/home/)
    await expect(page.getByRole('heading', { name: 'Recent claims' })).toBeVisible()
    const recentButtons = page.locator('.proposal-list').getByRole('button')
    await expect(recentButtons.first()).toBeVisible()
    const count = await recentButtons.count()
    expect(count, 'Home recent list must surface seeded proposals').toBeGreaterThan(0)
  })

  // ── Search "rust" → Entity → ProvenancePanel ─────────────────────────────
  test('Search → Entity: ProvenancePanel answers the 4 questions for rust', async ({
    page,
  }) => {
    await gotoNav(page, 'Search')
    await page.getByLabel('Query').fill('rust')
    await page.getByRole('button', { name: 'Search' }).click()

    const rustCard = page.getByRole('link', { name: /Open entity rust/i }).first()
    await expect(rustCard).toBeVisible()
    await rustCard.click()

    await expect(page.getByRole('heading', { name: 'Entity', level: 1 })).toBeVisible()
    await expect(page.locator('table.claims-table')).toContainText('rust')

    const row = page.locator('table.claims-table tbody tr.row').first()
    await row.click()
    await expect(page.locator('.timeline-list')).toBeVisible()

    const panel = page.locator('.provenance-panel')
    await expect(panel).toBeVisible()

    // ── 4 questions ──────────────────────────────────────────────────────
    // 1. WHAT — type_system = static_strong.
    await expect(panel.getByRole('heading', { name: 'What', level: 4 })).toBeVisible()
    const whatFacet = panel.locator('.facet', { hasText: 'What' })
    await expect(whatFacet).toContainText('type_system')
    await expect(whatFacet).toContainText('static_strong')

    // 2. SOURCE — provenance kind renders.
    const sourceFacet = panel.locator('.facet', { hasText: 'Source' })
    await expect(sourceFacet).toContainText(/From: \w+/)

    // 3. WHEN — facet renders.
    const whenFacet = panel.locator('.facet', { hasText: 'When true' })
    await expect(whenFacet).toContainText(/type_system/)

    // 4. CONNECTIONS facet structurally present.
    await expect(
      panel.getByRole('heading', { name: 'Connections', level: 4 }),
    ).toBeVisible()

    // 5. CLIENT — the "console" client shows.
    const clientFacet = panel.locator('.facet-clients')
    await expect(clientFacet.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Inbox review: sqlite proposal → diff before commit ──────────────────
  test('Inbox: reviewing the sqlite proposal shows a diff before commit', async ({
    page,
  }) => {
    await gotoNav(page, 'Inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()

    const sqliteRow = page.locator('.proposal-head').filter({ hasText: 'sqlite' }).first()
    await sqliteRow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toContainText('default_isolation')
    await expect(dialog.locator('.diff-preview')).toContainText('wal')

    await dialog.getByRole('button', { name: 'No, cancel' }).click()
    await expect(dialog).toHaveCount(0)
  })

  // ── Entity → Galaxy sub-view shows knowledge entities ────────────────────
  test('Entity → Galaxy sub-view renders for the rust knowledge entity', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('rust')
    await page.getByRole('button', { name: 'View' }).click()
    await expect(page.locator('table.claims-table')).toContainText('rust')

    await page.getByRole('group', { name: 'Entity view mode' }).getByRole('button', {
      name: 'Galaxy view',
    }).click()

    await expect(page.getByRole('group', { name: 'Galaxy zoom level' })).toBeVisible()
    await expect(page.locator('.galaxy-canvas')).toBeVisible({ timeout: 5_000 })
  })

  // ── Operations: clients + trust ─────────────────────────────────────────
  test('Operations: trust section loads and clients table shows console', async ({ page }) => {
    await gotoNav(page, 'Operations')
    await expect(
      page.getByRole('heading', { name: 'Operations', level: 1 }),
    ).toBeVisible()

    const trustCard = page.locator('section.card-trust')
    await expect(trustCard).toBeVisible()
    // Fresh knowledge seed has no contradictions — the empty state shows.
    await expect(trustCard.getByText(/No contradictions or stale flags/)).toBeVisible({
      timeout: 5_000,
    })

    const clientsCard = page.locator('section.card-clients')
    await expect(clientsCard).toBeVisible()
    await expect(clientsCard.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Destructive action: retract dialog opens for a knowledge claim ───────
  test('Destructive: retract dialog opens for a rust claim, ESC cancels', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('rust')
    await page.getByRole('button', { name: 'View' }).click()

    const retractDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Retract a claim',
    })
    await retractDetails.locator('summary').click()
    await expect(retractDetails.getByRole('button', { name: /Retract claim/ }).first()).toBeVisible()
    await retractDetails.getByRole('button', { name: /Retract claim/ }).first().click()

    const dialog = page.getByRole('dialog', { name: 'Confirm retract' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })
    await expect(dialog).toContainText('Predicate')
    await expect(dialog).toContainText('reversible')

    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })
})
