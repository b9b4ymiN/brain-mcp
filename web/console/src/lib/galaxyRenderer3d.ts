/**
 * 3D Galaxy renderer (E2.2 Part D) — `3d-force-graph` direct integration.
 *
 * Phase E plan (2026-07-17 decision): use `3d-force-graph` as a DIRECT CORE
 * (NOT a React wrapper) and own the lifecycle. The library appends its own
 * `<canvas>` to the host container on construction; we hold the resulting
 * `ForceGraph3DInstance` in a closure and call its documented `_destructor()`
 * method in `destroy()` (then strip the canvas from the DOM). This is the
 * foundation for §13 Task 5.2 DoD #4 ("heap ≤10% after 20 cycles").
 *
 * TS-types decision: `3d-force-graph` ships its own .d.ts but it is loose
 * (a lot of method overloads + `object` returns). Rather than accept the
 * looseness across the codebase, we (a) import the typed `ForceGraph3D`
 * default factory and `ForceGraph3DInstance` type, (b) construct the graph
 * with the typed `new`-form `ForceGraph3D(element)` (the factory export and
 * the constructor-class share the same shape per the .d.ts), and (c) isolate
 * the inevitable `as` casts to a single typed wrapper (this file). No `any`
 * escapes this module — we declare narrow local types where the lib's are
 * too wide, and use `unknown` + narrowing where the lib is genuinely
 * untyped. The factory wrapper file (`galaxyRenderer.ts`) only sees the
 * clean `GraphRenderer` interface.
 *
 * Anti-XSS: `nodeLabel` accepts either a string or an HTMLElement. The lib's
 * underlying tooltip (`float-tooltip`) routes a STRING label through d3's
 * `.html()` setter, which is `innerHTML` (verified at
 * `node_modules/float-tooltip/dist/float-tooltip.mjs:218`:
 * `state.tooltipEl.html(state.content)`). Therefore escaping is the PRIMARY
 * defense, NOT defense-in-depth: an attacker who controls `label` / `kind` /
 * `domain` could otherwise inject `<script>` or attribute-based payloads.
 * We `escapeHtml()` every untrusted field BEFORE assembling the tooltip
 * string; the surrounding `<b>` / `<br/>` tags are intentional formatting
 * and cannot be broken out of because `escapeHtml` neutralizes `<`, `>`,
 * `"`, `'`, `&`.
 */

import ForceGraph3D, { type ForceGraph3DInstance } from '3d-force-graph'
import type { NodeObject, LinkObject } from 'three-forcegraph'
import type { GalaxyNode, GalaxyPayload } from './api'
import { escapeHtml } from './safeText'
import type {
  GraphRenderer,
  RendererCallbacks,
  RendererKind,
  RendererOpts,
} from './galaxyRendererDetect'

/**
 * Local typed shape of a Galaxy node AS IT LIVES IN THE GRAPH. Carries the
 * original payload on `raw` so click/hover callbacks can hand back the exact
 * `GalaxyNode` the host expects. The lib MUTATES this object (adds x/y/z
 * etc.), so we extend `NodeObject` rather than composing.
 */
interface GalaxyGraphNode extends NodeObject {
  /** Original Galaxy payload node (passed through for click callbacks). */
  raw: GalaxyNode
}

/**
 * Local typed shape of a Galaxy edge as it lives in the graph. The lib's
 * `LinkObject<N>` is parameterised on the SOURCE/TARGET node type, but in
 * practice the lib accepts `LinkObject<NodeObject>` at the API boundary
 * (its `graphData()` setter takes the base generic). We declare our own
 * minimal shape and cast at the one call site (see `as` below).
 */
interface GalaxyGraphLink {
  source: string
  target: string
  kind: string
}

/**
 * Build the renderer. Returned object's `mount` constructs the ForceGraph3D
 * instance; `destroy` tears it down via `_destructor()` + DOM cleanup.
 */
