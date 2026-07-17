import { expect, test } from '@playwright/test'
import {
  detectRenderer,
  fallbackChain,
  webglAvailable,
  prefersReducedMotion,
} from '../src/lib/galaxyRendererDetect.ts'

// E2.2 DoD #9 + #10: capability-detection unit tests.
//
// Same shape as `safetext.spec.ts`: pure-function assertions run inside the
// Playwright chromium project. The renderer modules pull in 3d-force-graph /
// force-graph / three at import time; Playwright's spec loader resolves those
// through the workspace node_modules, and `tsconfig.node.json` (which type-
// checks the e2e/ specs) passes.
//
// CAPABILITY DETECTION IS A NODE-SIDE PURE FUNCTION for the purposes of
// these tests:
//   - `webglAvailable()` / `prefersReducedMotion()` short-circuit to `false`
//     when `window`/`document`/`matchMedia` are missing — i.e. under Node,
//     which is where Playwright runs the spec file itself. So in the spec
//     process: `webglAvailable()` is `false`, `prefersReducedMotion()` is
//     `false`.
//   - `detectRenderer('3d')` therefore walks past '3d' to '2d' in Node —
//     which is EXACTLY the "WebGL unavailable" fallback path we want to
//     assert (DoD #9). The "reduced motion" path (DoD #10) is asserted by
//     spying on the matchMedia seam via dependency-injected stubs.
//   - `detectRenderer('2d')` and `detectRenderer('list')` are always
//     available regardless of capability — asserted directly.
//
// Why MOCK rather than rely on a real browser WebGL probe: headless Chromium
// on CI may have no GPU (SwiftShader is the env-specific fallback, not
// guaranteed). The plan note explicitly says to mock. We treat the Node-side
// "no window" path as the canonical mock for WebGL-unavailable; the
// reduced-motion path is exercised by patching the module's matchMedia
// reference via a tiny re-evaluation.

test.describe('Galaxy renderer fallback chain', () => {
  test('fallbackChain returns 3d -> 2d -> list', () => {
    expect(fallbackChain()).toEqual(['3d', '2d', 'list'])
  })
})

test.describe('detectRenderer preference handling (preference always honoured for 2d/list)', () => {
  test("requesting '2d' always returns 2d (canvas 2D is universal)", () => {
    expect(detectRenderer('2d')).toBe('2d')
  })

  test("requesting 'list' always returns list", () => {
    expect(detectRenderer('list')).toBe('list')
  })
})

test.describe('detectRenderer (DoD #9: WebGL unavailable degrades past 3d)', () => {
  test(
    "requesting '3d' in a no-WebGL environment (Node, no window) returns non-3d",
    () => {
      // In the Playwright spec runner (Node), `window` is undefined, so
      // `webglAvailable()` short-circuits to `false` on its very first
      // guard. `detectRenderer('3d')` must therefore walk past '3d' to the
      // unconditional '2d' slot.
      const kind = detectRenderer('3d')
      expect(kind).not.toBe('3d')
      expect(['2d', 'list']).toContain(kind)
    },
  )

  test('webglAvailable() returns false when window is undefined (Node spec context)', () => {
    // Mirrors the runtime path in a non-browser JS environment; this is
    // also the path that headless Chromium without GPU would hit if we
    // extended the probe canvas to throw.
    expect(webglAvailable()).toBe(false)
  })

  test("'2d' preference is immune to WebGL availability", () => {
    // Even with WebGL off (the Node default), the 2D renderer is unconditional.
    expect(detectRenderer('2d')).toBe('2d')
  })
})

test.describe('detectRenderer (DoD #10: prefers-reduced-motion degrades past 3d)', () => {
  // The reduced-motion seam reads `window.matchMedia`. In Node, `window` is
  // undefined so `prefersReducedMotion()` returns `false` (the safe default).
  // To assert the reduced-motion fallback path we re-evaluate the
  // capability check with a STUBBED `window.matchMedia` that reports
  // `matches: true` for the reduced-motion query. We do this by patching
  // the global surface inside a Node `vm` doesn't quite work because the
  // module already closed over `window` at import time; instead we
  // exercise the logic by re-importing the function with a stubbed global.
  //
  // Simplest reliable approach: temporarily install a fake `window` on the
  // global object with a `matchMedia` that returns `matches: true`, then
  // call `prefersReducedMotion()` — it will see the fake window and the
  // reduced-motion match.

  test('prefersReducedMotion() defaults to false when matchMedia is unavailable', () => {
    // Node context: no window, no matchMedia — must NOT claim motion is
    // reduced (default is to ALLOW motion, not block it).
    expect(prefersReducedMotion()).toBe(false)
  })

  test('with window.matchMedia reporting reduced-motion, prefersReducedMotion() is true', () => {
    // Install a minimal `window` global with a `matchMedia` that mimics
    // the user's OS accessibility preference. The module reads
    // `window.matchMedia(...)` lazily, so installing it BEFORE the call is
    // sufficient.
    const fakeWindow = {
      matchMedia(query: string): { matches: boolean } {
        return { matches: query.includes('prefers-reduced-motion') }
      },
    }
    const g = globalThis as unknown as { window?: unknown }
    const savedWindow = g.window
    g.window = fakeWindow
    try {
      expect(prefersReducedMotion()).toBe(true)
    } finally {
      // Restore — leaving `window` installed would pollute other specs.
      if (savedWindow === undefined) {
        delete g.window
      } else {
        g.window = savedWindow
      }
    }
  })

  test(
    "requesting '3d' with prefers-reduced-motion: reduce returns non-3d",
    () => {
      // Even if WebGL WERE available, reduced-motion must drop us to 2D.
      // We simulate "WebGL available + reduced-motion preferred" by
      // installing a fake window that has BOTH a WebGLRenderingContext
      // constructor AND a matchMedia reporting reduced-motion. The
      // `webglAvailable()` probe also calls `document.createElement` — we
      // stub `document` too so the probe doesn't throw.
      const fakeDocument = {
        createElement(): { getContext(): unknown } {
          return {
            getContext(): unknown {
              // Truthy WebGL context — probe would succeed.
              return { __fake: 'webgl-context' }
            },
          }
        },
      }
      const fakeWindow = {
        WebGLRenderingContext: function FakeWebGLRenderingContext() {},
        matchMedia(query: string): { matches: boolean } {
          return { matches: query.includes('prefers-reduced-motion') }
        },
      }
      const g = globalThis as unknown as {
        window?: unknown
        document?: unknown
      }
      const savedWindow = g.window
      const savedDocument = g.document
      g.window = fakeWindow
      g.document = fakeDocument
      try {
        // WebGL probe should succeed (so '3d' would otherwise be picked)…
        expect(webglAvailable()).toBe(true)
        // …but reduced-motion forces the chain to skip '3d'.
        expect(detectRenderer('3d')).not.toBe('3d')
        expect(['2d', 'list']).toContain(detectRenderer('3d'))
      } finally {
        if (savedWindow === undefined) delete g.window
        else g.window = savedWindow
        if (savedDocument === undefined) delete g.document
        else g.document = savedDocument
      }
    },
  )
})

test.describe('module surface', () => {
  test('exports are callables', () => {
    expect(typeof detectRenderer).toBe('function')
    expect(typeof fallbackChain).toBe('function')
    expect(typeof webglAvailable).toBe('function')
    expect(typeof prefersReducedMotion).toBe('function')
  })
})
