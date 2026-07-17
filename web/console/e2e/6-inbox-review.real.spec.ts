import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E1.4 — Inbox review workflow against the REAL Rust server.
//
// THE DoD #2 TEST: approve/reject/supersede must show evidence + a diff
// BEFORE the mutation commits. The Inbox page's confirm dialog is the single
// chokepoint (see Inbox.svelte `openDialog()` / `confirmDialog()`); no path
// calls the mutation API without first rendering the dialog with a DiffPreview.
//
// This spec proves that end-to-end against real storage:
//   1. Open the inbox → seeded pending proposals appear (GULF, PTT, AAPL, XSS).
//   2. Open one proposal's detail → evidence excerpt + prior-claims note show.
//   3. Click Approve → the dialog opens WITH a diff visible BEFORE we click
//      the confirm Yes button.
//   4. Confirm → success flash + the proposal leaves the list.
//   5. Repeat for Reject on a DISTINCT proposal (no cross-test interference).
//
// Each test uses a distinct proposal so approving/rejecting one does not
// starve the other. The seed runs once per Playwright webServer start, so the
// full inbox is replenished if the server restarts; within a single server
// lifetime the proposals consumed here are gone, but tests run in definition
// order within a file and we pick distinct rows.
//
// Pin intra-file order to serial: test 3 approves AAPL (consumes it), test 4
// rejects GULF (consumes it), test 5 cancels on PTT (leaves it), and tests 1-2
// rely on all rows being present. With `workers: 1` this is already the
// behavior, but `mode: 'serial'` makes the intent explicit and survives any
// future `--shuffle` or `fullyParallel` flip. (This is what the config comment
// at playwright.config.ts ~L114 refers to.)
test.describe.configure({ mode: 'serial' })

test.describe('Inbox review (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
    await gotoNav(page, 'Inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()
  })

  test('lists the seeded pending proposals', async ({ page }) => {
    const list = page.getByRole('list', { name: 'Pending proposals' })
    await expect(list).toBeVisible()
    await expect(list.getByText('GULF').first()).toBeVisible()
    await expect(list.getByText('PTT').first()).toBeVisible()
    await expect(list.getByText('AAPL').first()).toBeVisible()
  })

  test('opening a proposal shows evidence excerpt before any action', async ({ page }) => {
    // The PTT proposal's evidence text was "PTT target price set to 62 ...".
    const pttRow = page.locator('.proposal-head').filter({ hasText: 'PTT' }).first()
    await pttRow.click()

    // Detail panel: Evidence heading + the excerpt blockquote (text-bound).
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()
    const excerpt = page.locator('blockquote.excerpt')
    await expect(excerpt).toBeVisible()
    await expect(excerpt).toContainText('PTT')
    // Approve/Reject/Supersede action buttons render (Supersede may be
    // disabled if there's no prior confirmed claim in scope for PTT — that's
    // fine, we only assert Approve + Reject are actionable).
    await expect(page.getByRole('button', { name: 'Approve' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Reject' })).toBeVisible()
  })

  test('DoD #2: approve shows a diff dialog BEFORE commit, then succeeds', async ({ page }) => {
    // Pick the AAPL sector proposal (distinct from the GULF/PTT rows used
    // elsewhere). Open its detail panel first.
    const aaplRow = page.locator('.proposal-head').filter({ hasText: 'AAPL' }).first()
    await aaplRow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    // Click Approve — opens the confirm dialog. The dialog MUST show a diff
    // (or the "new claim" null→value row) BEFORE the user confirms.
    await page.getByRole('button', { name: 'Approve' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm approve' })
    await expect(dialog).toBeVisible()
    // DiffPreview renders as an ARIA table (`role="table"` on a <div>, not a
    // real <table> element) inside a `.diff-preview` section. The proposal
    // introduces sector="tech" (no prior), so the diff shows a null → "tech"
    // row. Asserting the diff section + the field label is present is the
    // load-bearing DoD #2 assertion: the diff is visible while the Yes
    // button is still unclicked.
    await expect(dialog.locator('.diff-preview')).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toContainText('sector')
    await expect(dialog.locator('.diff-preview')).toContainText('tech')
    // Confirm prompt present (proves we're on the pre-commit screen).
    await expect(
      dialog.getByText('Approve this proposal and commit the new claim?'),
    ).toBeVisible()

    // NOW commit.
    await dialog.getByRole('button', { name: 'Yes, confirm' }).click()

    // Success flash (App.svelte renders the flash on a session store update).
    // The `.flash` class targets the banner specifically — StateBox's loading
    // paragraph also uses role="status", so a role selector would be ambiguous.
    await expect(page.locator('.flash')).toContainText(/Approved proposal/)
    // Dialog closes.
    await expect(dialog).toHaveCount(0)
    // The AAPL row leaves the list (refreshList runs after finalize).
    await expect(page.locator('.proposal-head').filter({ hasText: 'AAPL' })).toHaveCount(0)
  })

  test('DoD #2: reject shows a diff dialog BEFORE commit, then succeeds', async ({ page }) => {
    // Use the GULF target_price=58 proposal (distinct from AAPL above).
    const gulfRow = page.locator('.proposal-head').filter({ hasText: 'GULF' }).first()
    await gulfRow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    // Reject ALSO renders the diff before commit (same DiffPreview component —
    // `.diff-preview` section, ARIA table, not a real <table>).
    await expect(dialog.locator('.diff-preview')).toBeVisible()
    await expect(dialog.locator('.diff-preview')).toContainText('target_price')
    await expect(dialog.locator('.diff-preview')).toContainText('58')
    await expect(
      dialog.getByText('Reject this proposal? The decision is permanent.'),
    ).toBeVisible()

    await dialog.getByRole('button', { name: 'Yes, confirm' }).click()

    await expect(page.locator('.flash')).toContainText(/Rejected proposal/)
    await expect(dialog).toHaveCount(0)
    await expect(page.locator('.proposal-head').filter({ hasText: 'GULF' })).toHaveCount(0)
  })

  test('No, cancel returns to the inbox without committing', async ({ page }) => {
    const pttRow = page.locator('.proposal-head').filter({ hasText: 'PTT' }).first()
    await pttRow.click()
    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()

    await dialog.getByRole('button', { name: 'No, cancel' }).click()
    await expect(dialog).toHaveCount(0)
    // PTT row is still present — no mutation fired.
    await expect(page.locator('.proposal-head').filter({ hasText: 'PTT' })).toHaveCount(1)
  })
})
