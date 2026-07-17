/**
 * Galaxy renderer capability detection (E2.2 Part C — split out from
 * `galaxyRenderer.ts`).
 *
 * WHY A SEPARATE FILE: the full `galaxyRenderer.ts` factory imports the three
 * concrete renderer implementations (`galaxyRenderer3d/2d/List`), which in
 * turn pull in `3d-force-graph` / `force-graph` / `three`. The Playwright
 * spec `e2e/galaxyRenderer.spec.ts` needs to exercise `detectRenderer` /
 * `fallbackChain` / `webglAvailable` / `prefersReducedMotion` in the Node
 * spec runner (no browser window); importing those four pure functions
 * through `galaxyRenderer.ts` would transitively drag the entire three.js
 * graph into the Node-side `tsc -p tsconfig.node.json` check, which uses
 * `moduleResolution: nodenext` and rejects the extensionless relative
 * imports the renderer files use.
 *
 * Keeping detection in this zero-local-import module lets the spec import
 * ONLY the pure functions (the type imports here use `import type` so they
 * are erased at runtime and don't reach the renderer impls either). The
 * factory `createRenderer` stays in `galaxyRenderer.ts` and re-exports the
 * detection helpers for app-side callers.
 *
 * No `any` in this module.
 */

// `.js` extension is intentional: under `tsconfig.node.json`'s
// `moduleResolution: nodenext`, relative imports need an explicit extension
// (the `.js` resolves to this `.ts` file at compile time). The app-side
// config (`tsconfig.app.json`, bundler resolution) accepts the same form.
import type { GalaxyNode, GalaxyPayload } from './api.js'

// ── kinds ───────────────────────────────────────────────────────────────────

/**
 * One of the three renderer kinds. Mirrors the Rust `GraphRendererKind`
 * variants (minus the redundant `force_graph_` prefix the server-side enum
 * carries for serde stability).
 */
export type RendererKind = '3d' | '2d' | 'list'

/**
 * Canonical fallback chain — strongest first, always-available last.
 * Mirrors `GraphRendererKind::fallback_chain` in `src/console.rs` exactly.
 */
export function fallbackChain(): RendererKind[] {
  return ['3d', '2d', 'list']
}

// ── interfaces ──────────────────────────────────────────────────────────────

/**
 * Optional host→renderer hooks. All optional so a "just render it" caller
 * (e.g. the list fallback) can pass `{}`.
 */
export interface RendererCallbacks {
  /** Fired on primary-click of a node. `node` is the original `GalaxyNode`. */
  onNodeClick?(node: GalaxyNode): void
  /**
   * Fired on hover enter/leave. `null` means "pointer left the graph / moved
   * to background". Implementations that don't have a hover concept (the DOM
   * list) may simply never call this.
   */
  onNodeHover?(node: GalaxyNode | null): void
}

/**
 * The lifecycle every renderer implementation exposes.
 *
 *   - `mount` is called ONCE per renderer instance with the host container
 *     and the current `GalaxyPayload`. The renderer MUST NOT assume the
 *     container is empty — it should clear it on mount to be safe.
 *   - `destroy` MUST release all GPU/timer/DOM resources it created.
 *     3D/2D renderers do this by calling the underlying lib's `_destructor()`
 *     AND removing the canvas they appended; the list renderer clears
 *     `container.innerHTML`. This is the foundation for E2.3's
 *     "heap ≤10% after 20 mount/destroy cycles" DoD.
 *
 * Implementations hold their underlying lib instance (or DOM root) in a
 * closure so `destroy` can reach it without leaking it onto the public
 * interface.
 */
export interface GraphRenderer {
  readonly kind: RendererKind
  mount(
    container: HTMLElement,
    graph: GalaxyPayload,
    callbacks: RendererCallbacks,
  ): void
  destroy(): void
}

/** Factory options. `width`/`height` are CSS pixels for the canvas/DOM root. */
export interface RendererOpts {
  width: number
  height: number
}

// ── capability detection ────────────────────────────────────────────────────

/**
 * Is WebGL available in THIS browser context? Returns `false` during SSR
 * (`window` undefined) or if either the `WebGLRenderingContext` constructor
 * OR a probe `canvas.getContext('webgl')` fails. Exported (and split out as
 * a named function) so the unit test can drive the "no window" path in Node
 * AND stub `window.WebGLRenderingContext` for the "WebGL off" case.
 */
export function webglAvailable(): boolean {
  if (typeof window === 'undefined') return false
  const w = window as unknown as {
    WebGLRenderingContext?: unknown
  }
  if (typeof w.WebGLRenderingContext === 'undefined') return false
  try {
    const canvas = document.createElement('canvas')
    const ctx = canvas.getContext('webgl') ?? canvas.getContext('experimental-webgl')
    return ctx !== null
  } catch {
    // `getContext` can throw on some embedded WebViews if WebGL is blocklisted.
    return false
  }
}

/**
 * Does the user prefer reduced motion? (`prefers-reduced-motion: reduce`).
 * Returns `false` during SSR or when `matchMedia` isn't available. The 3D
 * renderer's continuous animation loop is borderline motion-sickness-inducing
 * — when this is set we drop to the 2D canvas renderer which only animates
 * during the initial force-layout settle.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false
  }
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return false
  }
}

/**
 * Map a user/renderer PREFERENCE to the best AVAILABLE kind by walking the
 * canonical fallback chain. The preference is the starting point — every
 * kind below it in `fallbackChain()` is "always available".
 *
 *   - `'3d'` requested → 3D only if `webglAvailable() && !prefersReducedMotion()`,
 *     else fall through to 2D.
 *   - `'2d'` requested → always 2D (canvas 2D is universally supported).
 *   - `'list'` requested → always list.
 *
 * (If a future caller passes a kind that isn't in the chain, the function
 * returns `'list'` as the ultimate safe fallback.)
 */
export function detectRenderer(preference: RendererKind): RendererKind {
  const chain = fallbackChain()
  const startIdx = chain.indexOf(preference)
  // Start at the preference; walk down. The 3D slot is conditional, every
  // later slot is unconditional.
  for (let i = Math.max(0, startIdx); i < chain.length; i++) {
    const kind = chain[i]
    if (kind === '3d') {
      if (webglAvailable() && !prefersReducedMotion()) return '3d'
      // otherwise keep walking — try '2d' next.
      continue
    }
    return kind
  }
  return 'list'
}