export function createRenderer3d(opts: RendererOpts): GraphRenderer {
  // Held in closure; `null` until `mount`, set back to `null` after `destroy`
  // so a double-`destroy()` is a no-op.
  let graph: ForceGraph3DInstance<GalaxyGraphNode, GalaxyGraphLink> | null = null
  // Track the canvas the lib appended so we can rip it out on destroy. The
  // lib's `_destructor()` cancels its RAF loop + three.js renderer but does
  // NOT remove the DOM node — that's our job.
  let appendedCanvas: HTMLCanvasElement | null = null
  // Track the container so destroy() can find the canvas without the caller
  // passing it again.
  let container: HTMLElement | null = null

  return {
    kind: '3d' satisfies RendererKind,

    mount(host: HTMLElement, payload: GalaxyPayload, callbacks: RendererCallbacks): void {
      if (graph !== null) {
        // Defensive: the caller contract is one-mount-per-instance, but if a
        // component is sloppy we shouldn't leak the previous graph. Tear it
        // down before re-mounting.
        this.destroy()
      }
      container = host
      // Clear any prior content (e.g. a list-fallback's `<ul>`) so the canvas
      // doesn't get stacked on top of stale DOM.
      host.replaceChildren()

      // Build the typed nodes/links arrays. We carry the original `GalaxyNode`
      // on each graph node so click/hover callbacks can hand back the exact
      // payload the host expects — the lib mutates these objects (adds x/y/z)
      // so a shallow clone of the raw fields would desync from the host data.
      const nodes: GalaxyGraphNode[] = payload.nodes.map((n) => ({
        id: n.id,
        raw: n,
      }))
      const links: GalaxyGraphLink[] = payload.edges.map((e) => ({
        source: e.source,
        target: e.target,
        kind: e.kind,
      }))

      // The lib's `.d.ts` exposes `ForceGraph3D` as the default export of
      // type `IForceGraph3D<N, L>` — an interface parameterised on the
      // NODE/LINK types with a `new(element)` signature returning
      // `ForceGraph3DInstance<N, L>`. To specialise the instance, we narrow
      // the constructor value to the parameterised type first (TS can't
      // infer generics off a bare `new` call on a value-typed symbol).
      const Ctor = ForceGraph3D as unknown as new (
        element: HTMLElement,
      ) => ForceGraph3DInstance<GalaxyGraphNode, GalaxyGraphLink>
      const instance = new Ctor(host)

      instance
        .width(opts.width)
        .height(opts.height)
        // transparent: clear with alpha 0 so the SpaceBackdrop shows
        // through (Home hero). Otherwise opaque #000 (Entity page).
        .backgroundColor(opts.transparent === true ? '#00000000' : '#000')
        .graphData({ nodes, links })
        .nodeLabel((node) => {
          // PRIMARY XSS defense — float-tooltip routes strings through
          // d3 `.html()` (= innerHTML). Every untrusted field is escaped
          // first; only the `<b>` / `<br/>` formatting tags are literal.
          // `node` is typed `GalaxyNode` via the constructor generic.
          const g = node as GalaxyGraphNode
          const label = escapeHtml(g.raw.label)
          const kind = escapeHtml(g.raw.kind)
          // E2.2 Carry 3 — only join the domain with a middot if it's
          // non-empty. An empty domain (which the benchmark fixtures and some
          // ego-neighborhood nodes carry) otherwise renders a trailing
          // " · " that looks like a broken separator.
          const domainPart = g.raw.domain ? ' · ' + escapeHtml(g.raw.domain) : ''
          return `<b>${label}</b><br/>${kind}${domainPart}`
        })
        .nodeAutoColorBy('kind')
        .linkAutoColorBy('kind')
        .onNodeClick((node) => {
          callbacks.onNodeClick?.((node as GalaxyGraphNode).raw)
        })
        .onNodeHover((node) => {
          callbacks.onNodeHover?.(node ? (node as GalaxyGraphNode).raw : null)
        })

      graph = instance
      // The lib appended a canvas during construction; capture it for teardown.
      const canvas = host.querySelector('canvas')
      appendedCanvas = canvas instanceof HTMLCanvasElement ? canvas : null
    },

    destroy(): void {
      if (graph !== null) {
        // Documented cleanup — releases three.js renderer, scene, RAF loop,
        // drag controls. Does NOT remove the appended canvas.
        graph._destructor()
        graph = null
      }
      if (appendedCanvas !== null && appendedCanvas.parentNode !== null) {
        appendedCanvas.parentNode.removeChild(appendedCanvas)
      }
      appendedCanvas = null
      // Belt-and-suspenders: clear anything else the lib might have left.
      if (container !== null) {
        container.replaceChildren()
      }
      container = null
    },
  }
}
