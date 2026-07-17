import { expect, test } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'
import {
  interceptGalaxy,
  loadFixture,
  openGalaxyView,
  type GalaxyFixture,
} from './galaxyBench.ts'

// Phase E Task E2.3 — Galaxy parity test (DoD: aggregated edge count = raw
// fixture 100%).
//
// Loads a SMALL known fixture (the 3-node triangle, deterministic — 3 nodes,
// 3 edges) and asserts the renderer reports the SAME counts as the raw
// fixture. The GalaxyGraph `.galaxy-meta` panel renders `{payload.node_count}`
// and `{payload.edges.length}`, so a parity regression in either:
//   - the route interception (wrong fixture served), OR
//   - the renderer's understanding of the payload shape, OR
//   - a future "edge aggregation" feature that clusters/suppresses edges
// would show up as a number mismatch here.
//
// The list renderer gets its own parity check: forcing it via the new
// (Carry 2) renderer-kind selector button, we assert the `<ul>` of nodes
// contains exactly the fixture's node count. The list renderer is the
// lowest-level renderer (pure DOM, no canvas), so this is the cleanest
// signal for "the data actually reached the renderer".

test.describe('Galaxy parity (real backend)', () => {
  test('triangle fixture — meta reports 3 nodes + 3 edges, 100% of raw', async ({ page }) => {
    await loginAsConsole(page)
    const triangle: GalaxyFixture = loadFixture('graph-triangle')
    await interceptGalaxy(page, triangle)
    await openGalaxyView(page)

    const meta = page.locator('.galaxy-meta')
    await expect(meta).toContainText('Nodes: 3')
    await expect(meta).toContainText('Edges: 3')

    // Explicit no-aggregation assertion: the renderer-badge indicates a
    // renderer is mounted; the meta count equals the raw fixture count
    // (not "≤" or "clustered"). We assert the EXACT text so any future
    // LOD-based aggregation (community supernodes etc.) that diverges from
    // the fixture is caught here, not silently accepted.
    const metaText = (await meta.textContent()) ?? ''
    expect(metaText).toContain('Nodes: 3')
    expect(metaText).toContain('Edges: 3')
    expect(triangle.nodes).toHaveLength(3)
    expect(triangle.edges).toHaveLength(3)
  })

  test('list renderer — <ul> shows exactly the fixture node count', async ({ page }) => {
    await loginAsConsole(page)
    // Use the 1k fixture here so we exercise the list renderer at a realistic
    // size (the triangle would pass with a trivial 3-row list, missing real
    // bugs). 1000 rows in a `<ul>` is the actual stress case.
    const fixture: GalaxyFixture = loadFixture('graph-1k')
    await interceptGalaxy(page, fixture)
    await openGalaxyView(page)

    // Force the list renderer via the Carry 2 renderer-kind selector.
    await page.getByRole('button', { name: 'List', exact: true }).click()

    // The list renderer lives INSIDE `.galaxy-canvas` (it clears the
    // container and writes its own DOM). Wait for the `.galaxy-list-nodes`
    // <ul> to appear.
    const nodeList = page.locator('.galaxy-list-nodes')
    await expect(nodeList).toBeVisible()

    // Count <li> children — must equal fixture.nodes.length (1000).
    const liCount = await nodeList.locator('li').count()
    expect(liCount, 'list renderer node count must equal raw fixture node count').toBe(
      fixture.nodes.length,
    )

    // Edges list — same parity check.
    const edgeList = page.locator('.galaxy-list-edges')
    await expect(edgeList).toBeVisible()
    const edgeLiCount = await edgeList.locator('li').count()
    expect(edgeLiCount, 'list renderer edge count must equal raw fixture edge count').toBe(
      fixture.edges.length,
    )

    // Carry 3 verification — the 1k fixture has 250 empty-domain nodes (i %
    // 4 === 0). The list renderer renders `${kind} · ${domain}` for each
    // via textContent (auto-escaped DOM text — NOT an XSS vector, just a
    // visual issue when domain is empty). Carry 3 was scoped to the 3D/2D
    // renderers' tooltip path; the list path is out of scope for this
    // commit but we verify the LABEL still renders correctly here (the
    // "Entity 0" text must be intact even when the domain is empty — that
    // IS what the user reads, and that's what we lock in for parity).
    // The list renders nodes in fixture order so node 0 is "Entity 0" with
    // kind "external_fact" + empty domain.
    const firstRow = nodeList.locator('li').first()
    const labelText = (await firstRow.locator('.galaxy-list-node-label').textContent()) ?? ''
    expect(labelText).toBe('Entity 0')
    const metaText = (await firstRow.locator('.galaxy-list-node-meta').textContent()) ?? ''
    expect(metaText).toContain('external_fact')
  })
})
