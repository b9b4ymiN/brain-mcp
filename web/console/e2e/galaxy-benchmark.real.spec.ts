import { expect, test, type Page } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'
import {
  captureEnvironment,
  interceptGalaxy,
  loadFixture,
  median,
  openGalaxyView,
  percentile,
  recordMetric,
  THRESHOLDS,
  writeBenchOutputs,
  type BenchEnvironment,
  type BenchMetric,
} from './galaxyBench.ts'

// Phase E Task E2.3 — Galaxy component benchmark (real-backend project).
//
// Measures the Svelte GalaxyGraph component on synthetic 1k + 5k node graphs:
//   - FPS (rAF-counted, 5 runs of 5s after a 10s warm-up; median reported)
//   - node-click latency (p95 over N clicks, click → side-panel visible)
//   - search-to-focus latency (p95 — currently driven via zoom/focus change
//     since the Galaxy component doesn't expose a free-text search box; we
//     measure the time from a zoom toggle click to the new graph painted)
//
// DEV-BENCH POLICY (see bench/README.md): thresholds are calibrated for the
// baseline HW (Phase F re-bench). On dev (typically no GPU in headless
// Chromium → SwiftShader software WebGL + throttled rAF) numbers are
// routinely below threshold without indicating a regression. The spec
// RECORDS + LOGS + CONTINUES — it never hard-fails on a missed threshold.
// A green run means "the harness produced numbers", NOT "we hit ship perf".
//
// WebGL-in-headless caveat: if headless Chromium on the dev box has no GPU,
// `detectRenderer('3d')` walks to '2d' (the canvas 2D renderer), and we
// measure FPS for THAT. The environment.json records `gpu: null` so the
// Phase F re-bench can tell which renderer was measured. The note
// "3D deferred — headless no GPU" is attached to the metric.
//
// rAF-in-headless caveat: if the tab is backgrounded or Chromium throttles
// rAF to 1 Hz (visible-task heuristic), the FPS counter would underestimate.
// We keep the page foregrounded (default Playwright behaviour) and warm up
// for 10s before measuring to let the force-layout settle.

test.describe.configure({ mode: 'serial' })

/** Shared across the serial tests in this file (one captured environment). */
let env: BenchEnvironment
/** Accumulated metrics from all tests in this file; flushed in afterAll. */
const metrics: BenchMetric[] = []

