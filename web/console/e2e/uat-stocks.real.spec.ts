import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E3.4 — UAT spec for the "stocks" domain (real backend).
//
// THE DoD #1 TEST for the stocks domain: walks Home → Search → Entity →
// Inbox-review → Galaxy → Operations, answering the four core provenance
// questions (What / Source / When / Client) AND exercising one destructive
// action (retract — reversible). The other UAT specs (`uat-project`,
// `uat-knowledge`) cover the same walk on their domains.
//
// Seed dependency (see `examples/seed_console_e2e.rs`):
//   * Confirmed GULF target_price=55 (drives Entity timeline + Galaxy).
//   * Pending GULF target_price=58, PTT target_price=62, AAPL sector="tech"
//     (drives Inbox review). The AAPL row is consumed by `6-inbox-review`,
//     so this spec reviews the PTT row to avoid cross-spec interference
//     (every spec uses a distinct pending row).
//
// Provenance-questions coverage on the GULF entity:
//   1. WHAT        — ProvenancePanel "What" facet shows "target_price = 55".
//   2. SOURCE      — ProvenancePanel "Source" facet shows a provenance kind
//                    (evidence / inference / user_assertion / mechanical).
//   3. WHEN        — ProvenancePanel "When true" facet shows either a valid
//                    interval or "current scope — no time bounds".
//   4. CLIENT      — ProvenancePanel "Client that edited" facet shows the
//                    "console" client registered at login (or the
//                    operations-page-loaded fallback). The "Connections"
//                    facet is structurally present (driven by Galaxy).
//
// Destructive action: retract dialog opens for the GULF target_price claim
// (reversible — plain confirm, ESC cancels so we don't mutate shared state).

test.describe.configure({ mode: 'serial' })

