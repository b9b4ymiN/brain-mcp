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
  import { galaxy, inbox, ApiError, type GalaxyPayload, type ProposalSummary, type GalaxyNode } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { navigate } from '../lib/router'
  import { setPendingSubject } from '../lib/quickSearch'
  import GalaxyGraph from '../components/GalaxyGraph.svelte'
  import DataTable from '../components/DataTable.svelte'
  import {
    setGalaxyCounts,
    setBusy,
    setError as setStatusError,
  } from '../lib/systemStatus.svelte'

  // ── Inbox preview (pending proposals) ───────────────────────────────
  let pendingProposals = $state<ProposalSummary[]>([])
  let inboxLoaded = $state(false)

  // ── Top subjects (derived from galaxy payload) ──────────────────────
  // Nodes with the most connections (degree) are the "hubs" — interesting
  // entities to surface. We compute degree client-side from the edges.
  let topSubjects = $derived.by<{ label: string; id: string; degree: number }[]>(() => {
    if (!payload) return []
    const degree = new Map<string, number>()
    for (const edge of payload.edges) {
      degree.set(edge.source, (degree.get(edge.source) ?? 0) + 1)
      degree.set(edge.target, (degree.get(edge.target) ?? 0) + 1)
    }
    return payload.nodes
      .map((n) => ({ label: n.label, id: n.id, degree: degree.get(n.id) ?? 0 }))
      .filter((n) => n.degree > 0)
      .sort((a, b) => b.degree - a.degree)
      .slice(0, 8)
  })

  // ── Galaxy nodes as table rows (for the Home DataTable) ──────────────
  // The galaxy payload's nodes become a browsable table alongside the
  // visual graph. Each row carries label/kind/domain/id + the degree we
  // already computed for topSubjects. Clicking a row → Entity page with
  // the subject staged.
  let galaxyRows = $derived.by<Record<string, unknown>[]>(() => {
    if (!payload) return []
    const degree = new Map<string, number>()
    for (const edge of payload.edges) {
      degree.set(edge.source, (degree.get(edge.source) ?? 0) + 1)
      degree.set(edge.target, (degree.get(edge.target) ?? 0) + 1)
    }
    return payload.nodes.map((n) => ({
      id: n.id,
      label: n.label,
      kind: n.kind,
      domain: n.domain,
      degree: degree.get(n.id) ?? 0,
    }))
  })

  interface Props {
    session: SessionStore
    toasts: ToastStore
  }

  let { session, toasts }: Props = $props()

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
    void loadInbox()
  })

  async function loadInbox(): Promise<void> {
    try {
      const proposals = await inbox()
      if (!destroyed) {
        pendingProposals = proposals.slice(0, 3)
        inboxLoaded = true
      }
    } catch {
      // Inbox is an optional preview on Home — errors are silent (the
      // Inbox page surfaces them properly). Don't block Home.
    }
  }

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
        toasts.push('error', 'Session expired', 'Please sign in again.')
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
    // Navigate to Entity. The user can enter a subject there or switch
    // to the galaxy sub-view. We don't stage a subject because "Enter
    // the graph" is a global entry, not a specific-entity action.
    navigate('entity')
  }

  function openSearch(): void {
    navigate('search')
  }

  function onNodeClick(node: GalaxyNode): void {
    // Wire the galaxy node click: stage the clicked node's label as the
    // pending subject (the node label IS the claim subject), then
    // navigate to Entity. Entity's onMount consumes the staged subject
    // via consumePendingSubject() and auto-loads the claims.
    setPendingSubject(node.label)
    navigate('entity')
  }

  function openSubject(subject: string): void {
    // Top-subject tag click: stage the subject before navigating.
    setPendingSubject(subject)
    navigate('entity')
  }

  function openInbox(): void {
    navigate('inbox')
  }

  let nodeCount = $derived(payload?.node_count ?? 0)
  let edgeCount = $derived(payload?.edges.length ?? 0)
  let isEmpty = $derived(payload !== null && payload.nodes.length === 0)
</script>

