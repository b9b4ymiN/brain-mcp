<script lang="ts">
  /**
   * Status — wiki health dashboard (Layout Option 2: hero band + detail grid).
   *
   * Hero band: SYSTEM HEALTH rollup (Nominal/Stale/Degraded) + key metrics +
   *   inline "Reindex now" action (POST /index/rebuild → poll /ops/jobs →
   *   refresh). Two-step confirm (NOT a modal — product ban).
   * Detail grid: Staleness bars, Graph shape, Backup (reuses /ops/backup-health).
   *
   * Severity rule (WikiStats has no `index.queryable` field, so derive here):
   *   - degraded: index never built (`index.built === null`)
   *   - stale:    index marked stale AND any page is 7d+ old
   *   - nominal:  otherwise
   */
  import { onMount } from 'svelte'
  import {
    status as apiStatus,
    opsBackupHealth as apiOpsBackupHealth,
    indexRebuild as apiIndexRebuild,
    opsJobs as apiOpsJobs,
    type WikiStats,
    type BackupHealth,
    ApiError,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { formatRelative } from '../lib/format'
  import StateBox from '../components/StateBox.svelte'
  import HoloPanel from '../components/HoloPanel.svelte'
  import PageHead from '../components/PageHead.svelte'

  interface Props {
    session: SessionStore
    toasts: ToastStore
  }
  // `session`/`toasts` are accepted for API symmetry with sibling pages; this
  // page is read-only today and pushes nothing. Kept on the props list so
  // future enhancements (e.g. warning toasts on partial failures) need no
  // caller churn.
  let { session, toasts }: Props = $props()

  // Per-section $state — mirrors Operations.svelte's race-guard pattern.
  let stats = $state<WikiStats | null>(null)
  let statsLoading = $state(false)
  let statsError = $state<string | null>(null)
  let statsSeq = 0

  let backup = $state<BackupHealth | null>(null)
  let backupLoading = $state(false)
  let backupError = $state<string | null>(null)
  let backupSeq = 0

  // ── Reindex action state ────────────────────────────────────────────────
  // The hero-band "Reindex now" button kicks a full rebuild inline (with a
  // two-step confirm) and polls /ops/jobs until the registry drains, then
  // refreshes /status so the new built/last_* fields show. Mirrors the
  // Config page's handleRebuild — kept here so the operator can act without
  // leaving Status. The other index action (incremental Update) stays on
  // Config: Update only matters when an incremental sync is cheaper than a
  // rebuild, which is an operator call Config surfaces with the staleness
  // context (state.toml commit vs HEAD). Status is the "is it healthy?"
  // view; the only action it offers is the panic-button rebuild.
  let reindexConfirming = $state(false)
  let reindexing = $state(false)

  function handleReadError(cause: unknown, setErr: (m: string) => void, what: string): void {
    if (cause instanceof ApiError) {
      setErr(cause.code === 'unauthorized' ? 'Session expired — sign in again.' : `${what}: ${cause.code}`)
    } else {
      setErr(`${what}: backend unreachable`)
    }
  }

  async function refreshStats(): Promise<void> {
    const seq = ++statsSeq
    statsLoading = true
    statsError = null
    try {
      const result = await apiStatus()
      if (seq !== statsSeq) return
      stats = result
    } catch (cause) {
      if (seq !== statsSeq) return
      handleReadError(cause, (m) => (statsError = m), 'status')
      stats = null
    } finally {
      if (seq === statsSeq) statsLoading = false
    }
  }

  async function refreshBackup(): Promise<void> {
    const seq = ++backupSeq
    backupLoading = true
    backupError = null
    try {
      const result = await apiOpsBackupHealth()
      if (seq !== backupSeq) return
      backup = result
    } catch (cause) {
      if (seq !== backupSeq) return
      handleReadError(cause, (m) => (backupError = m), 'backup health')
      backup = null
    } finally {
      if (seq === backupSeq) backupLoading = false
    }
  }

  async function refreshAll(): Promise<void> {
    await Promise.all([void refreshStats(), void refreshBackup()])
  }

  /** Kick a full index rebuild from the Status hero band. Two-step inline
   *  confirm (NOT a modal — product ban), then poll /ops/jobs until the
   *  registry drains, then refresh /status. */
  async function handleReindex(): Promise<void> {
    if (reindexing) return
    reindexing = true
    reindexConfirming = false
    try {
      // Wiki name comes from the loaded stats — /index/rebuild needs it.
      const wiki = stats?.wiki ?? 'brain'
      const { job_id } = await apiIndexRebuild(wiki)
      toasts.push('info', 'Reindex queued', `Background job ${job_id} — refreshing when it completes…`)
      const POLL_MS = 1000
      const MAX_POLLS = 60
      let polled = 0
      let sawOurJob = false
      while (polled < MAX_POLLS) {
        await sleep(POLL_MS)
        polled++
        const summary = await apiOpsJobs()
        if (!sawOurJob && (summary.active > 0 || summary.queued > 0)) sawOurJob = true
        if (summary.active === 0 && summary.queued === 0) break
      }
      await refreshStats()
      if (polled >= MAX_POLLS) {
        toasts.push('warning', 'Reindex still running', `Polled ${MAX_POLLS}s — hit Refresh to check.`)
      } else {
        toasts.push('success', 'Reindex complete', `Polled ${polled}s.`)
      }
    } catch (cause) {
      handleReadError(cause, () => {}, 'index rebuild')
      toasts.push('error', 'Reindex failed', cause instanceof ApiError ? cause.code : 'backend unreachable')
    } finally {
      reindexing = false
    }
  }

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  onMount(() => {
    void refreshAll()
  })

  // Severity → status word + dot variant. Derived from WikiStats only (no
  // `queryable` field — see header comment for the rule).
  type Severity = 'nominal' | 'stale' | 'degraded'
  let severity = $derived.by<Severity>(() => {
    if (!stats) return 'nominal'
    if (stats.index.built === null) return 'degraded'
    if (stats.index.stale && (stats.staleness.stale_30d > 0 || stats.staleness.stale_7d > 0)) {
      return 'stale'
    }
    return 'nominal'
  })
  let severityWord = $derived(
    severity === 'nominal' ? 'Nominal' : severity === 'stale' ? 'Stale' : 'Degraded',
  )

  // Staleness bar widths (out of total pages). Guard divide-by-zero.
  let totalStale = $derived(
    stats ? stats.staleness.fresh + stats.staleness.stale_7d + stats.staleness.stale_30d : 0,
  )
</script>

<section class="status-page">
  <PageHead
    kicker="Sector health"
    title="Status"
    tagline="Index health, staleness, graph shape, and backup. Read-only — destructive actions live on Config."
  >
    {#snippet actions()}
      <button
        type="button"
        class="action-btn"
        onclick={() => void refreshAll()}
        disabled={statsLoading || backupLoading}
        aria-busy={statsLoading || backupLoading}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <path d="M21 3v5h-5" />
        </svg>
        <span>{statsLoading || backupLoading ? 'Refreshing…' : 'Refresh all'}</span>
      </button>
    {/snippet}
  </PageHead>

  <!-- Hero band -->
  <HoloPanel
    variant="primary"
    label="System health"
    title="Status"
    ariaLabel="System health summary"
    ariaLive="polite"
    ariaBusy={statsLoading}
  >
    <StateBox
      loading={statsLoading}
      error={statsError}
      empty={stats === null && !statsLoading}
      emptyText="No status available."
    >
      {#if stats}
        <div class="hero-rollup">
          <p class="severity">
            <span class="severity-dot severity-dot--{severity}" aria-hidden="true"></span>
            <span class="severity-word">{severityWord}</span>
          </p>
          <p class="hero-meta">
            Index built {formatRelative(stats.index.built)} ·
            {stats.index.stale ? 'stale' : 'queryable'} ·
            {stats.pages} pages · {stats.orphans} orphans
          </p>
          {#if reindexConfirming}
            <span class="reindex-confirm">
              <span class="reindex-confirm-prompt">Full rebuild queues a background job. Continue?</span>
              <button
                type="button"
                class="reindex-link reindex-link--danger"
                onclick={() => void handleReindex()}
                disabled={reindexing}
                aria-busy={reindexing}
              >
                {reindexing ? 'Reindexing…' : 'Confirm reindex'}
              </button>
              <button
                type="button"
                class="reindex-link reindex-link--ghost"
                onclick={() => (reindexConfirming = false)}
                disabled={reindexing}
              >
                Cancel
              </button>
            </span>
          {:else}
            <button
              type="button"
              class="reindex-link"
              onclick={() => (reindexConfirming = true)}
              disabled={reindexing}
              aria-busy={reindexing}
            >
              {reindexing ? 'Reindexing…' : 'Reindex now'}
            </button>
          {/if}
        </div>
      {/if}
    </StateBox>
  </HoloPanel>

  <!-- Detail grid -->
  <div class="detail-grid">
    <HoloPanel label="Staleness buckets" title="Staleness" ariaLabel="Page staleness" ariaBusy={statsLoading}>
      <StateBox loading={statsLoading} error={statsError} empty={!stats} emptyText="No staleness data.">
        {#if stats && totalStale > 0}
          <div class="staleness-bars" role="group" aria-label="Staleness buckets">
            <div class="stale-row">
              <span class="stale-label">Fresh (≤7d)</span>
              <div class="stale-bar-bg">
                <div class="stale-bar stale-bar--fresh" style="width: {(stats.staleness.fresh / totalStale) * 100}%"></div>
              </div>
              <span class="stale-count">{stats.staleness.fresh}</span>
            </div>
            <div class="stale-row">
              <span class="stale-label">7–30d</span>
              <div class="stale-bar-bg">
                <div class="stale-bar stale-bar--warn" style="width: {(stats.staleness.stale_7d / totalStale) * 100}%"></div>
              </div>
              <span class="stale-count">{stats.staleness.stale_7d}</span>
            </div>
            <div class="stale-row">
              <span class="stale-label">&gt;30d</span>
              <div class="stale-bar-bg">
                <div class="stale-bar stale-bar--danger" style="width: {(stats.staleness.stale_30d / totalStale) * 100}%"></div>
              </div>
              <span class="stale-count">{stats.staleness.stale_30d}</span>
            </div>
          </div>
        {/if}
      </StateBox>
    </HoloPanel>

    <HoloPanel label="Graph shape" title="Graph" ariaLabel="Graph metrics" ariaBusy={statsLoading}>
      <StateBox loading={statsLoading} error={statsError} empty={!stats} emptyText="No graph data.">
        {#if stats}
          <dl class="metric-list">
            <div class="metric"><dt>Density</dt><dd>{stats.graph_density.toFixed(2)}</dd></div>
            <div class="metric"><dt>Avg links</dt><dd>{stats.avg_connections.toFixed(2)}</dd></div>
            <div class="metric"><dt>Diameter</dt><dd>{stats.diameter !== null ? stats.diameter.toFixed(1) : '—'}</dd></div>
            <div class="metric"><dt>Radius</dt><dd>{stats.radius !== null ? stats.radius.toFixed(1) : '—'}</dd></div>
            <div class="metric"><dt>Communities</dt><dd>{stats.communities ? stats.communities.count : '—'}</dd></div>
            <div class="metric"><dt>Center</dt><dd>{stats.center.length > 0 ? stats.center.slice(0, 3).join(', ') : '—'}</dd></div>
          </dl>
        {/if}
      </StateBox>
    </HoloPanel>

    <HoloPanel label="Backup health" title="Backup" ariaLabel="Backup health" ariaBusy={backupLoading}>
      <StateBox loading={backupLoading} error={backupError} empty={!backup} emptyText="No backup data.">
        {#if backup}
          <dl class="metric-list">
            <div class="metric"><dt>Last backup</dt><dd>{formatRelative(backup.last_backup_at)}</dd></div>
            <div class="metric"><dt>Drill OK</dt><dd>{backup.last_restore_drill_ok ? 'Yes' : 'No'}</dd></div>
          </dl>
        {/if}
      </StateBox>
    </HoloPanel>
  </div>
</section>

<style>
  /* Page chrome (kicker/h1/tagline/refresh button) lives in PageHead now. */
  .status-page { display: flex; flex-direction: column; gap: var(--space-md); }

  .hero-rollup { display: flex; flex-direction: column; gap: var(--space-xs); }
  .severity { display: flex; align-items: center; gap: var(--space-xs); margin: 0; }
  .severity-dot {
    width: 10px; height: 10px; border-radius: 50%;
    background: var(--color-success);
  }
  .severity-dot--stale { background: var(--color-warning); }
  .severity-dot--degraded { background: var(--color-danger); }
  .severity-word {
    font-family: var(--font-display); font-size: var(--text-title);
    font-weight: var(--weight-semibold); color: var(--text-primary);
  }
  .hero-meta { margin: 0; color: var(--text-secondary); font-family: var(--font-mono); font-size: var(--text-mono); }
  .reindex-link {
    align-self: flex-start; padding: var(--space-xs) var(--space-sm);
    background: transparent; border: 1px solid var(--color-accent);
    border-radius: var(--radius-md); color: var(--color-accent);
    font-family: var(--font-body); font-size: var(--text-body); cursor: pointer;
  }
  .reindex-link:hover:not(:disabled) { background: var(--surface-accent-soft); }
  .reindex-link:disabled { opacity: 0.6; cursor: not-allowed; }
  .reindex-link--danger { border-color: var(--color-danger); color: var(--color-danger); }
  .reindex-link--danger:hover:not(:disabled) { background: var(--surface-danger-soft); }
  .reindex-link--ghost { border-color: var(--color-hairline); color: var(--text-secondary); }
  .reindex-confirm {
    display: flex; flex-direction: column; align-items: flex-start;
    gap: var(--space-xs); padding: var(--space-sm);
    background: var(--surface-danger-soft);
    border: 1px solid var(--color-danger); border-radius: var(--radius-md);
  }
  .reindex-confirm-prompt {
    color: var(--text-primary); font-size: var(--text-body);
  }
  .reindex-confirm .reindex-link { align-self: flex-start; }

  .detail-grid {
    display: grid; gap: var(--space-md);
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  }

  .staleness-bars { display: flex; flex-direction: column; gap: var(--space-sm); }
  .stale-row { display: grid; grid-template-columns: 6em 1fr 3em; align-items: center; gap: var(--space-sm); }
  .stale-label { color: var(--text-secondary); font-size: var(--text-body); }
  .stale-bar-bg { height: 8px; background: var(--color-ink-deep); border-radius: 4px; overflow: hidden; }
  .stale-bar { height: 100%; }
  .stale-bar--fresh { background: var(--color-success); }
  .stale-bar--warn  { background: var(--color-warning); }
  .stale-bar--danger { background: var(--color-danger); }
  .stale-count { text-align: right; font-family: var(--font-mono); color: var(--text-primary); }

  .metric-list { display: flex; flex-direction: column; gap: var(--space-xs); margin: 0; }
  .metric { display: flex; justify-content: space-between; gap: var(--space-sm); }
  .metric dt { color: var(--text-secondary); font-family: var(--font-body); font-size: var(--text-body); }
  .metric dd { margin: 0; font-family: var(--font-mono); font-size: var(--text-mono); color: var(--text-primary); text-align: right; }
</style>