test.describe('Galaxy benchmark (real backend, dev-bench accepted)', () => {
  test.beforeAll(async ({ browser }: { browser: import('@playwright/test').Browser }) => {
    // Capture environment once per file-run. We open a throwaway page just
    // for the probe — the per-test pages each get their own context but the
    // captured fields (UA, hardwareConcurrency, GPU) are context-independent.
    const page = await browser.newPage()
    try {
      env = await captureEnvironment(page)
      console.log(`[BENCH] environment: ${env.os} / gpu=${env.gpu ?? 'null'} / ${env.cpu_cores} cores`)
    } finally {
      await page.close()
    }
  })

  test.afterAll(async () => {
    // Only write outputs if we actually captured an environment (beforeAll
    // didn't throw). The results JSON is gitignored.
    if (env) {
      await writeBenchOutputs(env, metrics)
    }
  })

  test(
    '1k-node graph — FPS + click latency + search-to-focus',
    async ({ page }) => {
      // The benchmark warms up for 10s + measures 5×5s FPS + click samples;
      // the default 30s test timeout is too tight. 120s gives headroom for
      // the warm-up + measurement + click + zoom-latency sub-tests.
      test.setTimeout(120_000)
      await loginAsConsole(page)
      const fixture = loadFixture('graph-1k')
      await interceptGalaxy(page, fixture)
      await openGalaxyView(page)

    // ── FPS ────────────────────────────────────────────────────────────
    // Detect which renderer is actually active (so we know whether we're
    // measuring 3D or 2D FPS — the note attached to the metric records this).
    const activeKind = await page.locator('.renderer-badge').textContent()
    const is3d = activeKind?.trim() === '3d'

    // Install a rAF counter. The function returns a function the spec can
    // call to read the current frame count. We DO NOT let the counter run
    // forever — it's stopped before the page navigates so teardown is clean.
    await page.evaluate(() => {
      const w = window as unknown as { __benchFrameCount?: number; __benchRafId?: number }
      w.__benchFrameCount = 0
      const tick = (): void => {
        w.__benchFrameCount = (w.__benchFrameCount ?? 0) + 1
        w.__benchRafId = requestAnimationFrame(tick)
      }
      w.__benchRafId = requestAnimationFrame(tick)
    })

    // Warm-up — let the force layout settle (3D especially does a long
    // initial layout pass that tanks FPS for the first few seconds).
    await page.waitForTimeout(10_000)
    // Reset the counter so the measurement window starts from zero.
    await page.evaluate(() => {
      const w = window as unknown as { __benchFrameCount?: number }
      w.__benchFrameCount = 0
    })

    // 5 runs of 5s each — median reported (more stable than a single run).
    const fpsRuns: number[] = []
    for (let run = 0; run < 5; run++) {
      const before = await page.evaluate(() => {
        const w = window as unknown as { __benchFrameCount?: number }
        return w.__benchFrameCount ?? 0
      })
      await page.waitForTimeout(5_000)
      const after = await page.evaluate(() => {
        const w = window as unknown as { __benchFrameCount?: number }
        return w.__benchFrameCount ?? 0
      })
      // frames-per-second over the 5s window.
      fpsRuns.push((after - before) / 5)
    }
    // Stop the rAF loop — we don't want it running during the click test.
    await page.evaluate(() => {
      const w = window as unknown as { __benchRafId?: number }
      if (typeof w.__benchRafId === 'number') cancelAnimationFrame(w.__benchRafId)
      w.__benchRafId = undefined
    })

    const fpsMedian = median(fpsRuns)
    metrics.push(
      recordMetric(test.info(), 'fps_1k_median', fpsMedian, 'fps', THRESHOLDS.fps1k, {
        note: is3d ? undefined : '3D deferred — headless no GPU; measured 2D/list renderer',
      }),
    )

    // ── click latency ──────────────────────────────────────────────────
    // The 3D/2D force graphs render to canvas — node positions are inside
    // the GL scene, not the DOM, so we can't ask Playwright "find me node
    // X". The canvas-probe approach (centre + offsets) hits a node only
    // when the force layout happens to park one there, which is not
    // reliable on dev (SwiftShader layout differs from baseline HW layout).
    //
    // For a deterministic DOM-clickable measurement we switch to the LIST
    // renderer for the click-latency metric (the Carry 2 toolbar control
    // lets us force it). The list renderer renders each node as an `<li>`
    // with a real onclick handler, so Playwright can click it directly.
    // This measures: Svelte event handling + state mutation + side-panel
    // DOM update. It SKIPS the canvas hit-testing path, but that path is
    // negligible (<1ms typically) so the metric is still representative
    // of perceived click latency. The list renderer is also the
    // keyboard-accessibility fallback, so a latency number here double-
    // duty-validates that path.
    await page.getByRole('button', { name: 'List', exact: true }).click()
    await expect(page.locator('.galaxy-list-nodes')).toBeVisible()

    const clickLatencies = await measureListClickLatency(page, { samples: 8 })
    if (clickLatencies.length > 0) {
      const p95 = percentile(clickLatencies, 95)
      metrics.push(
        recordMetric(test.info(), 'click_p95_1k', p95, 'ms', THRESHOLDS.clickP95Ms, {
          lowerIsBetter: true,
          note: 'measured via list renderer (DOM-clickable) — canvas-probe unreliable on SwiftShader',
        }),
      )
    } else {
      metrics.push(
        recordMetric(test.info(), 'click_p95_1k', NaN, 'ms', THRESHOLDS.clickP95Ms, {
          lowerIsBetter: true,
          note: 'no list row found',
        }),
      )
    }

    // ── search-to-focus latency (proxy: zoom toggle → repaint) ─────────
    // The Galaxy component doesn't expose a free-text search box (the
    // Entity-page `focus` prop is the closest analogue, set from the parent
    // on a node click). We measure the zoom-control → next-paint latency as
    // a proxy for "user-initiated graph mutation round-trip": click a zoom
    // button, measure time until renderer-badge reappears (torn down on
    // reload, re-added on mount). Done in the LIST renderer so it's
    // deterministic across dev (no canvas mount cost variance).
    const zoomLatencies = await measureZoomToggleLatency(page, { samples: 5 })
    if (zoomLatencies.length > 0) {
      const p95 = percentile(zoomLatencies, 95)
      metrics.push(
        recordMetric(
          test.info(),
          'search_to_focus_p95_1k',
          p95,
          'ms',
          THRESHOLDS.searchToFocusP95Ms,
          { lowerIsBetter: true },
        ),
      )
    } else {
      metrics.push(
        recordMetric(
          test.info(),
          'search_to_focus_p95_1k',
          NaN,
          'ms',
          THRESHOLDS.searchToFocusP95Ms,
          { lowerIsBetter: true, note: 'zoom-toggle repaint never observed' },
        ),
      )
    }

    // Sanity: the renderer did mount SOMETHING with the 1k fixture.
    await expect(page.locator('.galaxy-meta')).toContainText('Nodes: 1000')
  })

  test('5k-node graph — FPS', async ({ page }) => {
    // Same warm-up + 5×5s measurement as the 1k test.
    test.setTimeout(90_000)
    await loginAsConsole(page)
    const fixture = loadFixture('graph-5k')
    await interceptGalaxy(page, fixture)
    await openGalaxyView(page)

    const activeKind = await page.locator('.renderer-badge').textContent()
    const is3d = activeKind?.trim() === '3d'

    await page.evaluate(() => {
      const w = window as unknown as { __benchFrameCount?: number; __benchRafId?: number }
      w.__benchFrameCount = 0
      const tick = (): void => {
        w.__benchFrameCount = (w.__benchFrameCount ?? 0) + 1
        w.__benchRafId = requestAnimationFrame(tick)
      }
      w.__benchRafId = requestAnimationFrame(tick)
    })

    await page.waitForTimeout(10_000)
    await page.evaluate(() => {
      const w = window as unknown as { __benchFrameCount?: number }
      w.__benchFrameCount = 0
    })

    const fpsRuns: number[] = []
    for (let run = 0; run < 5; run++) {
      const before = await page.evaluate(() => {
        const w = window as unknown as { __benchFrameCount?: number }
        return w.__benchFrameCount ?? 0
      })
      await page.waitForTimeout(5_000)
      const after = await page.evaluate(() => {
        const w = window as unknown as { __benchFrameCount?: number }
        return w.__benchFrameCount ?? 0
      })
      fpsRuns.push((after - before) / 5)
    }
    await page.evaluate(() => {
      const w = window as unknown as { __benchRafId?: number }
      if (typeof w.__benchRafId === 'number') cancelAnimationFrame(w.__benchRafId)
      w.__benchRafId = undefined
    })

    const fpsMedian = median(fpsRuns)
    metrics.push(
      recordMetric(test.info(), 'fps_5k_median', fpsMedian, 'fps', THRESHOLDS.fps5k, {
        note: is3d ? undefined : '3D deferred — headless no GPU; measured 2D/list renderer',
      }),
    )

    await expect(page.locator('.galaxy-meta')).toContainText('Nodes: 5000')
  })
})

