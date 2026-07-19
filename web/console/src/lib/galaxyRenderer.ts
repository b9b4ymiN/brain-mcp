/**
 * Galaxy renderer factory + abstraction (E2.2 Part C).
 *
 * Three concrete implementations live side-by-side:
 *   - `galaxyRenderer3d.ts`   — `3d-force-graph` (WebGL, immersive)
 *   - `galaxyRenderer2d.ts`   — `force-graph`    (canvas 2D, lighter)
 *   - `galaxyRendererList.ts` — pure DOM `<ul>`  (no GPU, screen-reader friendly)
 *
 * The kind identifiers are the SHORT forms of the Rust `GraphRendererKind`
 * enum (`#[serde(rename_all = "snake_case")]`, e.g. `force_graph_3d`) with
 * the redundant `force_graph_` prefix stripped — the TS side never needs to
 * serialise back to Rust so the cleaner local shape wins.
 *
 * Capability detection + the public types live in `galaxyRendererDetect.ts`
 * (split so the Playwright spec can import the pure detection functions
 * without dragging in the three.js graph via the renderer impls — see that
 * file's header for the full rationale). This file re-exports them for
 * app-side callers and adds the `createRenderer` factory.
 *
 * **Code-splitting (Perf)**: the concrete renderer impls (which transitively
 * pull in `3d-force-graph`, `force-graph`, and `three` — the bulk of the
 * production bundle) are loaded via dynamic `import()` so they only ship to
 * the browser when a galaxy view is actually mounted. Pages that never
 * render a galaxy (Home/Search/Inbox/Operations) load lean; only Entity
 * (and any future Home immersive view) pay the cost. `createRenderer` is
 * async for this reason; callers must `await` it.
 *
 * No `any` anywhere in this module.
 */

// Re-export the detection surface + types so app callers can import
// everything from a single module path (`../lib/galaxyRenderer`). Detection
// is light (no three.js) so it stays a static import.
export {
  detectRenderer,
  fallbackChain,
  prefersReducedMotion,
  webglAvailable,
  type GraphRenderer,
  type RendererCallbacks,
  type RendererKind,
  type RendererOpts,
} from './galaxyRendererDetect'

import type { RendererKind, RendererOpts, GraphRenderer } from './galaxyRendererDetect'

// ── factory ─────────────────────────────────────────────────────────────────

/**
 * Construct a renderer instance for `kind`. Each implementation owns its own
 * closure state; the factory just dispatches. Callers should normally go via
 * `detectRenderer(preference)` first to pick the right `kind`, then call
 * `createRenderer(kind, opts)`.
 *
 * **Async**: the renderer impls are loaded via dynamic `import()` so the
 * 3d-force-graph / force-graph / three deps only ship when a galaxy is
 * actually rendered. Callers must `await createRenderer(...)`.
 *
 * Throws if `kind` is not one of the three known values (would be a
 * programmer error — the type system should prevent it, but the runtime
 * guard is defense in depth).
 */
export async function createRenderer(
  kind: RendererKind,
  opts: RendererOpts,
): Promise<GraphRenderer> {
  switch (kind) {
    case '3d': {
      const { createRenderer3d } = await import('./galaxyRenderer3d')
      return createRenderer3d(opts)
    }
    case '2d': {
      const { createRenderer2d } = await import('./galaxyRenderer2d')
      return createRenderer2d(opts)
    }
    case 'list': {
      // List renderer takes no size opts — its DOM is layout-driven.
      const { createRendererList } = await import('./galaxyRendererList')
      return createRendererList()
    }
    default: {
      // Exhaustiveness guard — TS narrows `kind` to `never` here.
      const exhaustive: never = kind
      throw new Error(`createRenderer: unknown kind ${String(exhaustive)}`)
    }
  }
}
