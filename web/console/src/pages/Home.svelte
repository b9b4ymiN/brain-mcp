<script lang="ts">
  /**
   * Home — the galaxy-immersive landing page.
   *
   * The page IS the galaxy: a full-bleed SpaceBackdrop (CSS starfield +
   * nebula) sits behind a transparent GalaxyGraph canvas (the live
   * `/api/v1/galaxy` subgraph). A minimal overlay — brand wordmark, live
   * node/edge stats, and a single "Enter the graph" CTA — floats at the
   * bottom; everything else (search, recent claims, review) lives on the
   * dedicated pages (Search / Inbox / Entity).
   *
   * State coverage (4 + loading):
   *   - loading    → SpaceBackdrop paints immediately (it's static CSS);
   *                  galaxy canvas mounts when data arrives; overlay
   *                  shows "Reading the cosmos…" until first paint.
   *   - default    → galaxy orbits; overlay shows live counts + CTA.
   *   - empty      → "This universe is empty." + capture CTA → Search.
   *   - error      → "Lost signal to the core." + retry.
   *   - permission → 401 → session.clear() → App.svelte flips to login.
   *
   * The graph itself (WebGL canvas, three.js) is rendered by the shared
   * GalaxyGraph component in `immersive` mode — no toolbar chrome, the
   * canvas fills the hero. GalaxyGraph owns the renderer lifecycle + the
   * /galaxy fetch; Home owns the hero layout + overlay.
   */
  import { onMount } from 'svelte'
  import { galaxy, ApiError, type GalaxyPayload } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import { navigate } from '../lib/router'
  import GalaxyGraph from '../components/GalaxyGraph.svelte'
  import {
    setGalaxyCounts,
    setBusy,
    setError as setStatusError,
  } from '../lib/systemStatus.svelte'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // ── Galaxy summary state (for the overlay stats) ───────────────────────
  // Home does its own light /galaxy read for the counts; GalaxyGraph does
  // a second read for the actual render. Two reads is fine — the endpoint
  // is cached server-side, and decoupling lets the overlay render counts
  // even if the WebGL canvas is still booting (or fell back to list/2D).
  let payload = $state<GalaxyPayload | null>(null)
  let loading = $state(true)
  let error = $state<string | null>(null)
  let sessionExpired = $state(false)

  let statsSeq = 0
  let destroyed = false

  onMount(() => {
    void loadStats()
  })

  async function loadStats(): Promise<void> {
    const seq = ++statsSeq
    loading = true
    error = null
    sessionExpired = false
    setBusy()
    const controller = new AbortController()
    try {
      const result = await galaxy({ zoom: 'far', signal: controller.signal })
      if (destroyed || seq !== statsSeq) return
      payload = result
      // Push counts into the global systemStatus store → drives the
      // HudFrame readout + the cockpit footer.
      setGalaxyCounts(result.node_count, result.edges.length)
    } catch (cause) {
      if (destroyed || seq !== statsSeq) return
      if (cause instanceof Error && cause.name === 'AbortError') return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        setStatusError('session expired')
        return
      }
      error =
        cause instanceof ApiError
          ? `Lost signal (${cause.code}).`
          : 'Lost signal to the core — is the backend running on :8080?'
      setStatusError(error)
    } finally {
      if (!destroyed && seq === statsSeq) loading = false
    }
  }

  function retry(): void {
    void loadStats()
  }

  function enterGraph(): void {
    // The galaxy sub-view is on Entity (GalaxyGraph's existing embed site
    // with viewMode: 'galaxy'). Navigate there with no focus node — the
    // user lands at the full graph.
    navigate('entity')
  }

  function openSearch(): void {
    navigate('search')
  }

  function onNodeClick(): void {
    // GalaxyGraph forwards node clicks; on Home the affordance is to
    // drop into the Entity detail. The node's id is already staged by
    // GalaxyGraph's onNodeClick prop — here we just navigate.
    navigate('entity')
  }

  let nodeCount = $derived(payload?.node_count ?? 0)
  let edgeCount = $derived(payload?.edges.length ?? 0)
  let isEmpty = $derived(payload !== null && payload.nodes.length === 0)
</script>