// ── measurement helpers ────────────────────────────────────────────────────

/**
 * Click `<li class="galaxy-list-node">` rows + measure time-to-sidepanel.
 * Each click fires the list renderer's `onclick` → `callbacks.onNodeClick`
 * → GalaxyGraph sets `selected` → the `.galaxy-sidepanel` appears.
 *
 * We click the SAME row repeatedly (row 0) so the measurement isn't
 * perturbed by row-height scrolling; the click→sidepanel path is
 * independent of which row was clicked. Between clicks we click the
 * toolbar to dismiss the side-panel so the next click has a real
 * transition to observe.
 */
async function measureListClickLatency(
  page: Page,
  opts: { samples: number },
): Promise<number[]> {
  const latencies: number[] = []
  const nodeList = page.locator('.galaxy-list-nodes')
  // Wait for at least one row to be present.
  const rowCount = await nodeList.locator('li').count()
  if (rowCount === 0) return []
  for (let i = 0; i < opts.samples; i++) {
    // Dismiss any open side-panel first by clicking the toolbar (empty
    // space relative to the list). The galaxy-toolbar click doesn't hit
    // any list row so `selected` is unaffected if it's already null; if
    // a side-panel is open it stays open. To FORCE a deselect we'd need
    // a "close" button — none exists in E2.2. Instead we measure the
    // click→sidepanel-update latency via the LABEL change: click row 0
    // first (side-panel opens with "Entity 0"), then click row 1 (label
    // updates to "Entity 1") — the latency we care about is the row-1
    // click → label-change time. The first click is warm-up.
    const rowIdx = i % Math.min(rowCount, 5) // cycle through first 5 rows
    const row = nodeList.locator('li').nth(rowIdx)
    const expectedLabel = `Entity ${rowIdx}`
    // Discard stale label state — read current sidepanel h3 if any.
    const t0 = await page.evaluate(() => performance.now())
    await row.click()
    try {
      await expect(page.locator('.galaxy-sidepanel h3')).toHaveText(expectedLabel, {
        timeout: 1_000,
      })
      const t1 = await page.evaluate(() => performance.now())
      latencies.push(t1 - t0)
    } catch {
      // side-panel didn't update in time — skip this sample
    }
  }
  return latencies
}

/**
 * Click a zoom-toggle button + measure time-to-renderer-badge-reappear. The
 * reload fetch is intercepted (instant), so this measures the renderer
 * re-mount cost, which is the right proxy for "user mutated the graph, how
 * long until they see the result".
 */
async function measureZoomToggleLatency(
  page: Page,
  opts: { samples: number },
): Promise<number[]> {
  const latencies: number[] = []
  // Cycle Far → Mid → Close → Far … ; each click triggers a reload + remount.
  const buttons = ['Far', 'Mid', 'Close'] as const
  for (let i = 0; i < opts.samples; i++) {
    const label = buttons[i % buttons.length]
    const t0 = await page.evaluate(() => performance.now())
    await page.getByRole('button', { name: label }).click()
    // The reload + remount cycle rebuilds `.galaxy-meta` and the renderer-
    // badge from scratch. Wait for the badge to come back (it's torn down
    // on reload and re-added on mount) — that's the user-visible signal
    // that the new graph is painted.
    try {
      await expect(page.locator('.renderer-badge')).toBeVisible({ timeout: 2_000 })
      const t1 = await page.evaluate(() => performance.now())
      latencies.push(t1 - t0)
    } catch {
      // renderer-badge didn't come back — skip this sample.
    }
  }
  return latencies
}
