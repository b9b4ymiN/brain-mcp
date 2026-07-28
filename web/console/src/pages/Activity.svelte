<script lang="ts">
  /**
   * Activity — chronological feed of recent page changes.
   *
   * Phase 1 (2026-07-25 Console expansion): git page events only, sourced
   * from `GET /activity` (which shells out one `git log --numstat` at the
   * repo root). No client-activity merge yet — that's TODO when
   * `/ops/clients` is reachable from the activity handler.
   *
   * Layout: page chrome (kicker + h1 + tagline + Refresh) → filter pills
   * (Today / This week / This month → since=1d/7d/30d) → HoloPanel wrapping
   * a Timeline of TimelineCards → show-more pager (10 at a time).
   *
   * Race guard: mirrors Status.svelte's per-section seq pattern so a slow
   * in-flight request can't overwrite a fresher one's result.
   */
  import { onMount } from 'svelte'
  import {
    activity as apiActivity,
    type ActivityEvent,
    type ActivityKind,
    ApiError,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { formatRelative } from '../lib/format'
  import StateBox from '../components/StateBox.svelte'
  import HoloPanel from '../components/HoloPanel.svelte'
  import Timeline from '../components/Timeline.svelte'
  import TimelineCard from '../components/TimelineCard.svelte'
  import PageHead from '../components/PageHead.svelte'

  interface Props {
    session: SessionStore
    toasts: ToastStore
  }
  // `session`/`toasts` are accepted for API symmetry with sibling pages;
  // this page is read-only today and pushes nothing. Kept on the props
  // list so future enhancements (e.g. a "clear feed" action) need no
  // caller churn.
  let { session, toasts }: Props = $props()

  // Feed state — single section, single seq guard (one in-flight fetch).
  let events = $state<ActivityEvent[]>([])
  let loading = $state(false)
  let error = $state<string | null>(null)
  let seq = 0

  // Time-window filter. Server maps these to --since=Nseconds.
  let sinceFilter = $state<'1d' | '7d' | '30d'>('1d')
  // Show-more pager state. Reset to 10 on every successful refresh.
  let visibleCount = $state(10)

  function handleReadError(cause: unknown): void {
    if (cause instanceof ApiError) {
      error = cause.code === 'unauthorized' ? 'Session expired — sign in again.' : `Activity: ${cause.code}`
    } else {
      error = 'Activity: backend unreachable'
    }
  }

  async function refresh(): Promise<void> {
    const s = ++seq
    loading = true
    error = null
    try {
      const result = await apiActivity({ since: sinceFilter, limit: 50 })
      if (s !== seq) return
      events = result
      visibleCount = 10
    } catch (cause) {
      if (s !== seq) return
      handleReadError(cause)
      events = []
    } finally {
      if (s === seq) loading = false
    }
  }

  onMount(() => {
    void refresh()
  })

  let visibleEvents = $derived(events.slice(0, visibleCount))
  let hasMore = $derived(visibleCount < events.length)

  // Semantic color per kind: created=success(green), edited=accent(amber),
  // deleted=danger(red). The card tints its icon background from currentColor,
  // so the same var drives both the glyph color and the wash behind it.
  function iconColorVar(kind: ActivityKind): string {
    if (kind === 'page_created') return '--color-success'
    if (kind === 'page_deleted') return '--color-danger'
    return '--color-accent' // page_edited
  }

  // Filter pill labels — kept in a const so the markup + e2e selectors agree.
  const FILTERS: ReadonlyArray<{ value: '1d' | '7d' | '30d'; label: string }> = [
    { value: '1d', label: 'Today' },
    { value: '7d', label: 'This week' },
    { value: '30d', label: 'This month' },
  ]
</script>

<section class="activity-page">
  <PageHead
    kicker="Recent activity"
    title="Activity"
    tagline="Page edits and creation across the wiki, newest first."
  >
    {#snippet actions()}
      <button
        type="button"
        class="action-btn"
        onclick={() => void refresh()}
        disabled={loading}
        aria-busy={loading}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <path d="M21 3v5h-5" />
        </svg>
        <span>{loading ? 'Refreshing…' : 'Refresh'}</span>
      </button>
    {/snippet}
  </PageHead>

  <div class="filter-bar" role="group" aria-label="Time window">
    {#each FILTERS as opt (opt.value)}
      <button
        type="button"
        class="filter-pill"
        class:active={sinceFilter === opt.value}
        aria-pressed={sinceFilter === opt.value}
        onclick={() => {
          sinceFilter = opt.value
          void refresh()
        }}
      >
        {opt.label}
      </button>
    {/each}
  </div>

  <HoloPanel
    label="Activity feed"
    title=""
    ariaLabel="Activity feed"
    ariaLive="polite"
    ariaBusy={loading}
  >
    <StateBox
      loading={loading}
      error={error}
      empty={events.length === 0 && !loading}
      emptyText="No activity in this window."
    >
      <Timeline>
        {#each visibleEvents as ev (ev.timestamp + ev.target + ev.kind)}
          <TimelineCard
            iconColorVar={iconColorVar(ev.kind)}
            timestamp={formatRelative(ev.timestamp)}
            title={ev.target}
            meta={`+${ev.detail.added} −${ev.detail.removed} · ${ev.actor}`}
            subtitle={ev.detail.subject}
          >
            {#snippet icon()}
              {#if ev.kind === 'page_created'}
                <!-- plus-circle — new page -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="8" x2="12" y2="16" />
                  <line x1="8" y1="12" x2="16" y2="12" />
                </svg>
              {:else if ev.kind === 'page_deleted'}
                <!-- trash — deleted page -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <polyline points="3 6 5 6 21 6" />
                  <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                  <path d="M10 11v6" />
                  <path d="M14 11v6" />
                </svg>
              {:else}
                <!-- pencil — edited page -->
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                  <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
                </svg>
              {/if}
            {/snippet}
          </TimelineCard>
        {/each}
      </Timeline>

      {#if hasMore}
        <div class="pager">
          <span class="pager-info">Showing {visibleEvents.length} of {events.length}</span>
          <button type="button" class="pager-btn" onclick={() => (visibleCount += 10)}>
            Show {Math.min(10, events.length - visibleCount)} more
          </button>
        </div>
      {/if}
    </StateBox>
  </HoloPanel>
</section>

<style>
  /* Page chrome (kicker/h1/tagline/refresh button) lives in PageHead now. */
  .activity-page { display: flex; flex-direction: column; gap: var(--space-md); }

  .filter-bar { display: flex; gap: var(--space-xs); flex-wrap: wrap; }
  .filter-pill {
    padding: var(--space-xs) var(--space-sm);
    background: transparent; border: var(--border-hairline);
    border-radius: var(--radius-md); color: var(--text-secondary);
    font-family: var(--font-body); font-size: var(--text-body); cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }
  .filter-pill:hover { color: var(--text-primary); }
  .filter-pill.active {
    background: var(--surface-active-nav); color: var(--text-primary);
    border-color: var(--color-accent);
  }

  .pager {
    display: flex; justify-content: space-between; align-items: center;
    gap: var(--space-sm); padding-top: var(--space-sm);
    border-top: var(--border-hairline); margin-top: var(--space-sm);
  }
  .pager-info {
    color: var(--text-secondary);
    font-family: var(--font-mono); font-size: var(--text-mono);
  }
  .pager-btn {
    padding: var(--space-xs) var(--space-sm);
    background: transparent; border: 1px solid var(--color-accent);
    border-radius: var(--radius-md); color: var(--color-accent);
    cursor: pointer;
    font-family: var(--font-body); font-size: var(--text-body);
  }
  .pager-btn:hover { background: var(--surface-accent-soft); }
</style>
