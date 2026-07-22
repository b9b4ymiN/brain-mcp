/**
 * 2D Galaxy renderer (E2.2 Part D) — `force-graph` direct integration.
 *
 * This is the lighter-weight sibling of `galaxyRenderer3d.ts`: same lifecycle
 * pattern, same anti-XSS posture, but uses a 2D canvas (no three.js). It's
 * the fallback for `prefers-reduced-motion: reduce` users AND for browsers
 * where WebGL is unavailable but the 2D canvas works (essentially every
 * shipping browser).
 *
 * Same TS-types decision as the 3D renderer: import the typed default +
 * `NodeObject`/`LinkObject` helpers, isolate any casts to this file, expose
 * only the clean `GraphRenderer` interface to callers.
 *
 * Cleanup: `ForceGraph._destructor()` releases its RAF loop + d3-force
 * workers + the canvas 2D context; we additionally strip the appended
 * `<canvas>` from the DOM. The lib does NOT auto-remove the canvas.
 */

import ForceGraph, { type NodeObject } from 'force-graph'
import type { GalaxyNode, GalaxyPayload } from './api'
import { escapeHtml } from './safeText'
import { starColorFor } from './galaxyColors'
import type {
  GraphRenderer,
  RendererCallbacks,
  RendererKind,
  RendererOpts,
} from './galaxyRendererDetect'

/**
 * Local typed shape of a Galaxy node as it lives in the 2D graph. Carries
 * the original payload on `raw` so click/hover callbacks can hand back the
 * exact `GalaxyNode` the host expects.
 */
interface GalaxyGraphNode extends NodeObject {
  /** Original Galaxy payload node (passed through for click callbacks). */
  raw: GalaxyNode
}

/**
 * Local typed shape of a Galaxy edge as it lives in the 2D graph. The lib's
 * `LinkObject<N>` is parameterised on the SOURCE/TARGET node type; we
 * declare our own minimal shape and let the lib widen at the API boundary.
 */
interface GalaxyGraphLink {
  source: string
  target: string
  kind: string
}

/**
 * Build the 2D renderer. Same closure pattern as the 3D variant: instance +
 * appended canvas held locally, `destroy()` releases both.
 */
export function createRenderer2d(opts: RendererOpts): GraphRenderer {
  let graph: ForceGraph<GalaxyGraphNode, GalaxyGraphLink> | null = null
  let appendedCanvas: HTMLCanvasElement | null = null
  let container: HTMLElement | null = null

  return {
    kind: '2d' satisfies RendererKind,

    mount(host: HTMLElement, payload: GalaxyPayload, callbacks: RendererCallbacks): void {
      if (graph !== null) {
        this.destroy()
      }
      container = host
      host.replaceChildren()

      const nodes: GalaxyGraphNode[] = payload.nodes.map((n) => ({
        id: n.id,
        raw: n,
      }))
      const links: GalaxyGraphLink[] = payload.edges.map((e) => ({
        source: e.source,
        target: e.target,
        kind: e.kind,
      }))

      // `ForceGraph` ships as a class default export (per the lib's .d.ts):
      //   `declare class ForceGraph<N, L> extends ForceGraphGeneric<...> {}`
      // Construction appends a `<canvas>` to `host`. Parameterised on our
      // enriched node type so callbacks receive the right shape.
      const instance = new ForceGraph<GalaxyGraphNode, GalaxyGraphLink>(host)

      instance
        .width(opts.width)
        .height(opts.height)
        .graphData({ nodes, links })
        .nodeLabel((node) => {
          // PRIMARY XSS defense — float-tooltip routes strings through
          // d3 `.html()` (= innerHTML). Every untrusted field is escaped
          // first; only the `<b>` / `<br/>` formatting tags are literal.
          const g = node as GalaxyGraphNode
          const label = escapeHtml(g.raw.label)
          const kind = escapeHtml(g.raw.kind)
          // E2.2 Carry 3 — only join the domain with a middot if it's
          // non-empty (matches the 3D renderer; empty domain no longer
          // leaves a dangling " · " in the tooltip).
          const domainPart = g.raw.domain ? ' · ' + escapeHtml(g.raw.domain) : ''
          return `<b>${label}</b><br/>${kind}${domainPart}`
        })
        .nodeColor((node) => starColorFor((node as GalaxyGraphNode).raw))
        .linkColor(() => 'rgba(216, 232, 245, 0.18)')
        .onNodeClick((node) => {
          callbacks.onNodeClick?.((node as GalaxyGraphNode).raw)
        })
        .onNodeHover((node) => {
          callbacks.onNodeHover?.(node ? (node as GalaxyGraphNode).raw : null)
        })

      graph = instance
      const canvas = host.querySelector('canvas')
      appendedCanvas = canvas instanceof HTMLCanvasElement ? canvas : null
    },

    destroy(): void {
      if (graph !== null) {
        graph._destructor()
        graph = null
      }
      if (appendedCanvas !== null && appendedCanvas.parentNode !== null) {
        appendedCanvas.parentNode.removeChild(appendedCanvas)
      }
      appendedCanvas = null
      if (container !== null) {
        container.replaceChildren()
      }
      container = null
    },
  }
}
