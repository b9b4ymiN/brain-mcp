<script lang="ts" module>
  /**
   * GalaxyGraph — reusable galaxy visualisation component (E2.2 Part E).
   *
   * Renders a 3D (with graceful fallback to 2D / DOM list) force graph of the
   * materialised subgraph from `GET /api/v1/galaxy`. Self-contained: owns
   * its data fetch, renderer lifecycle, zoom controls, and node side-panel.
   *
   * Embed sites:
   *   - Entity.svelte — `viewMode: 'galaxy'` toggle, `focus=entity_id`,
   *     `zoom='close'`. The "Open as entity" side-panel button navigates the
   *     parent back to the table view of the clicked node.
   *
   * The 5-page `ConsolePage` enum is intentionally NOT extended (Rust
   * contract is locked at 5); Galaxy is a sub-view, never a top-level page.
   *
   * Anti-XSS: side-panel node fields are bound via Svelte text interpolation
   * (auto-escaped). The raw-HTML Svelte mustache is NOT used anywhere in
   * this file (verified by the DoD grep). Tooltip-side escaping lives in
   * the renderer implementations.
   *
   * WebGL cleanup: `onDestroy` calls `renderer.destroy()` which calls the
   * underlying lib's `_destructor()` AND removes the appended canvas. This
   * is the foundation for E2.3's "heap ≤10% after 20 mount/destroy cycles"
   * DoD (#4 of §13 Task 5.2).
   *
   * DEFERRED (Part G): in-graph capture/propose/edit ("Add claim" / "Edit"
   * from the side panel) is NOT implemented in E2.2 — it requires backend
   * capture/propose endpoints that don't exist in E0 (which only has read +
   * review). The "Open as entity" button is the in-graph entry point to the
   * EXISTING capture/propose/inbox flow; "audit event" visibility comes from
   * refreshing the inbox after any change. Revisit in a later phase when
   * `POST /capture` etc. land.
   */
</script>

