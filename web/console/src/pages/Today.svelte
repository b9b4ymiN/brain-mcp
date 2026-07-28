<script lang="ts">
  /**
   * Today — wake-up dashboard. Answers: what needs review, is system
   * healthy, what changed today. No galaxy (galaxy lives on Home).
   *
   * Phase 1 (2026-07-25 Console expansion, IA-B'): the landing pad the
   * primary nav lands on by default after Home. Parallel-fetches four
   * read-only endpoints on mount (race-guarded by a single seq) and
   * projects them into: an alerts strip, a 3-up stat row, a 5-item
   * activity preview (reusing Timeline/TimelineCard), and a health
   * rollup — each with a deep-link "View all" into its dedicated page.
   *
   * Inbox shape note: `inbox()` returns a bare `InboxProposal[]` array
   * (NOT `{proposals: [...]}`) — see lib/api.ts. So `pendingCount =
   * i.length` directly. No wrapper field to fall back on.
   */
  import { onMount } from 'svelte'
  import {
    status as apiStatus, activity as apiActivity,
    opsBackupHealth as apiOpsBackupHealth,
    inbox as apiInbox,
    type WikiStats, type ActivityEvent, type BackupHealth,
    type ActivityKind,
    ApiError,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { navigate } from '../lib/router'
  import { formatRelative } from '../lib/format'
  import StateBox from '../components/StateBox.svelte'
  import HoloPanel from '../components/HoloPanel.svelte'
  import Timeline from '../components/Timeline.svelte'
  import TimelineCard from '../components/TimelineCard.svelte'
  import PageHead from '../components/PageHead.svelte'

  interface Props { session: SessionStore; toasts: ToastStore }
  // `session`/`toasts` accepted for API symmetry with sibling pages; this
  // page is read-only today and pushes nothing. Kept so future
  // enhancements (e.g. a "snooze alert" action) need no caller churn.
  let { session, toasts }: Props = $props()

  let stats = $state<WikiStats | null>(null)
  let events = $state<ActivityEvent[]>([])
  let backup = $state<BackupHealth | null>(null)
  let pendingCount = $state(0)
  let loading = $state(false)
  let error = $state<string | null>(null)
  let seq = 0

  async function refreshAll(): Promise<void> {
    const s = ++seq
    loading = true
    error = null
    try {
      // Parallel fetch — four independent reads. Inbox is a bare array.
      const [s_, a, b, i] = await Promise.all([
        apiStatus(),
        apiActivity({ since: '1d', limit: 5 }),
        apiOpsBackupHealth(),
        apiInbox(),
      ])
      if (s !== seq) return
      stats = s_
      events = a
      backup = b
      pendingCount = i.length
    } catch (cause) {
      if (s !== seq) return
      error = cause instanceof ApiError
        ? (cause.code === 'unauthorized' ? 'Session expired — sign in again.' : `Today: ${cause.code}`)
        : 'Today: backend unreachable'
    } finally {
      if (s === seq) loading = false
    }
  }
  onMount(() => { void refreshAll() })

  // Alerts computation (phase-1 hardcoded thresholds). Stale index + old
  // pages = one alert; failed restore drill = another. Empty → strip hidden.
  let alerts = $derived.by<string[]>(() => {
    const out: string[] = []
    if (stats && stats.index.stale && stats.staleness.stale_30d > 0) {
      out.push(`Index stale — ${stats.staleness.stale_30d} pages >30d old`)
    }
    if (backup && !backup.last_restore_drill_ok) {
      out.push('Backup drill failed last run')
    }
    return out
  })

  // Semantic icon color per activity kind — mirrors Activity.svelte's rule.
  function iconColorVar(kind: ActivityKind): string {
    if (kind === 'page_created') return '--color-success'
    if (kind === 'page_deleted') return '--color-danger'
    return '--color-accent' // page_edited
  }
</script>

<section class="today-page">
  <PageHead
    kicker={`Today · ${new Date().toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}`}
    title="Today"
    tagline="What deserves attention right now."
  >
    {#snippet actions()}
      <button type="button" class="action-btn" onclick={() => void refreshAll()} disabled={loading} aria-busy={loading}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-3-6.7L21 8" /><path d="M21 3v5h-5" /></svg>
        <span>{loading ? 'Refreshing…' : 'Refresh all'}</span>
      </button>
    {/snippet}
  </PageHead>

  {#if alerts.length > 0}
    <HoloPanel variant="danger" label="Alerts" title={`${alerts.length} alert${alerts.length === 1 ? '' : 's'}`} ariaLabel="Alerts needing attention" ariaLive="polite">
      <ul class="alert-list">
        {#each alerts as a, i (`a${i}`)}
          <li class="alert-row">⚠ {a}</li>
        {/each}
      </ul>
    </HoloPanel>
  {/if}

  <StateBox loading={loading && !stats} error={error} empty={false} emptyText="">
    <div class="stat-row">
      <HoloPanel label="Inbox" title="" ariaLabel="Pending inbox count">
        <button type="button" class="stat-card" onclick={() => navigate('inbox')}>
          <span class="stat-value">{pendingCount}</span>
          <span class="stat-label">pending · review now →</span>
        </button>
      </HoloPanel>
      <HoloPanel label="Index" title="" ariaLabel="Index status">
        <div class="stat-card">
          <span class="stat-value">{stats ? (stats.index.stale ? 'Stale' : 'Queryable') : '—'}</span>
          <span class="stat-label">{stats ? `built ${formatRelative(stats.index.built)}` : ''}</span>
        </div>
      </HoloPanel>
      <HoloPanel label="Today" title="" ariaLabel="Pages changed today">
        <div class="stat-card">
          <span class="stat-value">+{events.length}</span>
          <span class="stat-label">changes today</span>
        </div>
      </HoloPanel>
    </div>
  </StateBox>

  <HoloPanel label="Recent activity" title="Recent" ariaLabel="Recent activity">
    {#snippet actions()}<button type="button" class="view-all" onclick={() => navigate('activity')}>View all →</button>{/snippet}
    <StateBox loading={loading} error={null} empty={events.length === 0} emptyText="No activity today.">
      <Timeline>
        {#each events.slice(0, 5) as ev (ev.timestamp + ev.target + ev.kind)}
          <TimelineCard
            iconColorVar={iconColorVar(ev.kind)}
            timestamp={formatRelative(ev.timestamp)}
            title={ev.target}
            meta={ev.actor}
          >
            {#snippet icon()}
              {#if ev.kind === 'page_created'}
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="16" /><line x1="8" y1="12" x2="16" y2="12" /></svg>
              {:else if ev.kind === 'page_deleted'}
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6" /><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" /></svg>
              {:else}
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" /><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" /></svg>
              {/if}
            {/snippet}
          </TimelineCard>
        {/each}
      </Timeline>
    </StateBox>
  </HoloPanel>

  <HoloPanel label="System health" title="" ariaLabel="System health summary">
    {#snippet actions()}<button type="button" class="view-all" onclick={() => navigate('status')}>View →</button>{/snippet}
    {#if stats}
      <p class="health-rollup">
        ●{stats.index.stale ? 'Stale' : 'Nominal'} · {stats.pages} pages · {stats.orphans} orphans · density {stats.graph_density.toFixed(2)}
      </p>
      <p class="health-stale">
        Staleness: {stats.staleness.fresh} fresh · {stats.staleness.stale_7d} stale-7d · {stats.staleness.stale_30d} stale-30d
      </p>
    {:else}
      <p class="health-rollup">Loading…</p>
    {/if}
  </HoloPanel>
</section>

<style>
  /* Page chrome (kicker/h1/tagline/refresh button) lives in PageHead now. */
  .today-page { display: flex; flex-direction: column; gap: var(--space-md); }

  .alert-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .alert-row { color: var(--text-danger); font-family: var(--font-body); font-size: var(--text-body); }

  .stat-row {
    display: grid; gap: var(--space-md);
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  }
  /* The card is the clickable surface; the HoloPanel supplies the chrome.
   * Resetting button defaults so the value reads as a stat, not a CTA. */
  .stat-card {
    display: flex; flex-direction: column; gap: 4px;
    padding: 0; background: transparent; border: none;
    cursor: pointer; text-align: left; width: 100%;
    font: inherit; color: inherit;
  }
  .stat-card:focus-visible { outline: var(--border-focus); outline-offset: 2px; }
  .stat-value {
    font-family: var(--font-display); font-size: 2rem;
    font-weight: var(--weight-semibold); color: var(--text-primary);
    line-height: 1.1;
  }
  .stat-label { color: var(--text-secondary); font-size: var(--text-body); }

  .view-all {
    padding: var(--space-xs) var(--space-sm);
    background: transparent; border: 1px solid var(--color-accent);
    border-radius: var(--radius-md); color: var(--color-accent);
    cursor: pointer;
    font-family: var(--font-body); font-size: var(--text-body);
  }
  .view-all:hover { background: var(--surface-accent-soft); }

  .health-rollup {
    margin: 0; color: var(--text-secondary);
    font-family: var(--font-mono); font-size: var(--text-mono);
  }
  .health-stale {
    margin: 4px 0 0; color: var(--text-secondary);
    font-family: var(--font-mono); font-size: var(--text-mono);
  }
</style>
