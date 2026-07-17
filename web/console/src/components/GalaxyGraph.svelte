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
  }

  let {
    session,
    zoom = 'far',
    focus,
    domain,
    onNodeClick,
    height = 500,
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
    const instance = createRenderer(kind, {
      width: containerEl.clientWidth || 600,
      height,
    })
    instance.mount(containerEl, payload, {
      onNodeClick: (node) => {
        selected = node
      },
      onNodeHover: () => {
        // Hover state could drive a cursor change; we don't need it for E2.2.
      },
    })
    renderer = instance
    activeKind = kind
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

<section class="galaxy-graph">
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

  {#if sessionExpired}
    <p class="state state-error" role="alert">Session expired — sign in again.</p>
  {:else if error}
    <p class="state state-error" role="alert">
      {error}
      <button type="button" class="retry" onclick={retry}>Retry</button>
    </p>
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
  .galaxy-graph {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .galaxy-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

  .zoom-controls {
    display: inline-flex;
    border-radius: 0.375rem;
    overflow: hidden;
    border: 1px solid rgba(127, 127, 127, 0.45);
  }

  .zoom-controls button {
    padding: 0.4rem 0.875rem;
    border: none;
    border-right: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.06);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .zoom-controls button:last-child {
    border-right: none;
  }

  .zoom-controls button.active {
    background: rgba(127, 127, 127, 0.3);
    font-weight: 600;
  }

  .renderer-controls {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    border-radius: 0.375rem;
    overflow: hidden;
    border: 1px solid rgba(127, 127, 127, 0.45);
  }

  .renderer-controls button {
    padding: 0.4rem 0.75rem;
    border: none;
    border-right: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.06);
    color: inherit;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .renderer-controls button:last-of-type {
    border-right: none;
  }

  .renderer-controls button.active {
    background: rgba(127, 127, 127, 0.3);
    font-weight: 600;
  }

  .renderer-reason {
    margin: 0;
    padding: 0.4rem 0.6rem;
    font-size: 0.8rem;
    border-radius: 0.25rem;
    background: rgba(180, 140, 60, 0.12);
    border: 1px solid rgba(180, 140, 60, 0.35);
    opacity: 0.9;
  }

  .renderer-badge {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.65;
    padding: 0.2rem 0.5rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    border-radius: 0.25rem;
  }

  .galaxy-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    font-size: 0.8rem;
    opacity: 0.8;
    margin: 0;
  }

  .galaxy-meta code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  .galaxy-stage {
    display: block;
  }

  .galaxy-stage.has-sidepanel {
    display: grid;
    grid-template-columns: 1fr 18rem;
    gap: 0.75rem;
  }

  .galaxy-canvas {
    width: 100%;
    border: 1px solid rgba(127, 127, 127, 0.3);
    border-radius: 0.375rem;
    background: #000;
    overflow: hidden;
    box-sizing: border-box;
  }
  .galaxy-graph :global(.galaxy-list) {
    list-style: none;
    margin: 0;
    padding: 0.5rem;
    display: grid;
    gap: 0.25rem;
    max-height: 100%;
    overflow: auto;
    color: #eee;
  }

  .galaxy-graph :global(.galaxy-list-nodes) {
    grid-template-columns: 1fr;
  }

  .galaxy-graph :global(.galaxy-list-node) {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.25rem 0.4rem;
    border-radius: 0.25rem;
    cursor: pointer;
  }

  .galaxy-graph :global(.galaxy-list-node:hover),
  .galaxy-graph :global(.galaxy-list-node:focus-visible) {
    background: rgba(255, 255, 255, 0.1);
    outline: none;
  }

  .galaxy-graph :global(.galaxy-list-node-label) {
    font-weight: 600;
  }

  .galaxy-graph :global(.galaxy-list-node-meta) {
    opacity: 0.7;
    font-size: 0.85rem;
  }

  .galaxy-graph :global(.galaxy-list-edges) {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.8rem;
    opacity: 0.7;
  }

  .galaxy-graph :global(.galaxy-list-edge) {
    padding: 0.15rem 0.4rem;
  }

  .galaxy-graph :global(.galaxy-list-empty) {
    opacity: 0.6;
    font-style: italic;
    padding: 0.25rem 0.4rem;
    color: #eee;
  }

  .galaxy-graph :global(.galaxy-list h3) {
    color: #eee;
    margin: 0.5rem 0.5rem 0;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.7;
  }

  .galaxy-sidepanel {
    background: rgba(127, 127, 127, 0.08);
    border: 1px solid rgba(127, 127, 127, 0.35);
    border-radius: 0.375rem;
    padding: 0.75rem 1rem;
    margin-top: 0.5rem;
  }

  .galaxy-sidepanel h3 {
    margin: 0 0 0.5rem;
    font-size: 1rem;
    word-break: break-word;
  }

  .galaxy-sidepanel dl {
    margin: 0 0 0.75rem;
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.2rem 0.75rem;
    font-size: 0.85rem;
  }

  .galaxy-sidepanel dt {
    opacity: 0.65;
  }

  .galaxy-sidepanel dd {
    margin: 0;
    word-break: break-all;
  }

  .galaxy-sidepanel code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.8rem;
  }

  .galaxy-sidepanel button {
    padding: 0.4rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .state {
    margin: 0.5rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
  }

  .state-error {
    background: rgba(190, 70, 70, 0.15);
    border-color: rgba(190, 70, 70, 0.5);
  }

  .retry {
    margin-left: 0.75rem;
    padding: 0.2rem 0.6rem;
    border-radius: 0.25rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.1);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
</style>
