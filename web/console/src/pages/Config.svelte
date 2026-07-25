<script lang="ts">
  /**
   * Config — read-only config view + index management (Layout: two panels).
   *
   * Panel 1 (read-only): `GET /config` → wiki spaces table + server +
   *   extraction + logging + index sections. All secret-bearing fields are
   *   surfaced as env-var NAMES (the Rust projection never emits resolved
   *   values), so the whole payload is display-safe. A "Show advanced"
   *   toggle reveals the raw JSON.
   *
   * Panel 2 (index management): `GET /index-status` display + two actions:
   *   - `POST /index/update` (cheap incremental path)
   *   - `POST /index/rebuild` (full rebuild, background job)
   *   Rebuild shows an INLINE confirmation (NOT a modal — product ban on
   *   dialogs for reversible/index ops). Both push a toast + refresh status
   *   on success/failure.
   *
   * Mirrors the Status.svelte page-chrome pattern: kicker + h1 + tagline +
   * Refresh button. Per-section `$state` + race-guard. No `{@html}`.
   */
  import { onMount } from 'svelte'
  import {
    config as apiConfig,
    indexStatus as apiIndexStatus,
    indexUpdate as apiIndexUpdate,
    indexRebuild as apiIndexRebuild,
    type ConfigView,
    type IndexStatus,
    ApiError,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { formatRelative } from '../lib/format'
  import StateBox from '../components/StateBox.svelte'
  import HoloPanel from '../components/HoloPanel.svelte'

  interface Props {
    session: SessionStore
    toasts: ToastStore
  }
  let { session, toasts }: Props = $props()
  // `session` is accepted for API symmetry with sibling pages; current page
  // is read/action-only and does not flip auth state. Kept on the props list
  // so future enhancements (e.g. gating rebuild behind recent-reauth) need
  // no caller churn.

  // ── Read-only config state ──────────────────────────────────────────────
  let cfg = $state<ConfigView | null>(null)
  let cfgLoading = $state(false)
  let cfgError = $state<string | null>(null)
  let cfgSeq = 0
  let showAdvanced = $state(false)

  // ── Index status state ──────────────────────────────────────────────────
  let idx = $state<IndexStatus | null>(null)
  let idxLoading = $state(false)
  let idxError = $state<string | null>(null)
  let idxSeq = 0

  // ── Action state (separate from read state so a click doesn't blank the
  //    status display mid-flight) ──────────────────────────────────────────
  let updating = $state(false)
  let rebuilding = $state(false)
  let rebuildConfirming = $state(false)

  function handleReadError(cause: unknown, setErr: (m: string) => void, what: string): void {
    if (cause instanceof ApiError) {
      setErr(cause.code === 'unauthorized' ? 'Session expired — sign in again.' : `${what}: ${cause.code}`)
    } else {
      setErr(`${what}: backend unreachable`)
    }
  }

  function handleActionError(cause: unknown, what: string): string {
    if (cause instanceof ApiError) {
      if (cause.code === 'unauthorized') return 'Session expired — sign in again.'
      return `${what}: ${cause.code}`
    }
    return `${what}: backend unreachable`
  }

  async function refreshConfig(): Promise<void> {
    const seq = ++cfgSeq
    cfgLoading = true
    cfgError = null
    try {
      const result = await apiConfig()
      if (seq !== cfgSeq) return
      cfg = result
    } catch (cause) {
      if (seq !== cfgSeq) return
      handleReadError(cause, (m) => (cfgError = m), 'config')
      cfg = null
    } finally {
      if (seq === cfgSeq) cfgLoading = false
    }
  }

  async function refreshIndex(): Promise<void> {
    const seq = ++idxSeq
    idxLoading = true
    idxError = null
    try {
      // No explicit `wiki` → server falls back to the configured default.
      const result = await apiIndexStatus()
      if (seq !== idxSeq) return
      idx = result
    } catch (cause) {
      if (seq !== idxSeq) return
      handleReadError(cause, (m) => (idxError = m), 'index status')
      idx = null
    } finally {
      if (seq === idxSeq) idxLoading = false
    }
  }

  async function refreshAll(): Promise<void> {
    await Promise.all([void refreshConfig(), void refreshIndex()])
  }

  async function handleUpdate(): Promise<void> {
    if (updating || !idx) return
    updating = true
    try {
      const report = await apiIndexUpdate(idx.wiki)
      toasts.push(
        'success',
        'Index updated',
        `${report.updated} updated · ${report.deleted} deleted`,
      )
      await refreshIndex()
    } catch (cause) {
      toasts.push('error', 'Update failed', handleActionError(cause, 'index update'))
    } finally {
      updating = false
    }
  }

  async function handleRebuild(): Promise<void> {
    if (rebuilding || !idx) return
    rebuilding = true
    rebuildConfirming = false
    try {
      const { job_id } = await apiIndexRebuild(idx.wiki)
      toasts.push(
        'info',
        'Rebuild queued',
        `Background job ${job_id} — status refreshes below`,
      )
      // The rebuild is asynchronous, so a single refresh may not show the
      // new state yet. Kick one off anyway so the UI feels responsive; the
      // user can hit Refresh again once the job settles.
      await refreshIndex()
    } catch (cause) {
      toasts.push('error', 'Rebuild failed', handleActionError(cause, 'index rebuild'))
    } finally {
      rebuilding = false
    }
  }

  onMount(() => {
    void refreshAll()
  })
</script>

<section class="config-page">
  <header class="page-head">
    <div>
      <p class="page-kicker">System configuration</p>
      <h1>Config</h1>
      <p class="page-tagline">
        Read-only view of registered spaces, transport, extraction, logging, and index
        policy. Index actions run live — full rebuilds queue as background jobs.
      </p>
    </div>
    <button
      type="button"
      class="refresh-all"
      onclick={() => void refreshAll()}
      disabled={cfgLoading || idxLoading}
      aria-busy={cfgLoading || idxLoading}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
        <path d="M21 3v5h-5" />
      </svg>
      <span>{cfgLoading || idxLoading ? 'Refreshing…' : 'Refresh all'}</span>
    </button>
  </header>

  <!-- Panel 1: read-only config display -->
  <HoloPanel
    variant="primary"
    label="Configuration"
    title="Config"
    ariaLabel="Configuration overview"
    ariaLive="polite"
    ariaBusy={cfgLoading}
  >
    <StateBox
      loading={cfgLoading}
      error={cfgError}
      empty={cfg === null && !cfgLoading}
      emptyText="No configuration available."
    >
      {#if cfg}
        <!-- Wiki spaces -->
        <div class="cfg-section">
          <h3 class="cfg-section-title">Wiki spaces</h3>
          {#if cfg.wiki_spaces.length === 0}
            <p class="cfg-empty">No wiki spaces registered.</p>
          {:else}
            <table class="cfg-table">
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  <th scope="col">Path</th>
                  <th scope="col">Remote</th>
                  <th scope="col">Description</th>
                </tr>
              </thead>
              <tbody>
                {#each cfg.wiki_spaces as space (space.name)}
                  <tr>
                    <td class="cfg-mono">{space.name}</td>
                    <td class="cfg-mono cfg-path">{space.path}</td>
                    <td class="cfg-mono">{space.remote ?? '—'}</td>
                    <td>{space.description ?? '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>

        <!-- Server / transport -->
        <div class="cfg-section">
          <h3 class="cfg-section-title">Server</h3>
          <dl class="metric-list">
            <div class="metric">
              <dt>HTTP</dt>
              <dd>{cfg.server.http_enabled ? 'enabled' : 'disabled'} · port {cfg.server.http_port}</dd>
            </div>
            <div class="metric"><dt>Bind</dt><dd>{cfg.server.bind}</dd></div>
            <div class="metric">
              <dt>All interfaces</dt><dd>{cfg.server.bind_all_interfaces ? 'yes' : 'no'}</dd>
            </div>
            <div class="metric"><dt>ACP</dt><dd>{cfg.server.acp_enabled ? 'enabled' : 'disabled'}</dd></div>
            <div class="metric">
              <dt>Bootstrap username</dt>
              <dd>{cfg.server.bootstrap_username_env ?? 'default (USERNAME)'}</dd>
            </div>
            <div class="metric">
              <dt>Bootstrap password</dt>
              <dd>{cfg.server.bootstrap_password_env ?? 'default (PASSWORD)'}</dd>
            </div>
          </dl>
        </div>

        <!-- Extraction -->
        <div class="cfg-section">
          <h3 class="cfg-section-title">Extraction</h3>
          <dl class="metric-list">
            <div class="metric"><dt>Provider</dt><dd>{cfg.extraction.enabled ? 'enabled' : 'disabled'}</dd></div>
            <div class="metric"><dt>Base URL</dt><dd>{cfg.extraction.base_url || '—'}</dd></div>
            <div class="metric"><dt>API key env</dt><dd>{cfg.extraction.api_key_env || '—'}</dd></div>
            <div class="metric"><dt>Routine model</dt><dd>{cfg.extraction.routine_model || '—'}</dd></div>
            <div class="metric"><dt>Reasoning model</dt><dd>{cfg.extraction.reasoning_model || '—'}</dd></div>
          </dl>
        </div>

        <!-- Logging + Index (two-column grid) -->
        <div class="cfg-two-col">
          <div class="cfg-section">
            <h3 class="cfg-section-title">Logging</h3>
            <dl class="metric-list">
              <div class="metric"><dt>Format</dt><dd>{cfg.logging.format || '—'}</dd></div>
              <div class="metric"><dt>Rotation</dt><dd>{cfg.logging.rotation || '—'}</dd></div>
            </dl>
          </div>
          <div class="cfg-section">
            <h3 class="cfg-section-title">Index</h3>
            <dl class="metric-list">
              <div class="metric"><dt>Tokenizer</dt><dd>{cfg.index.tokenizer || '—'}</dd></div>
              <div class="metric"><dt>Auto-rebuild</dt><dd>{cfg.index.auto_rebuild ? 'enabled' : 'disabled'}</dd></div>
            </dl>
          </div>
        </div>

        <!-- Advanced: raw JSON dump -->
        <div class="cfg-advanced">
          <button
            type="button"
            class="advanced-toggle"
            aria-expanded={showAdvanced}
            onclick={() => (showAdvanced = !showAdvanced)}
          >
            {showAdvanced ? 'Hide advanced' : 'Show advanced'}
          </button>
          {#if showAdvanced}
            <pre class="cfg-raw">{JSON.stringify(cfg, null, 2)}</pre>
          {/if}
        </div>
      {/if}
    </StateBox>
  </HoloPanel>

  <!-- Panel 2: index management -->
  <HoloPanel
    label="Index management"
    title="Index"
    ariaLabel="Index status and actions"
    ariaLive="polite"
    ariaBusy={idxLoading}
  >
    <StateBox
      loading={idxLoading}
      error={idxError}
      empty={idx === null && !idxLoading}
      emptyText="No index status available."
    >
      {#if idx}
        <dl class="metric-list">
          <div class="metric"><dt>Wiki</dt><dd>{idx.wiki}</dd></div>
          <div class="metric"><dt>Path</dt><dd class="cfg-mono cfg-path">{idx.path}</dd></div>
          <div class="metric">
            <dt>Built</dt><dd>{formatRelative(idx.built)}</dd>
          </div>
          <div class="metric"><dt>Pages</dt><dd>{idx.pages}</dd></div>
          <div class="metric"><dt>Sections</dt><dd>{idx.sections}</dd></div>
          <div class="metric"><dt>Stale</dt><dd>{idx.stale ? 'yes' : 'no'}</dd></div>
          <div class="metric"><dt>Openable</dt><dd>{idx.openable ? 'yes' : 'no'}</dd></div>
          <div class="metric"><dt>Queryable</dt><dd>{idx.queryable ? 'yes' : 'no'}</dd></div>
          {#if idx.last_pages_indexed !== undefined}
            <div class="metric"><dt>Last indexed</dt><dd>{idx.last_pages_indexed} pages</dd></div>
          {/if}
          {#if idx.last_skipped !== undefined}
            <div class="metric"><dt>Last skipped</dt><dd>{idx.last_skipped}</dd></div>
          {/if}
          {#if idx.last_duration_ms !== undefined}
            <div class="metric"><dt>Last duration</dt><dd>{idx.last_duration_ms} ms</dd></div>
          {/if}
        </dl>

        <div class="index-actions" role="group" aria-label="Index actions">
          <button
            type="button"
            class="action action-secondary"
            onclick={() => void handleUpdate()}
            disabled={updating || rebuilding}
            aria-busy={updating}
          >
            {updating ? 'Updating…' : 'Update incremental'}
          </button>

          {#if rebuildConfirming}
            <span class="confirm-prompt">
              Full rebuild queues a background job. Continue?
              <button
                type="button"
                class="action action-danger"
                onclick={() => void handleRebuild()}
                disabled={rebuilding}
                aria-busy={rebuilding}
              >
                {rebuilding ? 'Queuing…' : 'Confirm rebuild'}
              </button>
              <button
                type="button"
                class="action action-cancel"
                onclick={() => (rebuildConfirming = false)}
                disabled={rebuilding}
              >
                Cancel
              </button>
            </span>
          {:else}
            <button
              type="button"
              class="action action-danger"
              onclick={() => (rebuildConfirming = true)}
              disabled={updating || rebuilding}
            >
              Rebuild full
            </button>
          {/if}
        </div>
      {/if}
    </StateBox>
  </HoloPanel>
</section>

<style>
  /* Mirror Status/Operations page chrome: kicker + h1 + tagline + refresh-all */
  .config-page { display: flex; flex-direction: column; gap: var(--space-md); }
  .page-head {
    display: flex; justify-content: space-between; align-items: flex-start;
    gap: var(--space-md); flex-wrap: wrap;
  }
  .page-kicker {
    margin: 0;
    font-family: var(--font-mono); font-size: var(--text-mono);
    color: var(--holo-cyan); letter-spacing: 0.05em; text-transform: uppercase;
  }
  .config-page h1 {
    margin: var(--space-xs) 0 0; font-family: var(--font-display);
    font-size: var(--text-title); font-weight: var(--weight-semibold);
    color: var(--text-primary);
  }
  .page-tagline { margin: var(--space-xs) 0 0; color: var(--text-secondary); max-width: 65ch; }
  .refresh-all {
    display: inline-flex; align-items: center; gap: var(--space-xs);
    padding: var(--space-xs) var(--space-sm);
    background: var(--surface-active-nav); border: var(--border-hairline);
    border-radius: var(--radius-md); color: var(--text-primary);
    font-family: var(--font-body); font-size: var(--text-body);
    cursor: pointer;
  }
  .refresh-all:hover:not(:disabled) { background: var(--surface-raised); }
  .refresh-all:disabled { opacity: 0.6; cursor: not-allowed; }

  /* ── Config sections ───────────────────────────────────────────────── */
  .cfg-section {
    display: flex; flex-direction: column; gap: var(--space-xs);
    padding-top: var(--space-sm);
    border-top: 1px solid var(--color-hairline);
  }
  .cfg-section:first-child {
    padding-top: 0; border-top: none;
  }
  .cfg-section-title {
    margin: 0;
    font-family: var(--font-display); font-size: var(--text-label);
    font-weight: var(--weight-semibold); color: var(--holo-cyan);
    letter-spacing: 0.04em; text-transform: uppercase;
  }
  .cfg-empty {
    margin: 0; color: var(--text-secondary); font-style: italic;
    font-size: var(--text-body);
  }
  .cfg-two-col {
    display: grid; gap: var(--space-sm);
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  }
  /* Each column keeps the section's top divider; suppress the first-row
     divider so the grid reads as a fresh band rather than two orphan rows. */
  .cfg-two-col .cfg-section { border-top: none; padding-top: 0; }

  /* ── Wiki spaces table ─────────────────────────────────────────────── */
  .cfg-table {
    width: 100%; border-collapse: collapse;
    font-family: var(--font-body); font-size: var(--text-body);
  }
  .cfg-table th {
    text-align: left; padding: var(--space-xs) var(--space-sm);
    color: var(--text-secondary); font-weight: var(--weight-medium);
    border-bottom: 1px solid var(--color-hairline);
  }
  .cfg-table td {
    padding: var(--space-xs) var(--space-sm);
    color: var(--text-primary); border-bottom: 1px solid var(--color-hairline);
    vertical-align: top; word-break: break-word;
  }
  .cfg-table tbody tr:last-child td { border-bottom: none; }
  .cfg-mono { font-family: var(--font-mono); font-size: var(--text-mono); }
  /* Long filesystem paths shouldn't blow out the panel width. */
  .cfg-path { word-break: break-all; }

  /* ── Metric list (mirrors Status.svelte) ───────────────────────────── */
  .metric-list { display: flex; flex-direction: column; gap: var(--space-xs); margin: 0; }
  .metric { display: flex; justify-content: space-between; gap: var(--space-sm); }
  .metric dt { color: var(--text-secondary); font-family: var(--font-body); font-size: var(--text-body); }
  .metric dd {
    margin: 0; font-family: var(--font-mono); font-size: var(--text-mono);
    color: var(--text-primary); text-align: right; word-break: break-word;
  }

  /* ── Advanced toggle + raw JSON ────────────────────────────────────── */
  .cfg-advanced { padding-top: var(--space-sm); border-top: 1px solid var(--color-hairline); }
  .advanced-toggle {
    padding: var(--space-xs) var(--space-sm);
    background: transparent; border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md); color: var(--text-secondary);
    font-family: var(--font-body); font-size: var(--text-body); cursor: pointer;
  }
  .advanced-toggle:hover { background: var(--surface-raised); color: var(--text-primary); }
  .cfg-raw {
    margin: var(--space-xs) 0 0; padding: var(--space-sm);
    background: var(--surface-sunken); border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    font-family: var(--font-mono); font-size: var(--text-mono);
    color: var(--text-secondary); white-space: pre-wrap;
    word-break: break-word; max-height: 24rem; overflow: auto;
  }

  /* ── Index actions ─────────────────────────────────────────────────── */
  .index-actions {
    display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-sm);
    padding-top: var(--space-sm); border-top: 1px solid var(--color-hairline);
  }
  .action {
    min-height: 40px; padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-md); border: 1px solid var(--color-hairline);
    background: transparent; color: var(--text-primary);
    font-family: var(--font-body); font-size: var(--text-body);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }
  .action:disabled { opacity: 0.55; cursor: not-allowed; }
  .action-secondary {
    border-color: var(--color-accent); color: var(--color-accent);
  }
  .action-secondary:hover:not(:disabled) { background: var(--surface-accent-soft); }
  .action-danger {
    border-color: var(--color-danger); color: var(--color-danger);
  }
  .action-danger:hover:not(:disabled) { background: var(--surface-danger-soft); }
  .action-cancel { color: var(--text-secondary); }
  .action-cancel:hover:not(:disabled) { background: var(--surface-raised); }

  .confirm-prompt {
    display: inline-flex; flex-wrap: wrap; align-items: center; gap: var(--space-xs);
    color: var(--text-secondary); font-size: var(--text-body);
  }
</style>
