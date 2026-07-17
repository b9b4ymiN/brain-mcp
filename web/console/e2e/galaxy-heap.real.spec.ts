import { expect, test } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'
import { interceptGalaxy, loadFixture, openGalaxyView } from './galaxyBench.ts'

// Phase E Task E2.3 — Galaxy heap-growth test (§13 Task 5.2 DoD #4).
//
// Mounts GalaxyGraph against the 1k fixture, then runs 20 cycles of:
//   reload (zoom toggle → fresh mount/teardown of the renderer) → sample heap.
// Asserts JS heap growth ≤ 10 % over the run.
//
// Heap is sampled via `performance.memory.usedJSHeapSize` — a Chromium-only
// API (the real-backend project uses Chromium, so this is fine). If the API
// is undefined in this Chromium config (some headless builds ship without
// it; CI runners may disable it), the test SKIPs with a clear message
// instead of failing — the dev-bench policy explicitly accepts skip-with-
// message as a green outcome for unavailable-but-optional capabilities.
//
// WebGL leak defence (Carry 1, this commit): GalaxyGraph's `onDestroy` now
// calls `abortController?.abort()` AND sets a `destroyed` flag so an in-
// flight fetch can't resolve into a torn-down component. The renderer
// itself (`galaxyRenderer3d._destructor()`) was already releasing WebGL
// resources in E2.2; this test is the safety net against regressions in
// both layers.

const CYCLES = 20
const HEAP_GROWTH_PCT = 10

test.describe('Galaxy heap growth (real backend)', () => {
  test('usedJSHeapSize grows ≤10% over 20 mount/reload cycles', async ({ page }) => {
    // Probe `performance.memory` up front — skip the whole test if the API
    // isn't there. The skip is a deliberate soft outcome (NOT a failure)
    // because the API is Chromium-specific and optional even within Chromium.
    const memoryAvailable = await page.evaluate(() => {
      const p = performance as Performance & { memory?: unknown }
      return typeof p.memory === 'object' && p.memory !== null
    })
    test.skip(!memoryAvailable, 'performance.memory unavailable in this Chromium build — skipping heap test')

    await loginAsConsole(page)
    const fixture = loadFixture('graph-1k')
    await interceptGalaxy(page, fixture)
    await openGalaxyView(page)

    // Initial heap baseline — read AFTER the first mount so the per-renderer
    // module load + three.js scene allocation is in the baseline (otherwise
    // the first cycle's "growth" would be inflated by lazy module loading,
    // which is a one-off cost, not a leak).
    // Tiny GC settle pause so any post-mount allocations are accounted for.
    await page.evaluate(() => new Promise((r) => setTimeout(r, 500)))
    const initial = await readUsedHeap(page)
    expect(initial, 'initial heap must be readable').toBeGreaterThan(0)
    console.log(`[BENCH-HEAP] initial usedJSHeapSize = ${(initial / 1024 / 1024).toFixed(1)} MB`)

    // 20 cycles of: zoom toggle → wait for re-mount → sample heap.
    // Each toggle triggers GalaxyGraph.reload() which aborts the prior
    // fetch, tears down the prior renderer, and mounts a new one — exactly
    // the path that leaks if WebGL cleanup is wrong.
    const buttons = ['Far', 'Mid', 'Close'] as const
    let lastHeap = initial
    for (let i = 0; i < CYCLES; i++) {
      const label = buttons[i % buttons.length]
      await page.getByRole('button', { name: label }).click()
      // Wait for the renderer to actually come back (badge is torn down on
      // reload, re-added on mount).
      await expect(page.locator('.renderer-badge')).toBeVisible({ timeout: 5_000 })
      // Small settle for GC. We DON'T force a major GC (Playwright doesn't
      // expose --js-flags='--expose-gc' by default); we rely on the
      // increments to be small per cycle so the cumulative drift is the
      // meaningful signal.
      await page.evaluate(() => new Promise((r) => setTimeout(r, 200)))
      lastHeap = await readUsedHeap(page)
    }

    const growthPct = ((lastHeap - initial) / initial) * 100
    console.log(
      `[BENCH-HEAP] final usedJSHeapSize = ${(lastHeap / 1024 / 1024).toFixed(1)} MB  ` +
        `(growth = ${growthPct.toFixed(2)}% over ${CYCLES} cycles; threshold ${HEAP_GROWTH_PCT}%)`,
    )

    // Soft-assert: the dev-bench policy (bench/README.md) records + flags
    // but doesn't hard-fail. We DO assert here because heap growth is a
    // deterministic invariant (unlike FPS) — a regression above 10% means
    // a real leak and should fail locally + in CI. The skip-with-message
    // branch above is the escape hatch for "API unavailable".
    expect(
      growthPct,
      `heap grew ${growthPct.toFixed(2)}% over ${CYCLES} cycles — threshold ${HEAP_GROWTH_PCT}%`,
    ).toBeLessThanOrEqual(HEAP_GROWTH_PCT)
  })
})

/** Read `performance.memory.usedJSHeapSize` (bytes). Throws if unavailable. */
async function readUsedHeap(page: import('@playwright/test').Page): Promise<number> {
  const bytes = await page.evaluate(() => {
    const p = performance as Performance & {
      memory?: { usedJSHeapSize: number }
    }
    if (!p.memory) throw new Error('performance.memory unavailable')
    return p.memory.usedJSHeapSize
  })
  return bytes
}