<div class="home-hero">
  <!-- LEFT: galaxy canvas (transparent so the global SpaceBackdrop shows
       through). Full-height, immersive. Clicking a node → Entity. -->
  <div class="home-split-left">
    <div class="home-galaxy" role="img" aria-label="Knowledge galaxy: {nodeCount} nodes, {edgeCount} edges">
      <GalaxyGraph {session} {toasts} zoom="far" immersive height={0} onNodeClick={onNodeClick} />
    </div>

    <!-- Overlay (brand + stats + CTA) floats above the galaxy canvas. -->
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

  <!-- RIGHT: data panel (DataTable + inbox preview + top subjects).
       Scrollable, holo-panel voice. This is the "instrument readout"
       beside the galaxy viewport. -->
  <div class="home-split-right">
    {#if !sessionExpired && !error && !isEmpty && !loading && galaxyRows.length > 0}
      <div class="home-data-panel">
        <DataTable
          tableId="home-galaxy-nodes"
          rows={galaxyRows}
          rowKey={(r) => r.id as string}
          columns={[
            { key: 'label', label: 'Subject' },
            { key: 'kind', label: 'Kind' },
            { key: 'domain', label: 'Domain', hideInCompact: true },
            { key: 'degree', label: 'Links', numeric: true, defaultSort: 'desc' },
          ]}
          searchableKeys={['label', 'kind', 'domain']}
          filterable={true}
          compactable={false}
          pageable={true}
          onRowClick={(r) => openSubject(r.label as string)}
          emptyText="No nodes in the graph."
          ariaLabel="Galaxy nodes"
        />

        {#if inboxLoaded && pendingProposals.length > 0}
          <div class="related-panel">
            <p class="related-label">Review queue · {pendingProposals.length} pending</p>
            <ul class="related-list">
              {#each pendingProposals as p (p.proposal_id)}
                <li>
                  <button type="button" class="related-item" onclick={openInbox}>
                    <span class="related-subject">{p.subject}</span>
                    <span class="related-predicate">{p.predicate}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {/if}

        {#if topSubjects.length > 0}
          <div class="related-panel">
            <p class="related-label">Top subjects</p>
            <div class="subject-tags">
              {#each topSubjects as s (s.id)}
                <button type="button" class="subject-tag" onclick={() => openSubject(s.label)}>
                  {s.label}
                  <span class="subject-degree" aria-hidden="true">{s.degree}</span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  /* ── Hero shell — split view: galaxy left, data panel right ────────── */
  .home-hero {
    position: relative;
    width: 100%;
    margin-left: calc(50% - 50vw);
    margin-right: calc(50% - 50vw);
    min-height: calc(100vh - var(--shell-header-height, 6rem));
    display: flex;
    flex-direction: row;
    overflow: hidden;
  }

  /* LEFT — galaxy canvas (60%). Relative positioning for the overlay. */
  .home-split-left {
    position: relative;
    flex: 1 1 60%;
    min-width: 0;
    overflow: hidden;
  }

  /* RIGHT — data panel (40%). Scrollable, holo-panel voice. */
  .home-split-right {
    flex: 0 0 40%;
    max-width: 32rem;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    padding: var(--space-md);
    background: color-mix(in oklch, var(--surface-flat) 70%, transparent);
    backdrop-filter: blur(8px);
    border-left: var(--border-holo);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .home-data-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  /* Mobile: stack vertically (galaxy top, data bottom). */
  @media (max-width: 64rem) {
    .home-hero {
      flex-direction: column;
    }

    .home-split-left {
      flex: 0 0 50vh;
    }

    .home-split-right {
      flex: 1;
      max-width: none;
      border-left: none;
      border-top: var(--border-holo);
    }
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
    /* Opaque danger-tinted surface — the old --overlay-danger-soft was 14%
     * transparent and yielded 1.14:1 contrast over the void. */
    background: var(--surface-danger-soft);
    backdrop-filter: none;
  }

  .home-state--empty {
    border-color: var(--color-accent);
    /* Opaque accent-tinted surface. The old --overlay-accent-soft wash let
     * the void show through and the empty message all but disappeared. */
    background: var(--surface-accent-soft);
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

  /* ── Related links (Inbox preview + top subjects) ─────────────────── */
  .home-related {
    display: flex;
    gap: var(--space-md);
    flex-wrap: wrap;
    align-items: flex-start;
  }

  .related-panel {
    flex: 1;
    min-width: 16rem;
    max-width: 24rem;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-holo);
    border-radius: var(--radius-md);
    background: color-mix(in oklch, var(--surface-flat) 82%, transparent);
    backdrop-filter: blur(8px);
  }

  .related-label {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--holo-cyan);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .related-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .related-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--space-xs) var(--space-sm);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    min-height: 44px;
    justify-content: center;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .related-item:hover {
    background: var(--overlay-ink-06);
  }

  .related-subject {
    font-family: var(--font-body);
    font-size: var(--text-body);
    font-weight: var(--weight-medium);
  }

  .related-predicate {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
  }

  .subject-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }

  .subject-tag {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    /* Touch target — WCAG 2.5.5 (was 32px). */
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm);
    border: 1px solid oklch(0.78 0.13 195 / 0.25);
    border-radius: var(--radius-pill);
    background: oklch(0.78 0.13 195 / 0.06);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }

  .subject-tag:hover {
    background: oklch(0.78 0.13 195 / 0.14);
    border-color: oklch(0.78 0.13 195 / 0.5);
  }

  .subject-degree {
    font-family: var(--font-mono);
    font-size: 0.625rem;
    color: var(--text-tertiary);
    background: var(--overlay-ink-06);
    padding: 0 4px;
    border-radius: var(--radius-pill);
  }

  /* ── Mobile ────────────────────────────────────────────────────────── */
  @media (max-width: 48rem) {
    .home-hero {
      min-height: calc(100vh - var(--shell-header-height-mobile, 5rem));
    }

    /* P0-6 (2026-07-20): the overlay's min-height was calc(100vh - 5rem)
     * but its container (.home-split-left) is only flex: 0 0 50vh — so the
     * overlay was taller than the galaxy pane, causing wordmark + stats +
     * CTA to squeeze/overlap/clip. Fix: match the overlay height to the
     * galaxy pane (50vh) and shrink the galaxy pane slightly (40vh) to give
     * the data pane more room. */
    .home-split-left {
      flex: 0 0 40vh;
    }

    .home-split-right {
      flex: 1 1 auto;
      max-height: 60vh;
      overflow-y: auto;
    }

    .home-overlay {
      min-height: unset;
      height: 40vh;
      padding: var(--space-sm) var(--space-md);
      /* Drop the space-between stretch — on mobile we want everything
       * packed near the bottom of the 40vh pane, above the data section. */
      justify-content: flex-end;
      gap: var(--space-xs);
    }

    /* Hide the hero wordmark on mobile — it's redundant with the header
     * brand mark and eats precious vertical space in the 40vh galaxy pane. */
    .home-overlay > .home-head {
      display: none;
    }

    .home-wordmark {
      font-size: clamp(1.25rem, 5vw, 1.75rem);
      /* Tighter leading so the tagline fits in 1-2 lines max. */
      line-height: 1.2;
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
