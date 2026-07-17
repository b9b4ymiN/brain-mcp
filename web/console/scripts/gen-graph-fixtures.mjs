// @ts-check
/**
 * Phase E Task E2.3 — Galaxy benchmark fixture generator.
 *
 * Produces deterministic synthetic `GalaxyPayload` JSON files of various sizes
 * for the Playwright benchmark / heap / parity specs. The Galaxy component
 * (`web/console/src/components/GalaxyGraph.svelte`) fetches its data from
 * `/api/v1/galaxy`; the benchmark suite INTERCEPTS that route via a Playwright
 * glob route match against the galaxy endpoint and serves these fixtures
 * directly so we measure the RENDERER, not the Rust backend's materializer
 * (which is bounded at 2000 nodes and would be the wrong thing to bench
 * anyway).
 *
 * Output shape mirrors `GalaxyPayload` exactly (`src/lib/api.ts`):
 *   { lod, max_nodes, node_count, nodes: GalaxyNode[], edges: GalaxyEdge[] }
 *
 * Determinism: a tiny LCG (numerical-recipes constants 1664525 / 1013904223)
 * seeded with a fixed per-size integer drives every "random" choice. Two
 * consecutive runs of this script produce byte-identical fixtures, so the
 * committed JSON is stable across machines / CI / reviewers.
 *
 * Idempotent: overwrites any prior fixtures at the same paths. Run once after
 * changing the shape; commit the regenerated files alongside the change.
 *
 * Usage:  node scripts/gen-graph-fixtures.mjs
 */

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CONSOLE_DIR = resolve(__dirname, '..')
const FIXTURES_DIR = join(CONSOLE_DIR, 'tests', 'fixtures')

// ── deterministic LCG ──────────────────────────────────────────────────────
// Numerical-Recipes constants; modulus 2^32. `Math.random()` is NOT used — it
// is seeded per-process and would produce different edges every run. The LCG
// is plenty good enough for benchmark fixtures (we only need determinism +
// spread; cryptographic strength is irrelevant).
function makeLcg(seed) {
  let state = seed >>> 0
  return function next() {
    // (state * 1664525 + 1013904223) mod 2^32 — keep it in uint32 range.
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0
    return state / 0x1_00_00_00_00 // normalize to [0, 1)
  }
}

// ── payload builders ───────────────────────────────────────────────────────

const KINDS = ['external_fact', 'inference', 'user_assertion']
// Edge kinds: `related` is the generic undirected case. Mixing in a couple of
// the semantic edge kinds (sources / supersedes / retracts) lets the renderer
// exercise its per-kind colour mapping; the parity test only counts TOTAL
// edges, so the kind distribution doesn't affect those assertions.
const EDGE_KINDS = ['related', 'related', 'related', 'sources', 'supersedes']

/**
 * Build a GalaxyPayload of `nodeCount` nodes + a deterministic edge set.
 *
 * Edges:
 *   - For each node i, add a `related` edge to i+1 (linear spine). This is
 *     what gives the renderer a connected graph to lay out (an isolated-node
 *     graph would skip the force-layout pass and skew the FPS measurement).
 *   - For each node i, with probability ~0.5 add a `related` edge to i+2
 *     (deterministic via LCG). Roughly doubles edge count vs the spine alone.
 *
 * Total edges ≈ nodeCount * 1.5 (matches the spec's "~1500 edges for 1k
 * nodes, ~7500 for 5k"). The exact count is deterministic per nodeCount.
 *
 * Empty `domain` on every 4th node — exercises the Carry 3 middot cleanup in
 * the 3D/2D tooltip path (otherwise a stray " · " would render).
 */
