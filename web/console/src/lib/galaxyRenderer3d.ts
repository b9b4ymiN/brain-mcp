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
import {
  CanvasTexture,
  Color,
  Group,
  Mesh,
  MeshBasicMaterial,
  Sprite,
  SpriteMaterial,
  SphereGeometry,
  SRGBColorSpace,
} from 'three'
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
 * Local typed shape of a Galaxy node AS IT LIVES IN THE GRAPH. Carries the
 * original payload on `raw` so click/hover callbacks can hand back the exact
 * `GalaxyNode` the host expects. The lib MUTATES this object (adds x/y/z
 * etc.), so we extend `NodeObject` rather than composing.
 */
interface GalaxyGraphNode extends NodeObject {
  /** Original Galaxy payload node (passed through for click callbacks). */
  raw: GalaxyNode
  /** Degree count — computed once at mount from the edge list. Drives
   * `nodeVal` so hub nodes read larger than leaf nodes (DESIGN.md §5
   * "size by degree / weight"). */
  __degree?: number
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

// ── Star texture cache ─────────────────────────────────────────────────────
// A procedural radial-gradient texture (bright core, soft amber halo) that
// reads as a real star rather than a flat-shaded sphere. Built once and
// reused across every node; tinted per kind via SpriteMaterial.color.
let starTexture: CanvasTexture | null = null

function getStarTexture(): CanvasTexture {
  if (starTexture !== null) return starTexture
  const size = 64
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size
  const ctx = canvas.getContext('2d')!
  const mid = size / 2
  const gradient = ctx.createRadialGradient(mid, mid, 0, mid, mid, mid)
  gradient.addColorStop(0.0, 'rgba(255, 255, 255, 1.0)') // hot white core
  gradient.addColorStop(0.18, 'rgba(255, 244, 214, 0.95)') // warm white
  gradient.addColorStop(0.4, 'rgba(245, 179, 66, 0.5)') // amber halo
  gradient.addColorStop(0.7, 'rgba(245, 179, 66, 0.15)')
  gradient.addColorStop(1.0, 'rgba(245, 179, 66, 0.0)')
  ctx.fillStyle = gradient
  ctx.fillRect(0, 0, size, size)
  const tex = new CanvasTexture(canvas)
  tex.colorSpace = SRGBColorSpace
  starTexture = tex
  return tex
}

// ── Node color — delegated to galaxyColors.ts ──────────────────────────
// The old KIND_COLOR table (source/concept/entity keys) almost never matched
// live claim_kind values, so every node fell through to a single white.
// starColorFor() now hashes the node's domain into a 12-color stellar palette
// (see galaxyColors.ts) for multi-hued starlight. Kind overrides
// (decision → amber, error → danger) are preserved.

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
      // NODE/LINK types with a `new(element, configOptions?)` signature
      // returning `ForceGraph3DInstance<N, L>`. To specialise the instance,
      // we narrow the constructor value to the parameterised type first
      // (TS can't infer generics off a bare `new` call on a value-typed
      // symbol). The second arg (configOptions) carries rendererConfig —
      // critical for the transparent-canvas path (Home hero): without
      // {alpha:true}, the WebGLRenderer defaults to alpha:false and paints
      // an opaque black rectangle over the SpaceBackdrop even though we
      // set backgroundColor('#00000000').
      const Ctor = ForceGraph3D as unknown as new (
        element: HTMLElement,
        configOptions?: { rendererConfig?: Record<string, unknown> },
      ) => ForceGraph3DInstance<GalaxyGraphNode, GalaxyGraphLink>
      const instance = new Ctor(
        host,
        opts.transparent === true
          ? { rendererConfig: { alpha: true } }
          : undefined,
      )

      // ── Kill library default UI overlays ──────────────────────────
      // The library renders two default overlays we must suppress:
      //
      // 1. `.scene-nav-info` — "Left-click: rotate, Mouse-wheel/middle-
      //    click: zoom, Right-click: pan" at bottom center. Killed via
      //    the public `.showNavInfo(false)` setter.
      // 2. `.graph-info-msg` — "Loading..." centered in lavender (22px
      //    sans-serif). No public setter — must DOM-remove the element
      //    after construction.
      instance.showNavInfo(false)
      host.querySelector('.graph-info-msg')?.remove()

      // Compute degree per node (edge count) → drives nodeVal so hub
      // nodes visibly read larger (DESIGN.md §5: "size by weight"). The
      // degree is also surfaced to the per-node object so nodeThreeObject
      // can scale the sprite accordingly.
      const degreeMap = new Map<string, number>()
      for (const link of links) {
        degreeMap.set(link.source, (degreeMap.get(link.source) ?? 0) + 1)
        degreeMap.set(link.target, (degreeMap.get(link.target) ?? 0) + 1)
      }
      for (const n of nodes) {
        n.__degree = degreeMap.get(n.id as string) ?? 1
      }

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
          const g = node as GalaxyGraphNode
          const label = escapeHtml(g.raw.label)
          const kind = escapeHtml(g.raw.kind)
          const domainPart = g.raw.domain ? ' · ' + escapeHtml(g.raw.domain) : ''
          return `<b>${label}</b><br/>${kind}${domainPart}`
        })
        // ── Star rendering ─────────────────────────────────────────────
        // Replace the lib's default flat-shaded spheres with additive
        // sprite halos over a small bright core. Reads as actual starlight,
        // not geometry. nodeColor is per-kind (cool/neutral default, amber
        // only on "active" kinds, danger on errors — One Voice Rule holds).
        .nodeColor((node) => {
          const g = node as GalaxyGraphNode
          return starColorFor(g.raw)
        })
        .nodeRelSize(2.5)
        // nodeVal scales the node by its lib-rendered default-size; combined
        // with our per-node sprite (below) it makes hubs visibly larger.
        .nodeVal((node) => {
          const g = node as GalaxyGraphNode
          return Math.min(12, 1 + Math.sqrt(g.__degree ?? 1))
        })
        // nodeThreeObject — wrap the node in a Group containing a bright
        // core sphere + an additive halo sprite. The Group lets us scale
        // both together. Bright core uses MeshBasicMaterial (unlit = pure
        // color, reads as light not geometry).
        .nodeThreeObject((node) => {
          const g = node as GalaxyGraphNode
          const val = Math.min(12, 1 + Math.sqrt(g.__degree ?? 1))
          const coreSize = Math.max(1.5, val * 0.6)
          const haloScale = Math.max(8, val * 6)
          const colorHex = starColorFor(g.raw)

          const group = new Group()
          // Bright core — small sphere that always reads as a pinpoint.
          const core = new Mesh(
            new SphereGeometry(coreSize, 12, 12),
            new MeshBasicMaterial({ color: new Color('#ffffff') }),
          )
          group.add(core)
          // Halo sprite — the radial-gradient texture, tinted to the kind
          // color. Additive blending = light stacks where stars overlap.
          const haloMat = new SpriteMaterial({
            map: getStarTexture(),
            color: new Color(colorHex),
            transparent: true,
            opacity: 0.85,
            blending: 2, // THREE.AdditiveBlending (avoid importing enum)
            depthWrite: false,
          })
          const halo = new Sprite(haloMat)
          halo.scale.set(haloScale, haloScale, 1)
          group.add(halo)
          return group
        })
        // Cool-white links, low opacity; kind isn't really meaningful for
        // edges (most are generic), and one neutral color reads cleaner.
        .linkColor(() => 'rgba(216, 232, 245, 0.18)')
        .linkWidth(0.4)
        .linkOpacity(0.35)
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
