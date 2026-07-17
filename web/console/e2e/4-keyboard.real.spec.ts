import { expect, test } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'

// Phase E1.4 — keyboard navigation across the Console (DoD #3).
//
// The Console must be fully drivable from the keyboard:
//   - Tab through the primary nav links; Enter activates one.
//   - On the Inbox, Tab reaches a proposal row; Enter opens its detail.
//   - Tab reaches the Approve/Reject/Supersede buttons; Enter activates one
//     (and the confirm dialog's focus trap keeps Tab inside the dialog).
//   - The dialog's Yes button is focused on open so Enter confirms immediately.
//
// We use real Playwright keyboard events (page.keyboard.press) — no
// `page.click` shortcuts — so the assertions reflect the genuine tab order
// the Svelte shell + browser expose to an assistive-tech user.

test.describe('Keyboard navigation (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  test('Tab through nav, Enter to land on Search', async ({ page }) => {
    // Start from the brand h1 (a natural top-of-document focus target).
    // `.first()` disambiguates between the shell-header h1 and the Home page
    // hero h1 (both read "Brain Console"); either is a valid Tab start point.
    await page.getByRole('heading', { name: 'Brain Console', level: 1 }).first().focus()

    // Tab until the Search NAV LINK (an <a href="#/search">) is focused, then
    // activate it. We match the anchor's href, not its text — the Home page
    // also has a "Search" button in its quick-search form, which would false-
    // match a text-based detector. We cap the tab count so a broken focus
    // order fails the test instead of looping forever.
    let onSearchLink = false
    for (let i = 0; i < 30 && !onSearchLink; i++) {
      await page.keyboard.press('Tab')
      const active = await page.evaluate(() => {
        const el = document.activeElement
        // Only an anchor with the search hash counts as the nav link.
        if (el && el.tagName === 'A' && el.getAttribute('href') === '#/search') {
          return 'search-nav-link'
        }
        return ''
      })
      if (active === 'search-nav-link') onSearchLink = true
    }
    expect(onSearchLink, 'Search nav link must be reachable by Tab').toBe(true)

    await page.keyboard.press('Enter')
    await expect(page).toHaveURL(/#\/search/)
    await expect(page.getByRole('heading', { name: 'Search', level: 1 })).toBeVisible()
  })

  test('Inbox: Tab to proposal row, Enter opens detail, Tab to Reject, Enter opens dialog', async ({
    page,
  }) => {
    // Navigate to Inbox via direct hash (covered elsewhere by nav keyboard
    // test; here we focus on the Inbox-internal tab order).
    await page.goto('/#/inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', level: 1 })).toBeVisible()
    await expect(page.locator('.proposal-head').first()).toBeVisible()

    // Tab to the first proposal-head. It has tabindex="0" + role="button".
    let onProposal = false
    for (let i = 0; i < 30 && !onProposal; i++) {
      await page.keyboard.press('Tab')
      onProposal = await page.evaluate(() => {
        const el = document.activeElement
        return !!el && el.classList.contains('proposal-head')
      })
    }
    expect(onProposal, 'a proposal-head must be reachable by Tab').toBe(true)

    // Enter opens the detail panel.
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    // Tab from the proposal into the action buttons. The first action button
    // is Approve (its row order: Approve, Reject, Supersede). We Tab until a
    // button is focused.
    let onActionButton = false
    for (let i = 0; i < 15 && !onActionButton; i++) {
      await page.keyboard.press('Tab')
      onActionButton = await page.evaluate(() => {
        const el = document.activeElement
        if (!el || el.tagName !== 'BUTTON') return false
        const t = el.textContent ?? ''
        return t === 'Approve' || t === 'Reject' || t === 'Supersede'
      })
    }
    expect(onActionButton, 'an action button must be reachable by Tab').toBe(true)

    // Tab to Reject (skip Approve so we don't consume a proposal the other
    // tests rely on differently — Reject here is fine, the inbox-review spec
    // uses GULF/AAPL/PTT and runs in its own file).
    let onReject = false
    for (let i = 0; i < 5 && !onReject; i++) {
      await page.keyboard.press('Tab')
      onReject = await page.evaluate(() => {
        const el = document.activeElement
        return !!el && el.tagName === 'BUTTON' && (el.textContent ?? '') === 'Reject'
      })
    }
    expect(onReject, 'Reject button must be reachable by Tab from the proposal').toBe(true)

    // Enter on Reject opens the confirm dialog.
    await page.keyboard.press('Enter')
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    // DoD: the dialog focuses the Yes button on open (Inbox.svelte focusYes)
    // so Enter alone confirms. Assert Yes is focused.
    const yesBtn = dialog.getByRole('button', { name: 'Yes, confirm' })
    await expect(yesBtn).toBeFocused()
  })
})