<div class="home-hero">
  <!-- The galaxy canvas — full-bleed, transparent so the global
       SpaceBackdrop (mounted in App.svelte) shows
       through where there are no nodes/edges. GalaxyGraph in immersive
       mode mounts its own renderer (3d → 2d → list fallback chain) into
       this container. role="img" + aria-label gives SR users a one-line
       summary (the overlay's counts are also live, but this anchors the
       non-text content). -->
  <div class="home-galaxy" role="img" aria-label="Knowledge galaxy: {nodeCount} nodes, {edgeCount} edges">
    <GalaxyGraph {session} zoom="far" immersive height={0} onNodeClick />
  </div>

  <!-- Overlay — minimal, floats above the canvas. Z-index scale puts it
       above the backdrop (z=0) and the canvas (z=1), below any future
       modal (z=300+). -->
  <div class="home-overlay">
    <header class="home-brand">
      <p class="home-eyebrow">Brain Console</p>
      <h1 class="home-wordmark">The shape of what it knows.</h1>
    </header>

    <div class="home-status">
      {#if sessionExpired}
        <p class="home-state home-state--error" role="alert">
          Session expired — sign in again.
        </p>
      {:else if error}
        <p class="home-state home-state--error" role="alert">
          {error}
          <button type="button" class="home-retry" onclick={retry}>Retry</button>
        </p>
      {:else if isEmpty}
        <p class="home-state home-state--empty">
          This universe is empty.
          <button type="button" class="home-retry" onclick={openSearch}>
            Capture your first claim
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <line x1="5" y1="12" x2="19" y2="12" />
              <polyline points="12 5 19 12 12 19" />
            </svg>
          </button>
        </p>
      {:else if loading || nodeCount === 0}
        <p class="home-state home-state--loading" role="status" aria-live="polite">
          <span class="home-pulse" aria-hidden="true"></span>
          Reading the cosmos…
        </p>
      {:else}
        <p class="home-meta">
          <span class="home-stat">
            <span class="home-stat-value">{nodeCount}</span>
            <span class="home-stat-label">nodes</span>
          </span>
          <span class="home-stat-sep" aria-hidden="true">·</span>
          <span class="home-stat">
            <span class="home-stat-value">{edgeCount}</span>
            <span class="home-stat-label">edges</span>
          </span>
        </p>
      {/if}
    </div>

    {#if !sessionExpired && !error && !isEmpty && !loading && nodeCount > 0}
      <div class="home-cta">
        <button type="button" class="home-enter" onclick={enterGraph}>
          Enter the graph
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="5" y1="12" x2="19" y2="12" />
            <polyline points="12 5 19 12 12 19" />
          </svg>
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  /* ── Hero shell — full viewport, contains the 4 stacked layers ─────── */
  .home-hero {
    /* Break out of App.svelte's shell-main max-width. Home IS the page.
     * Using 100% (not 100vw) + symmetric margin break-out avoids the
     * Windows scrollbar-overflow issue: 100vw includes the vertical
     * scrollbar width on Windows/Firefox, so the hero was ~17px wider
     * than the visible viewport. The 50% / -50vw pair centers regardless
     * of the parent's max-width, and the right margin mirrors the left. */
    position: relative;
    width: 100%;
    margin-left: calc(50% - 50vw);
    margin-right: calc(50% - 50vw);
    /* --shell-header-height is the height of App.svelte's <header.shell>.
     * Declared in tokens.css so changes there propagate here. */
    min-height: calc(100vh - var(--shell-header-height, 6rem));
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* ── Galaxy layer — sits above the backdrop, fills the hero ────────── */
  .home-galaxy {
    position: absolute;
    inset: 0;
    z-index: 1;
    /* GalaxyGraph's immersive canvas fills this; the renderer's own
     * transparent background lets the SpaceBackdrop show through the
     * empty regions of the graph. */
  }

  .home-galaxy :global(.galaxy-graph) {
    width: 100%;
    height: 100%;
  }

  .home-galaxy :global(.galaxy-canvas) {
    width: 100% !important;
    height: 100% !important;
  }

  /* ── Overlay — minimal chrome above the canvas ─────────────────────── */
  .home-overlay {
    position: relative;
    z-index: 2;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    align-items: stretch;
    min-height: calc(100vh - 6rem);
    padding: var(--space-xl) var(--space-lg);
    pointer-events: none; /* let galaxy drag/click pass through… */
  }

  /* …except where overlay elements actually live. */
  .home-overlay > * {
    pointer-events: auto;
  }

  /* ── Brand wordmark (top-left) ─────────────────────────────────────── */
  .home-brand {
    max-width: var(--content-measure);
  }

  .home-eyebrow {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--color-accent);
    letter-spacing: var(--text-label-tracking);
  }

  .home-wordmark {
    margin: 0;
    font-family: var(--font-display);
    font-size: clamp(2rem, 5vw, 3.5rem);
    font-weight: var(--weight-semibold);
    line-height: 1.05;
    letter-spacing: -0.03em;
    color: var(--text-primary);
    text-wrap: balance;
    /* The wordmark sits over potentially-bright galaxy regions; a subtle
     * text-shadow keeps it readable without smearing it (no glow halo —
     * that would be the "display-font with glow" SaaS cliché). */
    text-shadow: 0 2px 16px var(--surface-body);
  }

  /* ── Status row (bottom-left, above CTA) ──────────────────────────── */
  .home-status {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: var(--content-measure);
  }

  .home-state {
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    background: color-mix(in oklch, var(--surface-flat) 80%, transparent);
    backdrop-filter: blur(8px);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    align-self: flex-start;
    max-width: 100%;
  }

  .home-state--error {
    border-color: var(--color-danger);
    background: var(--overlay-danger-soft);
    backdrop-filter: none;
  }

  .home-state--empty {
    border-color: var(--color-accent);
    background: var(--overlay-accent-soft);
    backdrop-filter: none;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-sm);
  }

  .home-state--loading {
    color: var(--text-secondary);
  }

  .home-retry {
    min-height: 44px;
    padding: var(--space-xs) var(--space-md);
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
    margin-left: var(--space-sm);
  }

  .home-state--empty .home-retry {
    margin-left: 0;
    border-color: var(--color-accent);
    background: var(--color-accent);
    color: var(--text-on-accent);
  }

  .home-retry:hover {
    background: var(--overlay-ink-06);
  }

  .home-state--empty .home-retry:hover {
    background: var(--color-accent-deep);
  }

  /* Loading pulse — the single amber dot, breathes (kills the centered
   * spinner cliché; matches "stars appearing"). */
  .home-pulse {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent);
    animation: home-pulse 1.4s var(--ease-breathe) infinite;
  }

  @keyframes home-pulse {
    0%, 100% { opacity: 0.4; transform: scale(0.8); }
    50%      { opacity: 1;   transform: scale(1.1); }
  }

  /* ── Live stats (default state) — inline, sentence case ───────────── */
  .home-meta {
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    background: color-mix(in oklch, var(--surface-flat) 80%, transparent);
    backdrop-filter: blur(8px);
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-sm);
    align-self: flex-start;
  }

  .home-stat {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-xs);
  }

  .home-stat-value {
    font-family: var(--font-mono);
    font-size: 1.25rem;
    font-weight: var(--weight-medium);
    color: var(--text-primary);
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  .home-stat-label {
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-secondary);
  }

  .home-stat-sep {
    color: var(--text-tertiary);
  }

  /* ── CTA (bottom-right on desktop, bottom-full on mobile) ─────────── */
  .home-cta {
    display: flex;
    justify-content: flex-end;
  }

  .home-enter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 48px;
    padding: var(--space-sm) var(--space-lg);
    border: none;
    border-radius: var(--radius-pill);
    background: var(--color-accent);
    color: var(--text-on-accent);
    font-family: var(--font-body);
    font-size: var(--text-body);
    font-weight: var(--weight-semibold);
    cursor: pointer;
    box-shadow: var(--shadow-lift);
    transition: background var(--duration-fast) var(--ease-out-quart),
      transform var(--duration-fast) var(--ease-out-quart);
  }

  .home-enter:hover {
    background: var(--color-accent-deep);
    transform: translateY(-1px);
  }

  .home-enter:active {
    transform: translateY(0);
  }

  /* ── Mobile ────────────────────────────────────────────────────────── */
  @media (max-width: 48rem) {
    .home-hero {
      min-height: calc(100vh - var(--shell-header-height-mobile, 5rem));
    }

    .home-overlay {
      min-height: calc(100vh - var(--shell-header-height-mobile, 5rem));
      padding: var(--space-md);
    }

    .home-wordmark {
      font-size: clamp(1.5rem, 7vw, 2.5rem);
    }

    .home-cta {
      justify-content: stretch;
    }

    .home-enter {
      width: 100%;
      justify-content: center;
    }
  }

  /* ── Reduced motion: kill pulse, breathe, transform ───────────────── */
  @media (prefers-reduced-motion: reduce) {
    .home-pulse {
      animation: none;
      opacity: 0.8;
    }

    .home-enter:hover {
      transform: none;
    }
  }
</style>
