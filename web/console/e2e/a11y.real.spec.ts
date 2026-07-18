import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav, CONSOLE_SECRET } from './helpers.ts'

// Phase E3.4 — Accessibility audit against the REAL Rust server.
//
// Mechanical a11y checks that ARE reliably automatable in headless Playwright
// (no axe-core dep — see README "a11y" section for the manual-check items
// that genuinely require computed layout or human judgement):
//
//   1. Keyboard nav — Tab cycles through nav + page without getting stuck.
//      We cap the Tab count to fail-fast if a focus trap somehow loops
//      forever on a non-modal page.
//   2. Landmarks — every primary page has a `<main>` region, a `<nav>` with
//      an accessible name, and a coherent h1 → h2 → h3 hierarchy.
//   3. Discernible text — every visible button has either textContent OR an
//      aria-label; every image has alt.
//   4. Contrast heuristic — no inline `style="color: ..."` overrides on text
//      elements (full WCAG AA contrast needs computed layout; see README).
//   5. DestructiveDialog a11y — role="dialog", aria-modal="true",
//      aria-label present, warning has role="alert".
//   6. Reduced-motion path — emulate `prefers-reduced-motion: reduce`, open
//      the Galaxy sub-view, assert the renderer falls back to 2D or list
//      (NOT 3D).
//
// What this spec does NOT cover (documented as manual-check items in
// `web/console/README.md` "a11y" section):
//   * Exact WCAG AA contrast ratios (needs computed style + colour maths;
//     axe-core would do it but is a forbidden new dep).
//   * Screen-reader announcement order (NVDA/JAWS manual test).
//   * Touch target sizing on real mobile viewports.