<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import {
    galaxy as fetchGalaxy,
    ApiError,
    type GalaxyPayload,
    type GalaxyNode,
    type ZoomLevel,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import {
    createRenderer,
    detectRenderer,
    prefersReducedMotion,
    webglAvailable,
    type GraphRenderer,
    type RendererKind,
  } from '../lib/galaxyRenderer'
  import StateBox from './StateBox.svelte'

  interface Props {
    /** Session store — for `clear()` on 401 + flash banners. */
    session: SessionStore
    /** Zoom level. When `focus` is set, `'close'` is the natural default. */
    zoom?: ZoomLevel
    /** Optional entity-id to pin an ego neighborhood around. */
    focus?: string
    /** Optional domain filter. */
    domain?: string
    /**
     * Fired when the user activates a node (click / Enter on list rows).
     * Parent typically navigates to the Entity page for that id.
     */
    onNodeClick?: (node: GalaxyNode) => void
    /** Container height in CSS pixels. */
    height?: number
    /**
     * Immersive mode — used when the graph is the page's full-bleed hero
     * (the Home landing). Hides the toolbar chrome (zoom/renderer controls,
     * the LOD meta line, the StateBox wrapper), lets the canvas fill its
     * container at 100% height, and renders error/empty states as a glass
     * overlay instead of an inline banner. The parent owns the hero layout
     * (overlay panels, brand, CTA); this component just draws the graph.
     */
    immersive?: boolean
  }

  let {
    session,
    zoom = 'far',
    focus,
    domain,
    onNodeClick,
    height = 500,
    immersive = false,
  }: Props = $props()

  // ── state ──────────────────────────────────────────────────────────────
  let loading = $state(false)
  let error = $state<string | null>(null)
  let payload = $state<GalaxyPayload | null>(null)
  let sessionExpired = $state(false)
  // `activeZoom` is the user-controllable zoom. Default to `'far'`; the
  // initial prop value is applied in `onMount` (NOT in the `$state(...)`
  // initializer — referencing `zoom` there triggers Svelte's
  // `state_referenced_locally` warning because the rune would only capture
  // the initial value). onMount seeding gives the same one-time effect.
  let activeZoom = $state<ZoomLevel>('far')
  let selected = $state<GalaxyNode | null>(null)
  // The kind that was ACTUALLY mounted (after capability detection). Shown
  // in the UI so the user understands why they're seeing a 2D/list view.
  let activeKind = $state<RendererKind | null>(null)
  // E2.2 Carry 2 — tracks whether the user has explicitly chosen a renderer
  // kind via the toolbar control. When `false` (the default — auto-detected),
  // the inline "Showing 2D — WebGL unavailable" reasoner line appears
  // whenever `activeKind !== '3d'` so the user understands WHY 3D isn't on.
  // When `true`, the user picked this kind themselves and the reasoner stays
  // silent (their own choice, not a fallback).
  let manualPreference = $state(false)
  // The user's chosen kind (when manualPreference=true). Seeded to '3d' so
  // the very first paint attempts 3D; the toolbar can flip it to '2d'/'list'.
  let requestedKind = $state<RendererKind>('3d')

  // Renderer instance held OUTSIDE reactive state — we don't want Svelte
  // reactivity poking at the lib's internals. Mutable `let` in component
  // scope; assigned in effects, read in `onDestroy`.
  let renderer: GraphRenderer | null = null
  // Container div bound via Svelte; the renderer mounts into this.
  let containerEl: HTMLDivElement | null = $state(null)

  // Monotonic request-id guard — same pattern as Entity.svelte: if the user
  // changes zoom while a fetch is in flight, the stale response is discarded.
  let fetchSeq = 0
  // ── Unmount guard (E2.2 Carry 1) ───────────────────────────────────────
  // If the component unmounts while `fetchGalaxy()` is mid-flight, the
  // resolved promise would otherwise mutate a destroyed component's state
  // (set `payload`, mount a renderer into a torn-down container, etc.). We
  // pair a `destroyed` flag (set in `onDestroy`) with an `AbortController`
  // whose `signal` is forwarded into `fetch` via `api.galaxy({signal})`. The
  // aborted fetch rejects with an `AbortError`; we swallow it silently. The
  // `destroyed` flag is the belt-and-suspenders backstop in case the abort
  // races (e.g. fetch already completed but the microtask hasn't run yet).
  let destroyed = false
  let abortController: AbortController | null = null

  // ── data fetch ─────────────────────────────────────────────────────────
  async function reload(): Promise<void> {
    const seq = ++fetchSeq
    loading = true
    error = null
    sessionExpired = false
    // Tear down any existing renderer BEFORE we re-fetch — the new payload
    // may have a different size, and a stale graph shouldn't outlive its
    // fetch.
    teardownRenderer()
    payload = null
    selected = null
    // Abort any PREVIOUS in-flight fetch (the seq guard alone would discard
    // its result, but aborting also frees the network resources). Always
    // create a fresh controller for THIS fetch.
    abortController?.abort()
    const controller = new AbortController()
    abortController = controller
    try {
      const result = await fetchGalaxy({
        zoom: activeZoom,
        focus,
        domain,
        signal: controller.signal,
      })
      // AbortError would have thrown before reaching here; still defend
      // against both the destroy path and the supersede path.
      if (destroyed || seq !== fetchSeq) return
      payload = result
    } catch (cause) {
      if (destroyed || seq !== fetchSeq) return
      // Expected on unmount — abort fired by onDestroy or by a newer reload.
      // Swallow silently (this is the Carry 1 contract).
      if (cause instanceof Error && cause.name === 'AbortError') return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      error =
        cause instanceof ApiError
          ? `Failed to load galaxy (${cause.code}).`
          : 'Failed to load galaxy — is the backend running on :8080?'
    } finally {
      if (!destroyed && seq === fetchSeq) loading = false
    }
  }

  // ── renderer lifecycle ─────────────────────────────────────────────────
  function teardownRenderer(): void {
    if (renderer !== null) {
      renderer.destroy()
      renderer = null
    }
    activeKind = null
  }

  function mountRenderer(): void {
    if (payload === null) return
    if (containerEl === null) return
    // Capability detection walks the fallback chain for us — '3d' requested
    // becomes '2d' or 'list' if WebGL is off / reduced-motion is preferred.
    // When the user has explicitly chosen a kind (manualPreference=true),
    // we honor it verbatim (the badge reasoner is suppressed in that case
    // because the user picked it on purpose). When auto, we still start
    // from '3d' and let detection walk down.
    const requested = manualPreference ? requestedKind : '3d'
    const kind = detectRenderer(requested)
    // createRenderer is async — it dynamically imports the heavy renderer
    // deps (3d-force-graph / three) so they only ship to the browser when
    // a galaxy is actually rendered. We fire it without awaiting; the
    // promise resolves into `renderer` and the next $effect cycle picks up
    // the populated instance. We guard against mounting into a torn-down
    // container (destroyed flag) or a superseded request (seq/fetchSeq).
    void createRenderer(kind, {
      width: containerEl.clientWidth || 600,
      // Immersive (Home hero): transparent clear so the SpaceBackdrop
      // shows through. Otherwise opaque #000 (Entity page).
      height: immersive ? containerEl.clientHeight || 600 : height,
      transparent: immersive,
    }).then((instance) => {
      if (destroyed) {
        // Component unmounted while we were importing. Tear down the
        // instance we just created so it doesn't leak.
        instance.destroy()
        return
      }
      if (containerEl === null) {
        instance.destroy()
        return
      }
      instance.mount(containerEl, payload!, {
        onNodeClick: (node) => {
          selected = node
        },
        onNodeHover: () => {
          // Hover state could drive a cursor change; we don't need it for E2.2.
        },
      })
      renderer = instance
      activeKind = kind
    })
  }

  // After every successful fetch (or prop change that requires a re-mount),
  // mount the renderer once the container is in the DOM. `$effect` runs after
  // the DOM update, so `containerEl` is populated by then.
  $effect(() => {
    // Re-run when these change.
    const currentPayload = payload
    const currentContainer = containerEl
    if (currentPayload !== null && currentContainer !== null && renderer === null) {
      mountRenderer()
    }
  })

  // Re-fetch when zoom / focus / domain change.
  $effect(() => {
    // Read the reactive props so Svelte tracks them.
    void activeZoom
    void focus
    void domain
    void reload()
  })

  // Resize handler — if the container width changes (window resize, sidebar
  // collapse), re-mount so the canvas matches. Cheap because we're already
  // discarding the prior renderer on every reload; for in-place resizes we
  // just remount with the current payload.
  let resizeObserver: ResizeObserver | null = null
  onMount(() => {
    // Seed the user-controllable zoom from the prop NOW (one-time). Doing
    // this in onMount rather than in the `$state(...)` initializer avoids
    // the `state_referenced_locally` warning and gives us a clean one-shot
    // seed (subsequent prop changes do NOT override the user's manual zoom
    // selection, which is the desired UX).
    activeZoom = zoom
    void reload()
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => {
        // Only remount if we already have a payload — avoid racing with the
        // initial fetch.
        if (payload !== null && containerEl !== null) {
          teardownRenderer()
          mountRenderer()
        }
      })
      if (containerEl !== null) resizeObserver.observe(containerEl)
    }
  })

  onDestroy(() => {
    // §13 Task 5.2 DoD #4 — release WebGL resources on teardown.
    // E2.2 Carry 1: also signal any in-flight fetch to abort so its resolved
    // promise can't mutate a destroyed component's state. The `destroyed`
    // flag is the secondary backstop — checked after every `await` in reload.
    destroyed = true
    abortController?.abort()
    abortController = null
    teardownRenderer()
    if (resizeObserver !== null) {
      resizeObserver.disconnect()
      resizeObserver = null
    }
  })

  // ── UI handlers ────────────────────────────────────────────────────────
  function setZoom(next: ZoomLevel): void {
    if (next === activeZoom) return
    activeZoom = next
    // The $effect above picks this up and refetches + remounts.
  }

  /**
   * E2.2 Carry 2 — user explicitly chose a renderer kind via the toolbar
   * control. Sets the manual-preference flag (suppresses the "Showing 2D —
   * WebGL unavailable" reasoner on subsequent renders because the user
   * picked it themselves), stores the choice, and re-mounts the renderer
   * against the current payload (no refetch needed — same graph, different
   * drawing engine).
   */
  function setRequestedKind(kind: RendererKind): void {
    if (manualPreference && requestedKind === kind) return
    manualPreference = true
    requestedKind = kind
    if (payload !== null && containerEl !== null) {
      teardownRenderer()
      mountRenderer()
    }
  }

  function openSelected(): void {
    if (selected !== null) {
      onNodeClick?.(selected)
    }
  }

  function retry(): void {
    void reload()
  }

  // Convenience derived values.
  let isEmpty = $derived(payload !== null && payload.nodes.length === 0)