function buildPayload(nodeCount, seed) {
  const rand = makeLcg(seed)
  const maxNodes = nodeCount // un-capped; we want exactly the requested size
  const nodes = new Array(nodeCount)
  for (let i = 0; i < nodeCount; i++) {
    const id = `n${i}`
    const label = `Entity ${i}`
    const kind = KINDS[i % KINDS.length]
    // Every 4th node carries an EMPTY domain — the Carry 3 middot-cleanup
    // assertion in the parity spec checks the tooltip on one of these.
    const domain = i % 4 === 0 ? '' : 'bench'
    nodes[i] = { id, label, kind, domain }
  }

  const edges = []
  for (let i = 0; i < nodeCount; i++) {
    // Spine: i -> i+1 (skip the wrap-around so we don't introduce a giant
    // cycle that would make the 3D layout collapse to a torus).
    if (i + 1 < nodeCount) {
      edges.push({
        source: `n${i}`,
        target: `n${i + 1}`,
        kind: 'related',
      })
    }
    // Cross-link: i -> i+2 with ~0.5 deterministic probability.
    if (i + 2 < nodeCount && rand() < 0.5) {
      edges.push({
        source: `n${i}`,
        target: `n${i + 2}`,
        kind: EDGE_KINDS[Math.floor(rand() * EDGE_KINDS.length)],
      })
    }
  }

  return {
    // 'visible_nodes' lod is what the server returns for the close-zoom
    // benchmark path; the renderer doesn't care (it renders whatever's there).
    lod: 'visible_nodes',
    max_nodes: maxNodes,
    node_count: nodeCount,
    nodes,
    edges,
  }
}

// ── tiny 3-node triangle for the parity suite ──────────────────────────────
// The parity test asserts aggregated renderer counts == raw fixture counts
// (100%). A triangle is the smallest connected graph with a non-trivial edge
// set (3 nodes / 3 edges) — easy to eyeball and trivially countable.
function buildTriangle() {
  return {
    lod: 'visible_nodes',
    max_nodes: 0,
    node_count: 3,
    nodes: [
      { id: 't0', label: 'Alpha', kind: 'external_fact', domain: 'bench' },
      { id: 't1', label: 'Beta', kind: 'inference', domain: 'bench' },
      { id: 't2', label: 'Gamma', kind: 'user_assertion', domain: 'bench' },
    ],
    edges: [
      { source: 't0', target: 't1', kind: 'related' },
      { source: 't1', target: 't2', kind: 'related' },
      { source: 't0', target: 't2', kind: 'sources' },
    ],
  }
}

// ── writer ────────────────────────────────────────────────────────────────
const TARGETS = [
  { file: 'graph-1k.json', count: 1000, seed: 1 },
  { file: 'graph-5k.json', count: 5000, seed: 2 },
  // 20k target is available for on-demand stress generation
  // (`node scripts/gen-graph-fixtures.mjs`) but not committed — see Phase F
  // baseline re-bench. The DoD only requires 1k + 5k; committing a ~5 MB JSON
  // for a fixture no live spec loads is repo bloat. To regenerate, append
  // `{ file: 'graph-20k.json', count: 20000, seed: 3 }` here and re-run.
  // Triangle — fixture for the parity spec.
  { file: 'graph-triangle.json', count: 0, seed: 0, triangle: true },
]

mkdirSync(FIXTURES_DIR, { recursive: true })

for (const t of TARGETS) {
  const payload = t.triangle ? buildTriangle() : buildPayload(t.count, t.seed)
  const outPath = join(FIXTURES_DIR, t.file)
  // Pretty-printed 2-space JSON — keeps the committed fixtures (1k ~240KB,
  // 5k ~1.2MB) reviewable in a diff; reviewers can collapse them. The 20k
  // stress fixture (~5MB) is intentionally NOT in TARGETS — see above.
  writeFileSync(outPath, JSON.stringify(payload, null, 2) + '\n', 'utf8')
  console.log(
    `[gen-graph-fixtures] wrote ${t.file} — ${payload.nodes.length} nodes / ${payload.edges.length} edges`,
  )
}
console.log(`[gen-graph-fixtures] fixtures dir: ${FIXTURES_DIR}`)