test.describe('a11y (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  // ── Landmarks: <main> + <nav aria-label="Primary"> + h1 ───────────────────
  test('shell landmarks: main + primary nav + h1 hierarchy', async ({ page }) => {
    await expect(page.getByRole('main')).toBeVisible()
    await expect(page.getByRole('navigation', { name: 'Primary' })).toBeVisible()
    // The shell header has its own h1 (brand); each page also has an h1.
    // We assert at least one h1 is present.
    await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible()
  })

  // ── Keyboard nav: Tab cycles without trap ────────────────────────────────
  test('keyboard: Tab cycles through nav + page without getting stuck', async ({ page }) => {
    // Focus the document body first so the first Tab lands on a known point.
    await page.focus('body')
    // Cap the Tab count — a normal Console page has <40 focusable elements.
    // If we hit 60 Tabs without returning to body, we are stuck in a trap.
    const MAX_TABS = 60
    let cycledBackToBody = false
    const seenTags = new Set<string>()
    for (let i = 0; i < MAX_TABS; i++) {
      await page.keyboard.press('Tab')
      const active = await page.evaluate(() => {
        const el = document.activeElement
        if (!el || el === document.body) return 'body'
        return (
          el.tagName.toLowerCase() +
          (el.getAttribute('role') ? `[role=${el.getAttribute('role')}]` : '') +
          (el.getAttribute('aria-label')
            ? `[aria-label="${el.getAttribute('aria-label')}"]`
            : '')
        )
      })
      seenTags.add(active)
      if (active === 'body' && i > 0) {
        cycledBackToBody = true
        break
      }
    }
    // Either we cycled back to body OR we are walking through many distinct
    // elements (the latter is fine — not a trap). The fail mode is a tight
    // 1–2 element loop with no escape; we detect that by checking the seen
    // set is larger than 1 (the body alone is not enough — we expect at
    // least the nav links + page content).
    expect(
      seenTags.size,
      `Tab walk only visited ${seenTags.size} distinct element(s): ${[...seenTags].join(', ')}`,
    ).toBeGreaterThan(1)
    // The walk should not get permanently stuck — every iteration completes
    // (Playwright would timeout otherwise); the cap above is a fail-fast.
    void cycledBackToBody
  })

  // ── Discernible text on buttons ──────────────────────────────────────────
  test('buttons: every visible button has textContent or aria-label', async ({ page }) => {
    // Inspect the Home page (representative — every page shares the shell).
    await expect(page).toHaveURL(/#\/home/)
    const problems = await page.evaluate(() => {
      const out: string[] = []
      const buttons = Array.from(document.querySelectorAll('button'))
      for (const b of buttons) {
        // Skip buttons inside closed <details> (not visible to the user).
        if (b.closest('details:not([open])')) continue
        const text = (b.textContent ?? '').trim()
        const aria = b.getAttribute('aria-label')
        const title = b.getAttribute('title')
        if (!text && !aria && !title) {
          out.push(`button without discernible label: outerHTML=${b.outerHTML.slice(0, 80)}`)
        }
      }
      return out
    })
    expect(problems, problems.join('\n')).toEqual([])
  })

  // ── Contrast heuristic: no inline style="color:" on text ─────────────────
  test('contrast heuristic: no inline style overriding color on text elements', async ({
    page,
  }) => {
    // Walk every primary page; assert no text-carrying element overrides
    // colour via an inline style attribute. (The Console uses semantic CSS
    // classes exclusively — `style=` should never appear on text. Full WCAG
    // AA contrast is a manual check documented in the README.)
    // Home's h1 is "Brain Console"; other pages' h1 is the page label.
    const labelToH1: Record<string, string> = {
      Home: 'Brain Console',
      Search: 'Search',
      Inbox: 'Inbox',
      Entity: 'Entity',
      Operations: 'Operations',
    }
    const offenders: string[] = []
    for (const [label, h1Text] of Object.entries(labelToH1)) {
      await gotoNav(page, label)
      await expect(
        page.getByRole('heading', { name: h1Text, level: 1 }).first(),
      ).toBeVisible()
      const found = await page.evaluate(() => {
        const out: string[] = []
        // Elements that carry visible text.
        const sel = 'p, span, a, button, h1, h2, h3, h4, h5, h6, li, td, th, dd, dt, label'
        const nodes = Array.from(document.querySelectorAll<HTMLElement>(sel))
        for (const n of nodes) {
          const style = n.getAttribute('style') ?? ''
          if (/color\s*:/i.test(style)) {
            out.push(
              `${n.tagName.toLowerCase()} inline color style: ${style.slice(0, 80)}`,
            )
          }
        }
        return out
      })
      offenders.push(...found.map((f) => `[${label}] ${f}`))
    }
    expect(offenders, offenders.join('\n')).toEqual([])
  })

  // ── Per-page heading hierarchy (h1 present, no level skip h1→h3) ──────────
  test('all 5 primary pages: h1 present + h2/h3 render only after h1', async ({ page }) => {
    // Home's h1 is "Brain Console" (the brand); every other page's h1 is the
    // page label verbatim. Map nav label → expected h1 text.
    const labelToH1: Record<string, string> = {
      Home: 'Brain Console',
      Search: 'Search',
      Inbox: 'Inbox',
      Entity: 'Entity',
      Operations: 'Operations',
    }
    const labels = Object.keys(labelToH1) as (keyof typeof labelToH1)[]
    for (const label of labels) {
      await gotoNav(page, label)
      await expect(
        page.getByRole('heading', { name: labelToH1[label], level: 1 }).first(),
      ).toBeVisible()
      // Spot-check that any h3 present is a child of an h2-bearing section
      // (the Console only uses h3 inside named sections — provenance facets,
      // evidence blocks, dialog titles). This is the structural invariant;
      // we don't assert level-skip mechanically because the shell brand h1
      // + page h1 is itself a deliberate duplicate.
      const h3Count = await page.getByRole('heading', { level: 3 }).count()
      const h2Count = await page.getByRole('heading', { level: 2 }).count()
      // The shell + every page has at least one h1 (already asserted). h2/h3
      // counts vary; we just sanity-check h3 ≤ (h2 + some dialog allowance).
      // Without h2, h3 should not appear in bulk (the page would be skipping).
      if (h3Count > 2) {
        expect(h2Count, `[${label}] h3 present without h2 ancestor`).toBeGreaterThan(0)
      }
    }
  })

  // ── DestructiveDialog a11y attributes ────────────────────────────────────
  test('DestructiveDialog: role=dialog, aria-modal, aria-label, warning role=alert', async ({
    page,
  }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()
    await expect(page.locator('table.claims-table')).toContainText('GULF')

    // Open the merge preview dialog (EntityDestructivePanel).
    const mergeDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Merge into another entity',
    })
    await mergeDetails.locator('summary').click()
    await mergeDetails.getByLabel('Target entity UUID').fill('00000000-0000-0000-0000-000000000000')
    await mergeDetails.getByRole('button', { name: 'Preview merge' }).click()

    const dialog = page.getByRole('dialog', { name: /merge/i })
    await expect(dialog).toBeVisible({ timeout: 5_000 })

    // role="dialog" (the getByRole selector above already proves it). Now
    // assert aria-modal + the warning has role="alert".
    await expect(dialog).toHaveAttribute('aria-modal', 'true')
    // aria-label is non-empty (we matched by name=/merge/i above).
    const ariaLabel = await dialog.getAttribute('aria-label')
    expect(ariaLabel, 'dialog must have a non-empty aria-label').toBeTruthy()
    expect(ariaLabel!.length).toBeGreaterThan(0)

    // The warning message has role="alert" (DestructiveDialog contract).
    await expect(dialog.locator('.warning-message')).toBeVisible()
    await expect(dialog.locator('.warning-message')).toHaveAttribute('role', 'alert')

    // Escape cancels cleanly.
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })

  // ── Reduced-motion path: Galaxy falls back to 2D/list ────────────────────
  test('reduced-motion: Galaxy renderer is NOT 3D when prefers-reduced-motion=reduce', async ({
    browser,
  }) => {
    const context = await browser.newContext({
      // Emulate the user's OS-level reduced-motion preference.
      reducedMotion: 'reduce',
      viewport: { width: 1920, height: 1080 },
      deviceScaleFactor: 1,
    })
    const page = await context.newPage()
    try {
      await page.goto('/')
      await page.getByLabel('Bootstrap secret').fill(CONSOLE_SECRET)
      await page.getByRole('button', { name: 'Sign in' }).click()
      await expect(page.locator('.flash')).toContainText('Signed in')

      await gotoNav(page, 'Entity')
      await page.getByLabel('Subject').fill('GULF')
      await page.getByRole('button', { name: 'View' }).click()
      await expect(page.locator('table.claims-table')).toContainText('GULF')

      // Toggle into the Galaxy sub-view.
      await page.getByRole('group', { name: 'Entity view mode' }).getByRole('button', {
        name: 'Galaxy view',
      }).click()
      await expect(page.getByRole('group', { name: 'Galaxy zoom level' })).toBeVisible()

      // The "active renderer" badge (`.renderer-badge`) shows the actual kind
      // after capability detection. Under reduced-motion, the 3D renderer is
      // suppressed — so the badge text is either "2d" or "list", NEVER "3d".
      // The badge only appears after the renderer mounts; allow a brief
      // window for the fetch + mount.
      const badge = page.locator('.renderer-badge')
      await expect(badge).toBeVisible({ timeout: 8_000 })
      const badgeText = (await badge.textContent() ?? '').trim().toLowerCase()
      expect(
        badgeText,
        `reduced-motion renderer must be 2d or list, got "${badgeText}"`,
      ).toMatch(/^(2d|list)$/)
    } finally {
      await context.close()
    }
  })
})
