import { expect, test, type Page } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'
import { interceptGalaxy, loadFixture, openGalaxyView } from './galaxyBench.ts'

// Phase E Task E2.3 — Galaxy fallback-chain test (DoD: fallback works for
// no-WebGL / reduced-motion / keyboard / list).
//
// Three fallback scenarios + a keyboard-nav sub-test:
//
//   1. No-WebGL: stub `HTMLCanvasElement.prototype.getContext` to return
//      `null` for webgl/webgl2 BEFORE the Galaxy component mounts. The
//      renderer-detection (`webglAvailable()` in galaxyRendererDetect.ts)
//      probes via `canvas.getContext('webgl')` — null result flips it to
//      `false`, and `detectRenderer('3d')` walks the chain to '2d' (or
//      'list' if 2D canvas is also broken, which we don't simulate here).
//      We assert the renderer-badge shows '2d' or 'list' (NOT '3d') AND
//      that the new (Carry 2) `renderer-reason` one-liner appears with the
//      "WebGL unavailable" copy.
//
//   2. Reduced-motion: `page.emulateMedia({ reducedMotion: 'reduce' })`
//      flips `prefers-reduced-motion` to 'reduce'. `detectRenderer('3d')`
//      walks past '3d' to '2d'. We assert renderer-badge is non-'3d' AND
//      the `renderer-reason` line mentions "reduced motion".
//
//   3. List renderer + keyboard: force the list renderer via the Carry 2
//      renderer-kind selector, then Tab through nodes + Enter to select.
//      The list renderer gives each `<li>` `tabindex=0` + role=button +
//      onclick/onkeydown handlers; Tab order is DOM order, so the first
//      Tab into the list lands on the first node.
//
// The fixture for these tests is the small triangle (3 nodes) — fallback
// detection doesn't depend on payload size, so a tiny graph keeps the test
// fast and the assertions readable.

test.describe('Galaxy fallback chain (real backend)', () => {
  test('no-WebGL: renderer-badge is 2d or list, reason shows "WebGL unavailable"', async ({ page }) => {
    await installNoWebGLStub(page)
    await loginAsConsole(page)
    await interceptGalaxy(page, loadFixture('graph-triangle'))
    await openGalaxyView(page)

    const badge = page.locator('.renderer-badge')
    await expect(badge).toBeVisible()
    const kind = ((await badge.textContent()) ?? '').trim()
    expect(kind, 'no-WebGL must degrade past 3d').toMatch(/^(2d|list)$/)

    // Carry 2 — the reasoner one-liner must surface WHY 3D isn't on.
    const reason = page.locator('.renderer-reason')
    await expect(reason).toBeVisible()
    await expect(reason).toContainText('WebGL unavailable')
  })

  test('reduced-motion: renderer-badge is 2d or list, reason mentions reduced motion', async ({
    page,
  }) => {
    // emulateMedia BEFORE mounting so the matchMedia probe sees 'reduce'.
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await loginAsConsole(page)
    await interceptGalaxy(page, loadFixture('graph-triangle'))
    await openGalaxyView(page)

    const badge = page.locator('.renderer-badge')
    await expect(badge).toBeVisible()
    const kind = ((await badge.textContent()) ?? '').trim()
    expect(kind, 'reduced-motion must degrade past 3d').toMatch(/^(2d|list)$/)

    // Carry 2 — the reasoner must mention the reduced-motion trigger.
    const reason = page.locator('.renderer-reason')
    await expect(reason).toBeVisible()
    await expect(reason).toContainText('reduced motion')
  })

  test('list renderer: Tab reaches a node, Enter opens the side-panel', async ({ page }) => {
    await loginAsConsole(page)
    await interceptGalaxy(page, loadFixture('graph-triangle'))
    await openGalaxyView(page)

    // Force the list renderer via the Carry 2 selector. (Without this the
    // list is only the bottom of the fallback chain — explicitly picking it
    // is what the keyboard test wants to exercise.)
    await page.getByRole('button', { name: 'List', exact: true }).click()
    await expect(page.locator('.galaxy-list-nodes')).toBeVisible()

    // Tab INTO the list. The first Tab from outside lands on the first
    // `<li>` (tabindex=0, role=button). We Tab a bounded number of times
    // from the renderer-kind selector button so we don't infinite-loop on
    // a broken tab order.
    await page.getByRole('button', { name: 'List', exact: true }).focus()
    let onNode = false
    for (let i = 0; i < 10 && !onNode; i++) {
      await page.keyboard.press('Tab')
      onNode = await page.evaluate(() => {
        const el = document.activeElement
        return !!el && el.classList.contains('galaxy-list-node')
      })
    }
    expect(onNode, 'a galaxy-list-node must be reachable by Tab').toBe(true)

    // Enter on the focused node fires `callbacks.onNodeClick(node)`, which
    // GalaxyGraph routes to setting `selected` → the `.galaxy-sidepanel`
    // appears with the node's label as its h3.
    await page.keyboard.press('Enter')
    const sidepanel = page.locator('.galaxy-sidepanel')
    await expect(sidepanel).toBeVisible()
    // The triangle's first node label is "Alpha".
    await expect(sidepanel.locator('h3')).toHaveText('Alpha')
  })
})

/**
 * Stub WebGL detection BEFORE the page boots the Galaxy component. We
 * override `HTMLCanvasElement.prototype.getContext` so any 'webgl' /
 * 'webgl2' / 'experimental-webgl' request returns `null`; other context
 * types (most importantly '2d' — used by the 2D renderer) still work.
 *
 * The override must be installed BEFORE the Svelte component mounts;
 * addInitScript does this at the earliest point (before any page script
 * runs). loginAsConsole + openGalaxyView then drive the app as normal.
 *
 * IMPLEMENTATION NOTE: the original `getContext` reads canvas-internal
 * state (the instance, not the prototype), so we must call it with `this`
 * bound to the canvas instance the caller invoked on. Using
 * `proto.getContext.bind(proto)` would lose that, returning `null` for 2D
 * contexts too — which would prevent the 2D fallback renderer from
 * mounting and defeat the test. Instead we save the original function and
 * `.call(this, ...)` it.
 *
 * The stub is per-page (Playwright contexts are isolated); other tests
 * in the suite are NOT affected.
 */
async function installNoWebGLStub(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const proto = HTMLCanvasElement.prototype
    const orig = proto.getContext
    // Cast through `unknown` because the patched signature returns
    // `RenderingContext | null` (a wider union than the canonical overloads
    // on `HTMLCanvasElement.prototype.getContext`); TS won't accept the
    // narrower overloads as the assignment target.
    proto.getContext = function patched(
      this: HTMLCanvasElement,
      contextId: string,
      ...rest: unknown[]
    ): RenderingContext | null {
      if (
        contextId === 'webgl' ||
        contextId === 'webgl2' ||
        contextId === 'experimental-webgl'
      ) {
        return null
      }
      return orig.call(this, contextId, ...rest) as RenderingContext | null
    } as typeof proto.getContext
  })
}