test.describe('UAT — stocks domain (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  // ── Home → recent claims load from the real API ──────────────────────────
  test('Home surfaces pending proposals from the real API', async ({ page }) => {
    await expect(page).toHaveURL(/#\/home/)
    await expect(page.getByRole('heading', { name: 'Recent claims' })).toBeVisible()
    // The "Recent claims" list reflects the live inbox. 6-inbox-review.spec
    // consumes GULF (reject) and AAPL (approve) but leaves PTT (cancel),
    // so by the time this spec runs only PTT + the XSS row + the UAT
    // project/knowledge rows remain. We assert the list is populated from
    // the real API AND that PTT is one of the visible rows — both are
    // observable contracts of the Home → real-API wiring regardless of
    // the prior spec's writes.
    const recentButtons = page.locator('.proposal-list').getByRole('button')
    await expect(recentButtons.first()).toBeVisible()
    await expect(page.getByRole('button', { name: /PTT/i }).first()).toBeVisible()
  })

  // ── Search "gulf" → Entity → ProvenancePanel ─────────────────────────────
  test('Search → Entity: ProvenancePanel answers the 4 questions for GULF', async ({
    page,
  }) => {
    await gotoNav(page, 'Search')
    await page.getByLabel('Query').fill('gulf')
    await page.getByRole('button', { name: 'Search' }).click()

    const gulfCard = page.getByRole('link', { name: /Open entity GULF/i })
    await expect(gulfCard).toBeVisible()
    await gulfCard.click()

    await expect(page.getByRole('heading', { name: 'Entity', level: 1 })).toBeVisible()
    await expect(page.locator('table.claims-table')).toContainText('GULF')
    await expect(page.locator('table.claims-table')).toContainText('target_price')
    await expect(page.locator('table.claims-table')).toContainText('55')

    // Expand the GULF row to fetch the timeline (the ProvenancePanel reads
    // the timeline map the parent derives from the expanded state).
    const row = page.locator('table.claims-table tbody tr.row').first()
    await row.click()
    await expect(page.locator('.timeline-list')).toBeVisible()

    // ProvenancePanel mounts only when an entity_id is on the loaded claims
    // (the seed's confirmed GULF claim has one).
    const panel = page.locator('.provenance-panel')
    await expect(panel).toBeVisible()

    // ── 4 questions ──────────────────────────────────────────────────────
    // 1. WHAT — predicate + value render.
    await expect(panel.getByRole('heading', { name: 'What', level: 4 })).toBeVisible()
    await expect(panel.locator('.facet', { hasText: 'What' })).toContainText('target_price')
    await expect(panel.locator('.facet', { hasText: 'What' })).toContainText('55')

    // 2. SOURCE — provenance kind renders (non-empty "From: …" line OR the
    //    "No provenance recorded." empty branch; for a confirmed claim with
    //    captured evidence the non-empty branch is the expected path).
    await expect(panel.getByRole('heading', { name: 'Source', level: 4 })).toBeVisible()
    const sourceFacet = panel.locator('.facet', { hasText: 'Source' })
    await expect(sourceFacet).toContainText(/From: \w+/)

    // 3. WHEN — the timeline has a confirmed entry, so either a valid
    //    interval renders OR the "current scope — no time bounds" hint
    //    (both are legitimate answers — we assert the facet renders at all).
    await expect(panel.getByRole('heading', { name: 'When true', level: 4 })).toBeVisible()
    const whenFacet = panel.locator('.facet', { hasText: 'When true' })
    await expect(whenFacet).toContainText(/target_price/)

    // 4. CONNECTIONS facet is structurally present (Galaxy is the deep answer;
    //    the panel surfaces the audit client-edit facet below it).
    await expect(
      panel.getByRole('heading', { name: 'Connections', level: 4 }),
    ).toBeVisible()

    // 5. CLIENT THAT EDITED — the seed + login register a "console" client.
    //    The Entity page refreshes /ops/clients best-effort, so allow a brief
    //    window for the client facet to populate.
    const clientFacet = panel.locator('.facet-clients')
    await expect(clientFacet.getByRole('heading', { name: /Client that edited/ })).toBeVisible()
    await expect(clientFacet.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Inbox review: PTT proposal → diff before commit ─────────────────────
  test('Inbox: reviewing the PTT stocks proposal shows a diff before commit', async ({
    page,
  }) => {
    await gotoNav(page, 'Inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()

    // Open the PTT proposal detail (distinct from the GULF/AAPL rows used
    // elsewhere in the suite — every consuming spec picks its own row).
    const pttRow = page.locator('.proposal-head').filter({ hasText: 'PTT' }).first()
    await pttRow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    // Click Reject — opens the confirm dialog. The dialog MUST show a diff
    // (DoD #2 — diff-before-commit invariant) before any commit.
    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toContainText('target_price')
    await expect(dialog.locator('.diff-preview')).toContainText('62')

    // Cancel — do NOT mutate shared state; other specs rely on the PTT row.
    await dialog.getByRole('button', { name: 'No, cancel' }).click()
    await expect(dialog).toHaveCount(0)
  })

  // ── Entity → Galaxy sub-view toggle ──────────────────────────────────────
  test('Entity → Galaxy sub-view renders for the GULF entity', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()
    await expect(page.locator('table.claims-table')).toContainText('GULF')

    // Toggle into the Galaxy sub-view (still on the Entity page — the 5-page
    // ConsolePage enum is unchanged; Galaxy is a sub-view).
    await page.getByRole('group', { name: 'Entity view mode' }).getByRole('button', {
      name: 'Galaxy view',
    }).click()

    // GalaxyGraph mounts its renderer. We assert the toolbar + canvas/list
    // stage are present (renderer kind depends on WebGL availability).
    await expect(page.getByRole('group', { name: 'Galaxy zoom level' })).toBeVisible()
    await expect(page.locator('.galaxy-canvas')).toBeVisible({ timeout: 5_000 })
  })

  // ── Operations: trust section surfaces the stocks data ───────────────────
  test('Operations: trust section loads and clients table shows console', async ({ page }) => {
    await gotoNav(page, 'Operations')
    await expect(
      page.getByRole('heading', { name: 'Operations', level: 1 }),
    ).toBeVisible()

    // Trust card is present (empty for a fresh stocks-only seed — no
    // contradictions yet).
    const trustCard = page.locator('section.card-trust')
    await expect(trustCard).toBeVisible()
    await expect(trustCard.getByText(/No contradictions or stale flags/)).toBeVisible({
      timeout: 5_000,
    })

    // Clients table shows the "console" client (registered at login).
    const clientsCard = page.locator('section.card-clients')
    await expect(clientsCard).toBeVisible()
    await expect(clientsCard.getByText('console').first()).toBeVisible({ timeout: 5_000 })
  })

  // ── Destructive action: retract dialog opens (reversible) ────────────────
  test('Destructive: retract dialog opens for the GULF claim, ESC cancels', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()

    const retractDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Retract a claim',
    })
    await retractDetails.locator('summary').click()
    // The retract list has the GULF target_price claim.
    await expect(retractDetails.getByRole('button', { name: /Retract claim/ }).first()).toBeVisible()
    await retractDetails.getByRole('button', { name: /Retract claim/ }).first().click()

    // Retract is reversible — its own plain dialog (NOT the destructive
    // dialog with warning/preview). Distinct aria-label.
    const dialog = page.getByRole('dialog', { name: 'Confirm retract' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })
    await expect(dialog).toContainText('Predicate')
    await expect(dialog).toContainText('reversible')

    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })
})
