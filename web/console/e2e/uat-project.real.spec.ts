import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E3.4 — UAT spec for the "project" domain (real backend).
//
// Walks Home → Search → Entity → Inbox-review → Operations, answering the
// four provenance questions on a domain that is NOT stocks. The seed
// (`examples/seed_console_e2e.rs`) plants:
//   * Confirmed: phase-e status="in_progress", phase-e task_count=14,
//     phase-d status="closed" — drives Entity timeline + ProvenancePanel.
//   * Pending:   phase-e risk="schedule" — drives Inbox review.
//
// Provenance-questions coverage on the phase-e entity:
//   1. WHAT        — "status = in_progress" and/or "task_count = 14".
//   2. SOURCE      — provenance kind renders.
//   3. WHEN        — facet renders (interval or "no time bounds").
//   4. CLIENT      — "console" client facet.
//
// Destructive action: retract dialog opens for a phase-e claim (reversible).
// Galaxy view: not asserted here for project — the Galaxy materializer lives
// off claim edges; the project seed has no edges, so the Galaxy sub-view
// would show "No nodes at this zoom" (still a valid fallback). We DO assert
// the Galaxy toggle is present so the sub-view is reachable.

test.describe.configure({ mode: 'serial' })

test.describe('UAT — project domain (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  // ── Home → recent claims load from the real API ──────────────────────────
  test('Home surfaces pending proposals from the real API', async ({ page }) => {
    await expect(page).toHaveURL(/#\/home/)
    await expect(page.getByRole('heading', { name: 'Recent claims' })).toBeVisible()
    // The "Recent claims" list caps at 5; the project proposal lands at #5
    // (event_seq ASC, after the 4 stocks rows), so phase-e risk is visible
    // here on a fresh seed. 6-inbox-review.spec consumes GULF/AAPL but the
    // UAT specs run AFTER it; since 6-inbox-review does NOT touch phase-e,
    // it stays. We assert the list is populated AND that phase-e is one of
    // the visible rows.
    const recentButtons = page.locator('.proposal-list').getByRole('button')
    await expect(recentButtons.first()).toBeVisible()
    await expect(page.getByRole('button', { name: /phase-e/i }).first()).toBeVisible()
  })

  // ── Search "phase-e" → Entity → ProvenancePanel ──────────────────────────
  test('Search → Entity: ProvenancePanel answers the 4 questions for phase-e', async ({
    page,
  }) => {
    await gotoNav(page, 'Search')
    await page.getByLabel('Query').fill('phase-e')
    await page.getByRole('button', { name: 'Search' }).click()

    // The Search hits include the phase-e rows.
    const phaseECard = page.getByRole('link', { name: /Open entity phase-e/i }).first()
    await expect(phaseECard).toBeVisible()
    await phaseECard.click()

    await expect(page.getByRole('heading', { name: 'Entity', level: 1 })).toBeVisible()
    await expect(page.locator('table.claims-table')).toContainText('phase-e')

    // Expand the first row to fetch the timeline (ProvenancePanel reads the
    // parent's derived timeline map).
    const row = page.locator('table.claims-table tbody tr.row').first()
    await row.click()
    await expect(page.locator('.timeline-list')).toBeVisible()

    const panel = page.locator('.provenance-panel')
    await expect(panel).toBeVisible()

    // ── 4 questions ──────────────────────────────────────────────────────
    // 1. WHAT — at least one of status / task_count renders.
    await expect(panel.getByRole('heading', { name: 'What', level: 4 })).toBeVisible()
    const whatFacet = panel.locator('.facet', { hasText: 'What' })
    await expect(whatFacet).toContainText(/status|task_count/)

    // 2. SOURCE — provenance kind renders.
    const sourceFacet = panel.locator('.facet', { hasText: 'Source' })
    await expect(sourceFacet).toContainText(/From: \w+/)

    // 3. WHEN — facet renders.
    const whenFacet = panel.locator('.facet', { hasText: 'When true' })
    await expect(whenFacet).toContainText(/status|task_count/)

    // 4. CONNECTIONS facet structurally present.
    await expect(
      panel.getByRole('heading', { name: 'Connections', level: 4 }),
    ).toBeVisible()

    // 5. CLIENT — the "console" client shows in the audit facet.
    const clientFacet = panel.locator('.facet-clients')
    await expect(clientFacet.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Inbox review: phase-e risk proposal → diff before commit ─────────────
  test('Inbox: reviewing the phase-e risk proposal shows a diff before commit', async ({
    page,
  }) => {
    await gotoNav(page, 'Inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()

    const phaseERow = page.locator('.proposal-head').filter({ hasText: 'phase-e' }).first()
    await phaseERow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    // DiffPreview must render before any commit (DoD #2 invariant).
    await expect(dialog.locator('.diff-preview')).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toContainText('risk')
    await expect(dialog.locator('.diff-preview')).toContainText('schedule')

    // Cancel — do NOT mutate shared state.
    await dialog.getByRole('button', { name: 'No, cancel' }).click()
    await expect(dialog).toHaveCount(0)
  })

  // ── Entity → Galaxy sub-view toggle is reachable ─────────────────────────
  test('Entity → Galaxy sub-view toggle is reachable for phase-e', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('phase-e')
    await page.getByRole('button', { name: 'View' }).click()
    await expect(page.locator('table.claims-table')).toContainText('phase-e')

    // The Galaxy toggle is present and enabled (claims are loaded).
    const galaxyBtn = page.getByRole('group', { name: 'Entity view mode' }).getByRole('button', {
      name: 'Galaxy view',
    })
    await expect(galaxyBtn).toBeVisible()
    await expect(galaxyBtn).toBeEnabled()
  })

  // ── Operations: backup health Phase F callout + clients ──────────────────
  test('Operations: backup health shows Phase F callout; clients show console', async ({
    page,
  }) => {
    await gotoNav(page, 'Operations')
    await expect(
      page.getByRole('heading', { name: 'Operations', level: 1 }),
    ).toBeVisible()

    // Backup health surfaces the honest "Phase F" callout (E3.3 contract:
    // last_restore_drill_ok=false until the drill harness ships).
    const backupCard = page.locator('section.card-backup')
    await expect(backupCard).toBeVisible()
    await expect(backupCard.getByText('not yet drilled — Phase F')).toBeVisible({
      timeout: 5_000,
    })

    // Clients table shows the "console" client (registered at login).
    const clientsCard = page.locator('section.card-clients')
    await expect(clientsCard).toBeVisible()
    await expect(clientsCard.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Destructive action: retract dialog opens for a phase-e claim ─────────
  test('Destructive: retract dialog opens for a phase-e claim, ESC cancels', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('phase-e')
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