</script>

<section class="galaxy-graph" class:immersive>
  {#if !immersive}
    <div class="galaxy-toolbar">
      <div class="zoom-controls" role="group" aria-label="Galaxy zoom level">
        <button
          type="button"
          class:active={activeZoom === 'far'}
          onclick={() => setZoom('far')}
        >
          Far
        </button>
        <button
          type="button"
          class:active={activeZoom === 'mid'}
          onclick={() => setZoom('mid')}
        >
          Mid
        </button>
        <button
          type="button"
          class:active={activeZoom === 'close'}
          onclick={() => setZoom('close')}
        >
          Close
        </button>
      </div>
      <div class="renderer-controls" role="group" aria-label="Galaxy renderer">
        <button
          type="button"
          class:active={requestedKind === '3d'}
          aria-pressed={requestedKind === '3d'}
          onclick={() => setRequestedKind('3d')}
          title="3D force graph (needs WebGL + motion OK)"
        >
          3D
        </button>
        <button
          type="button"
          class:active={requestedKind === '2d'}
          aria-pressed={requestedKind === '2d'}
          onclick={() => setRequestedKind('2d')}
          title="2D canvas force graph"
        >
          2D
        </button>
        <button
          type="button"
          class:active={requestedKind === 'list'}
          aria-pressed={requestedKind === 'list'}
          onclick={() => setRequestedKind('list')}
          title="Accessible DOM list (no GPU)"
        >
          List
        </button>
        {#if activeKind !== null}
          <span class="renderer-badge" title="Active renderer (3D needs WebGL + motion OK)">
            {activeKind}
          </span>
        {/if}
      </div>
    </div>

    {#if activeKind !== null && activeKind !== '3d' && !manualPreference}
      {#if !webglAvailable()}
        <p class="renderer-reason" role="note">
          Showing {activeKind.toUpperCase()} — WebGL unavailable.
        </p>
      {:else if prefersReducedMotion()}
        <p class="renderer-reason" role="note">
          Showing {activeKind.toUpperCase()} — reduced motion is on.
        </p>
      {/if}
    {/if}
  {/if}

  {#if sessionExpired}
    <p class="state state-error" role="alert">Session expired — sign in again.</p>
  {:else if error}
    <p class="state state-error" role="alert">
      {error}
      <button type="button" class="retry" onclick={retry}>Retry</button>
    </p>
  {:else if immersive}
    {#snippet children()}
      <div
        class="galaxy-canvas"
        style="height: {height}px;"
        bind:this={containerEl}
      ></div>
    {/snippet}
    {@render children()}
  {:else}
    <StateBox
      loading={loading}
      error={null}
      empty={isEmpty}
      emptyText="No nodes at this zoom. Try a wider view."
    >
      {#snippet children()}
        {#if payload !== null}
          <p class="galaxy-meta">
            <span>LOD: <code>{payload.lod}</code></span>
            <span>Nodes: {payload.node_count}{#if payload.max_nodes > 0} / max {payload.max_nodes}{/if}</span>
            <span>Edges: {payload.edges.length}</span>
          </p>
        {/if}
        <div
          class="galaxy-stage"
          class:has-sidepanel={selected !== null}
        >
          <div
            class="galaxy-canvas"
            style="height: {height}px;"
            bind:this={containerEl}
          ></div>
          {#if selected !== null}
            <aside class="galaxy-sidepanel" aria-label="Selected node">
              <h3>{selected.label}</h3>
              <dl>
                <div><dt>Kind</dt><dd>{selected.kind}</dd></div>
                <div><dt>Domain</dt><dd>{selected.domain}</dd></div>
                <div><dt>ID</dt><dd><code>{selected.id}</code></dd></div>
              </dl>
              <button type="button" onclick={openSelected}>Open as entity</button>
            </aside>
          {/if}
        </div>
      {/snippet}
    </StateBox>
  {/if}
</section>

<style>
  /* ── GalaxyGraph — token-driven, with immersive variant ──────────────
   * Token migration: every prior rgba(127,127,127,X) + #eee + #000 now
   * reads from lib/tokens.css. The .galaxy-list h3 was a tracked-uppercase
   * eyebrow (the saturated AI scaffold tell) — now sentence-case Inter.
   * Immersive mode (Home hero): no toolbar, canvas fills container, no
   * border on the canvas (it sits on the SpaceBackdrop already). */

  .galaxy-graph {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    color: var(--text-primary);
    font-family: var(--font-body);
  }

  /* Immersive: collapse the gap, no toolbar chrome above. */
  .galaxy-graph.immersive {
    gap: 0;
    height: 100%;
  }

  .galaxy-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .zoom-controls,
  .renderer-controls {
    display: inline-flex;
    align-items: center;
    border-radius: var(--radius-md);
    overflow: hidden;
    border: var(--border-hairline);
  }

  .zoom-controls button,
  .renderer-controls button {
    min-height: 44px;
    padding: var(--space-xs) var(--space-md);
    border: none;
    color: var(--text-secondary);
    background: transparent;
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .zoom-controls button {
    border-right: 1px solid var(--color-hairline);
  }

  .renderer-controls button {
    border-right: 1px solid var(--color-hairline);
    font-size: var(--text-mono);
    font-family: var(--font-mono);
  }

  .zoom-controls button:last-child,
  .renderer-controls button:last-of-type {
    border-right: none;
  }

  .zoom-controls button:hover,
  .renderer-controls button:hover {
    background: var(--overlay-ink-06);
    color: var(--text-primary);
  }

  .zoom-controls button.active,
  .renderer-controls button.active {
    background: var(--overlay-ink-15);
    color: var(--text-primary);
    font-weight: var(--weight-semibold);
  }

  .renderer-reason {
    margin: 0;
    padding: var(--space-xs) var(--space-sm);
    font-family: var(--font-body);
    font-size: var(--text-mono);
    color: var(--text-primary);
    border-radius: var(--radius-sm);
    background: var(--overlay-accent-soft);
    border: 1px solid var(--color-accent);
  }

  .renderer-badge {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-hairline);
    border-radius: var(--radius-sm);
    margin-left: var(--space-xs);
  }

  .galaxy-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
    margin: 0;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-mono);
  }

  .galaxy-meta code {
    font-family: var(--font-mono);
    color: var(--text-primary);
  }

  .galaxy-stage {
    display: block;
  }

  .galaxy-stage.has-sidepanel {
    display: grid;
    grid-template-columns: 1fr 18rem;
    gap: var(--space-md);
  }

  @media (max-width: 48rem) {
    .galaxy-stage.has-sidepanel {
      grid-template-columns: 1fr;
    }
  }

  .galaxy-canvas {
    width: 100%;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    /* The canvas itself is the void — the SpaceBackdrop (Home) sits
     * behind it. On Entity (no backdrop) the void is the canvas bg. */
    background: var(--surface-body);
    overflow: hidden;
    box-sizing: border-box;
  }

  /* Immersive: no border, no radius — the canvas IS the page. */
  .galaxy-graph.immersive .galaxy-canvas {
    border: none;
    border-radius: 0;
    background: transparent;
  }

  /* ── List renderer (DOM fallback, no GPU) ──────────────────────────── */
  .galaxy-graph :global(.galaxy-list) {
    list-style: none;
    margin: 0;
    padding: var(--space-sm);
    display: grid;
    gap: var(--space-xs);
    max-height: 100%;
    overflow: auto;
    color: var(--text-primary);
  }

  .galaxy-graph :global(.galaxy-list-nodes) {
    grid-template-columns: 1fr;
  }

  .galaxy-graph :global(.galaxy-list-node) {
    display: flex;
    justify-content: space-between;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--text-primary);
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .galaxy-graph :global(.galaxy-list-node:hover),
  .galaxy-graph :global(.galaxy-list-node:focus-visible) {
    background: var(--overlay-ink-10);
    outline: none;
    box-shadow: inset 0 0 0 1px var(--color-accent);
  }

  .galaxy-graph :global(.galaxy-list-node-label) {
    font-family: var(--font-body);
    font-weight: var(--weight-semibold);
  }

  .galaxy-graph :global(.galaxy-list-node-meta) {
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
  }

  .galaxy-graph :global(.galaxy-list-edges) {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
  }

  .galaxy-graph :global(.galaxy-list-edge) {
    padding: var(--space-xs) var(--space-sm);
  }

  .galaxy-graph :global(.galaxy-list-empty) {
    color: var(--text-tertiary);
    font-style: italic;
    padding: var(--space-xs) var(--space-sm);
  }

  /* List heading — was tracked-uppercase eyebrow; now sentence-case. */
  .galaxy-graph :global(.galaxy-list h3) {
    color: var(--text-secondary);
    margin: var(--space-sm) var(--space-sm) 0;
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: 0;
    text-transform: none;
  }

  /* ── Side panel (selected node detail) ────────────────────────────── */
  .galaxy-sidepanel {
    background: var(--surface-flat);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    padding: var(--space-md);
    margin-top: var(--space-xs);
  }

  .galaxy-sidepanel h3 {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-display);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    word-break: break-word;
    letter-spacing: 0;
  }

  .galaxy-sidepanel dl {
    margin: 0 0 var(--space-sm);
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-xs) var(--space-md);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  .galaxy-sidepanel dt {
    color: var(--text-secondary);
    font-size: var(--text-label);
  }

  .galaxy-sidepanel dd {
    margin: 0;
    color: var(--text-primary);
    word-break: break-all;
  }

  .galaxy-sidepanel code {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
  }

  .galaxy-sidepanel button {
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .galaxy-sidepanel button:hover {
    background: var(--overlay-ink-06);
  }

  /* ── State banners (error/session-expired only — immersive mode) ──── */
  .state {
    margin: var(--space-xs) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  .state-error {
    background: var(--overlay-danger-soft);
    border-color: var(--color-danger);
  }

  .retry {
    margin-left: var(--space-md);
    min-height: 44px;
    padding: var(--space-xs) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    cursor: pointer;
  }

  .retry:hover {
    background: var(--overlay-ink-06);
  }
</style>
