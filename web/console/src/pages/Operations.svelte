<script lang="ts">
  /**
   * Operations — the operations + trust dashboard (Task E3.3 Part B + Part D
   * hard-purge section).
   *
   * Each card is its OWN fetch (parallel `Promise.all` on mount + per-section
   * refresh buttons). 4-state coverage via `<StateBox>` (loading / error /
   * empty / data). Per-section race guards use the same monotonic-seq pattern
   * as Inbox.svelte + Entity.svelte so a stale refresh cannot clobber a
   * newer one.
   *
   * Sections:
   *   1. Trust       — contradictions + stale flags + optional retrieval
   *                    trace (`api.trust()`).
   *   2. Clients     — `ClientActivity[]` table.
   *   3. Jobs        — `JobSummary` big-number card.
   *   4. Evals       — domain input → `EvalSummary` card.
   *   5. Backup      — `BackupHealth` card (Phase F callout when drill=false).
   *   6. Hard purge  — destructive-action surface (textarea → preview →
   *                    `<DestructiveDialog>` with full guard chain: preview +
   *                    nonce + reauth + warning). UI-enforced — no path can
   *                    call `/purge/execute` without satisfying all gates.
   *
   * Anti-XSS (Part F): every dynamic string (subject, claim id, client label,
   * capabilities, warning message, preview items, nonce, flash text) is bound
   * as text via Svelte's `{value}` (auto-escaped). There is NO `{@html}` in
   * this file. The seeded XSS payload (if it ever reached trust/clients)
   * would render as literal text.
   */
  import { onMount } from 'svelte'
  import {
    trust as apiTrust,
    opsClients as apiOpsClients,
    opsJobs as apiOpsJobs,
    opsEvals as apiOpsEvals,
    opsBackupHealth as apiOpsBackupHealth,
    destructiveWarning as apiDestructiveWarning,
    purgePreview as apiPurgePreview,
    purgeExecute as apiPurgeExecute,
    ApiError,
    type TrustResponse,
    type ClientActivity,
    type JobSummary,
    type EvalSummary,
    type BackupHealth,
    type DestructiveWarning,
    type PurgePreview,
    type PurgeReceipt,
    type DestructivePreviewItem,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import StateBox from '../components/StateBox.svelte'
  import DestructiveDialog from '../components/DestructiveDialog.svelte'
  import { formatDate } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // ── Trust section ────────────────────────────────────────────────────────
  let trustData = $state<TrustResponse | null>(null)
  let trustLoading = $state(false)
  let trustError = $state<string | null>(null)
  let trustSeq = 0

  // ── Clients section ──────────────────────────────────────────────────────
  let clients = $state<ClientActivity[]>([])
  let clientsLoading = $state(false)
  let clientsError = $state<string | null>(null)
  let clientsSeq = 0

  // ── Jobs section ─────────────────────────────────────────────────────────
  let jobs = $state<JobSummary | null>(null)
  let jobsLoading = $state(false)
  let jobsError = $state<string | null>(null)
  let jobsSeq = 0

  // ── Evals section ────────────────────────────────────────────────────────
  let evalsDomain = $state('')
  let evals = $state<EvalSummary | null>(null)
  let evalsLoading = $state(false)
  let evalsError = $state<string | null>(null)
  let evalsSeq = 0

  // ── Backup health section ────────────────────────────────────────────────
  let backup = $state<BackupHealth | null>(null)
  let backupLoading = $state(false)
  let backupError = $state<string | null>(null)
  let backupSeq = 0

  // ── Hard purge section (the destructive surface) ─────────────────────────
  let purgeInput = $state('')
  let purgePreviewing = $state(false)
  let purgePreview = $state<PurgePreview | null>(null)
  let purgeWarning = $state<DestructiveWarning | null>(null)
  let purgeDialogOpen = $state(false)
  let purgeExecuting = $state(false)

  onMount(() => {
    void refreshAll()
  })

  // ── Refresh orchestration ────────────────────────────────────────────────
  async function refreshAll(): Promise<void> {
    // Fire the independent reads in parallel — they don't share state. Evals
    // is omitted (requires a domain). Each section's seq guard protects
    // against a stale refresh from clobbering a newer one.
    await Promise.all([
      void refreshTrust(),
      void refreshClients(),
      void refreshJobs(),
      void refreshBackup(),
    ])
  }

  // ── Per-section refresh helpers ──────────────────────────────────────────
  // Each helper mirrors Inbox.svelte's race-guard pattern: bump the section's
  // seq, set loading, fetch, and write the result back ONLY if the seq still
  // matches. 401 → session.clear + flash (the page-level session-expiry
  // pattern from Inbox/Entity).

  async function refreshTrust(): Promise<void> {
    const seq = ++trustSeq
    trustLoading = true
    trustError = null
    try {
      const result = await apiTrust()
      if (seq !== trustSeq) return
      trustData = result
    } catch (cause) {
      if (seq !== trustSeq) return
      handleReadError(cause, (msg) => (trustError = msg), 'trust')
      trustData = null
    } finally {
      if (seq === trustSeq) trustLoading = false
    }
  }

  async function refreshClients(): Promise<void> {
    const seq = ++clientsSeq
    clientsLoading = true
    clientsError = null
    try {
      const result = await apiOpsClients()
      if (seq !== clientsSeq) return
      clients = result
    } catch (cause) {
      if (seq !== clientsSeq) return
      handleReadError(cause, (msg) => (clientsError = msg), 'clients')
      clients = []
    } finally {
      if (seq === clientsSeq) clientsLoading = false
    }
  }

  async function refreshJobs(): Promise<void> {
    const seq = ++jobsSeq
    jobsLoading = true
    jobsError = null
    try {
      const result = await apiOpsJobs()
      if (seq !== jobsSeq) return
      jobs = result
    } catch (cause) {
      if (seq !== jobsSeq) return
      handleReadError(cause, (msg) => (jobsError = msg), 'jobs')
      jobs = null
    } finally {
      if (seq === jobsSeq) jobsLoading = false
    }
  }

  async function refreshEvals(event: SubmitEvent): Promise<void> {
    event.preventDefault()
    const domain = evalsDomain.trim()
    if (!domain) {
      evalsError = 'Enter a domain (e.g. stocks).'
      return
    }
    const seq = ++evalsSeq
    evalsLoading = true
    evalsError = null
    try {
      const result = await apiOpsEvals(domain)
      if (seq !== evalsSeq) return
      evals = result
    } catch (cause) {
      if (seq !== evalsSeq) return
      handleReadError(cause, (msg) => (evalsError = msg), 'evals')
      evals = null
    } finally {
      if (seq === evalsSeq) evalsLoading = false
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
      handleReadError(cause, (msg) => (backupError = msg), 'backup health')
      backup = null
    } finally {
      if (seq === backupSeq) backupLoading = false
    }
  }

  /**
   * Shared read-error handler. 401 → session.clear + flash + early return
   * (the section's error message stays null so the session-expiry banner
   * dominates — App.svelte re-renders the login form on `session.clear()`).
   */
  function handleReadError(
    cause: unknown,
    setError: (msg: string) => void,
    label: string,
  ): void {
    if (cause instanceof ApiError && cause.status === 401) {
      session.clear()
      session.pushFlash('error', 'Session expired — sign in again.')
      return
    }
    setError(
      cause instanceof ApiError
        ? `Failed to load ${label} (${cause.code}).`
        : `Failed to load ${label} — is the backend running on :8080?`,
    )
  }

  // ── Hard purge flow ──────────────────────────────────────────────────────
  //
  // Phase 1 (preview): textarea → list of object_ids → `POST /purge/preview`
  // returns the `PurgePreview` (preview_hash + nonce + targets + expiry) AND
  // the `DestructiveWarning` (the hard-purge warning the UI MUST display
  // before any execute click). We open the dialog with both.
  //
  // Phase 2 (execute): the user must (a) type the nonce verbatim into the
  // dialog's confirmation input, AND (b) re-auth via the embedded
  // `<ReauthForm>`. Only then does the Yes button enable, and only then can
  // `purgeExecute(preview_hash, nonce)` fire. The body sends the ORIGINAL
  // server nonce (NOT the typed value — typing is a confirmation gate).
  async function startPurgePreview(): Promise<void> {
    if (purgePreviewing) return
    const ids = purgeInput
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0)
    if (ids.length === 0) {
      session.pushFlash('error', 'Enter at least one object id to purge.')
      return
    }
    purgePreviewing = true
    try {
      const result = await apiPurgePreview(ids)
      purgePreview = result.preview
      purgeWarning = result.warning
      purgeDialogOpen = true
    } catch (cause) {
      if (cause instanceof ApiError) {
        if (cause.status === 401) {
          session.clear()
          session.pushFlash('error', 'Session expired — sign in again.')
          return
        }
        session.pushFlash(
          'error',
          `Purge preview failed (${cause.code}).`,
        )
        return
      }
      session.pushFlash(
        'error',
        'Purge preview failed — is the backend running on :8080?',
      )
    } finally {
      purgePreviewing = false
    }
  }

  async function onPurgeExecute(): Promise<void> {
    if (purgeExecuting || !purgePreview) return
    purgeExecuting = true
    try {
      const receipt: PurgeReceipt = await apiPurgeExecute(
        purgePreview.preview_hash,
        purgePreview.nonce,
      )
      session.pushFlash(
        'success',
        `Hard purge ${receipt.state} (purge_id=${receipt.purge_id.slice(0, 8)}…).`,
      )
      closePurgeDialog()
      // Reset the textarea + cached preview so a second purge starts clean.
      purgeInput = ''
      purgePreview = null
      purgeWarning = null
    } catch (cause) {
      if (cause instanceof ApiError) {
        if (cause.status === 401) {
          session.clear()
          session.pushFlash('error', 'Session expired — sign in again.')
          closePurgeDialog()
          return
        }
        if (cause.status === 403) {
          // reauth_required OR CSRF — either way the user must re-auth. Keep
          // the dialog open so they can re-auth inline without losing the
          // preview.
          session.pushFlash(
            'error',
            cause.code === 'reauth_required'
              ? 'Re-authentication required — use the form below.'
              : 'Session expired or CSRF failed — please sign in again.',
          )
          return
        }
        session.pushFlash('error', `Purge failed (${cause.code}).`)
        return
      }
      session.pushFlash(
        'error',
        'Purge failed — is the backend running on :8080?',
      )
    } finally {
      purgeExecuting = false
    }
  }

  function closePurgeDialog(): void {
    if (purgeExecuting) return
    purgeDialogOpen = false
  }

  function cancelPurgePreview(): void {
    purgePreview = null
    purgeWarning = null
    purgeDialogOpen = false
  }

  // Build the structured preview items for the destructive dialog. The
  // server's `purgePreview.targets` is a bare string list; we project each
  // into a `DestructivePreviewItem` so the dialog renders a uniform table.
  let purgePreviewItems = $derived<DestructivePreviewItem[]>(
    (purgePreview?.targets ?? []).map((id) => ({
      target_kind: 'object',
      target_id: id,
      effect: 'Content key destroyed (irreversible).',
    })),
  )
</script>

<section class="ops-page">
  <header class="ops-page-head">
    <div>
      <h1>Operations</h1>
      <p class="ops-tagline">
        Trust, clients, jobs, evals, and backup health. Each panel refreshes independently.
      </p>
    </div>
    <button
      type="button"
      class="ops-refresh-all"
      onclick={() => void refreshAll()}
      disabled={trustLoading || clientsLoading || jobsLoading || backupLoading}
      aria-busy={trustLoading || clientsLoading || jobsLoading || backupLoading}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
        <path d="M21 3v5h-5" />
      </svg>
      <span>{trustLoading || clientsLoading || jobsLoading || backupLoading ? 'Refreshing…' : 'Refresh all'}</span>
    </button>
  </header>

  <div class="ops-grid">
    <!-- ── Trust (primary surface — engineer acts on this) ────────────── -->
    <section class="ops-panel ops-panel--primary" aria-label="Trust surface" aria-live="polite" aria-busy={trustLoading}>
      <header class="ops-panel-head">
        <h2>Trust</h2>
        <button
          type="button"
          class="ops-icon-btn"
          onclick={() => void refreshTrust()}
          disabled={trustLoading}
          aria-label="Refresh trust"
        >
          {#if trustLoading}
            <span aria-hidden="true">…</span>
          {:else}
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          {/if}
        </button>
      </header>
      <StateBox
        loading={trustLoading}
        error={trustError}
        empty={(trustData?.contradictions.length ?? 0) +
          (trustData?.stale.length ?? 0) ===
          0 && trustData !== null}
        emptyText="No contradictions or stale flags."
      >
        {#if trustData}
          <div class="ops-trust-body">
            <div class="ops-trust-col">
              <h3 class="ops-section-label">
                Contradictions
                {#if trustData.contradictions.length > 0}
                  <span class="ops-count ops-count--danger">{trustData.contradictions.length}</span>
                {/if}
              </h3>
              {#if trustData.contradictions.length > 0}
                <ul class="ops-flag-list">
                  {#each trustData.contradictions as flag, i (`c-${i}`)}
                    {#if flag.kind === 'contradiction'}
                      <li class="ops-flag ops-flag--danger">
                        <span class="ops-flag-ids">
                          {#each flag.claim_ids as id, j (`c-${i}-${j}`)}
                            <code class="ops-mono">{id}</code>{#if j < flag.claim_ids.length - 1}<span class="ops-flag-sep">,</span>{/if}
                          {/each}
                        </span>
                      </li>
                    {/if}
                  {/each}
                </ul>
              {:else}
                <p class="ops-empty-inline">None.</p>
              {/if}
            </div>

            <div class="ops-trust-col">
              <h3 class="ops-section-label">
                Stale
                {#if trustData.stale.length > 0}
                  <span class="ops-count ops-count--warning">{trustData.stale.length}</span>
                {/if}
              </h3>
              {#if trustData.stale.length > 0}
                <ul class="ops-flag-list">
                  {#each trustData.stale as flag, i (`s-${i}`)}
                    {#if flag.kind === 'stale'}
                      <li class="ops-flag ops-flag--warning">
                        <span class="ops-flag-days">{flag.days_since_modified}d</span>
                        <code class="ops-mono">{flag.claim_id}</code>
                      </li>
                    {/if}
                  {/each}
                </ul>
              {:else}
                <p class="ops-empty-inline">None.</p>
              {/if}
            </div>
          </div>

          {#if trustData.retrieval_trace}
            <div class="ops-trust-trace">
              <h3 class="ops-section-label">Retrieval trace</h3>
              <p class="ops-trace-reason">{trustData.retrieval_trace.reason}</p>
              <p class="ops-meta">
                Included: <span class="ops-mono">{trustData.retrieval_trace.included_claim_ids.length}</span>
                · Excluded: <span class="ops-mono">{trustData.retrieval_trace.excluded_claim_ids.length}</span>
              </p>
            </div>
          {/if}
        {/if}
      </StateBox>
    </section>

    <!-- ── Jobs (inline metrics — no hero-metric template) ─────────────── -->
    <section class="ops-panel" aria-label="Job queue" aria-live="polite" aria-busy={jobsLoading}>
      <header class="ops-panel-head">
        <h2>Jobs</h2>
        <button
          type="button"
          class="ops-icon-btn"
          onclick={() => void refreshJobs()}
          disabled={jobsLoading}
          aria-label="Refresh jobs"
        >
          {#if jobsLoading}
            <span aria-hidden="true">…</span>
          {:else}
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          {/if}
        </button>
      </header>
      <StateBox
        loading={jobsLoading}
        error={jobsError}
        empty={jobs === null}
        emptyText="No job summary available."
      >
        {#if jobs}
          <div class="ops-stat-row" role="group" aria-label="Job counts">
            <span class="ops-stat">
              <span class="ops-stat-value">{jobs.active}</span>
              <span class="ops-stat-label">active</span>
            </span>
            <span class="ops-stat-sep" aria-hidden="true">·</span>
            <span class="ops-stat">
              <span class="ops-stat-value">{jobs.queued}</span>
              <span class="ops-stat-label">queued</span>
            </span>
            <span class="ops-stat-sep" aria-hidden="true">·</span>
            <span class="ops-stat" class:ops-stat--danger={jobs.failed > 0}>
              <span class="ops-stat-value">{jobs.failed}</span>
              <span class="ops-stat-label">failed</span>
            </span>
          </div>
        {/if}
      </StateBox>
    </section>

    <!-- ── Clients (wide — table needs room) ──────────────────────────── -->
    <section class="ops-panel ops-panel--wide" aria-label="Registered clients" aria-live="polite" aria-busy={clientsLoading}>
      <header class="ops-panel-head">
        <h2>Clients</h2>
        <button
          type="button"
          class="ops-icon-btn"
          onclick={() => void refreshClients()}
          disabled={clientsLoading}
          aria-label="Refresh clients"
        >
          {#if clientsLoading}
            <span aria-hidden="true">…</span>
          {:else}
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          {/if}
        </button>
      </header>
      <StateBox
        loading={clientsLoading}
        error={clientsError}
        empty={clients.length === 0}
        emptyText="No registered clients."
      >
        <div class="ops-table-scroll">
          <table class="ops-clients-table">
            <thead>
              <tr>
                <th>Label</th>
                <th>Client ID</th>
                <th>Capabilities</th>
                <th>Last active</th>
                <th>Mutations</th>
              </tr>
            </thead>
            <tbody>
              {#each clients as c (c.client_id)}
                <tr>
                  <td>{c.label}</td>
                  <td><code class="ops-mono">{c.client_id}</code></td>
                  <td>{c.capabilities.join(', ')}</td>
                  <td>{formatDate(c.last_active_at)}</td>
                  <td><span class="ops-mono">{c.mutation_count}</span></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </StateBox>
    </section>

    <!-- ── Evals ──────────────────────────────────────────────────────── -->
    <section class="ops-panel" aria-label="Domain evals">
      <header class="ops-panel-head">
        <h2>Evals</h2>
      </header>
      <form class="ops-evals-form" onsubmit={refreshEvals}>
        <label for="evals-domain" class="ops-field-label">Domain</label>
        <input
          id="evals-domain"
          class="ops-input"
          type="text"
          placeholder="e.g. stocks"
          bind:value={evalsDomain}
          disabled={evalsLoading}
        />
        <button type="submit" class="ops-btn ops-btn--ghost" disabled={evalsLoading}>
          {evalsLoading ? 'Loading…' : 'Load'}
        </button>
      </form>
      <StateBox
        loading={evalsLoading}
        error={evalsError}
        empty={evals === null && !evalsLoading && !evalsError}
        emptyText="Enter a domain to load the eval summary."
      >
        {#if evals}
          <dl class="ops-fields">
            <div><dt>Cases</dt><dd><span class="ops-mono">{evals.case_count}</span></dd></div>
            <div><dt>Passed</dt><dd><span class="ops-mono">{evals.passed}</span></dd></div>
            <div>
              <dt>Abstention invariant</dt>
              <dd>{evals.abstention_passed ? 'passed' : 'failed'}</dd>
            </div>
            <div><dt>Run at</dt><dd>{formatDate(evals.run_at)}</dd></div>
          </dl>
        {/if}
      </StateBox>
    </section>

    <!-- ── Backup health (wide — pairs with Evals on desktop) ─────────── -->
    <section class="ops-panel ops-panel--wide" aria-label="Backup health" aria-live="polite" aria-busy={backupLoading}>
      <header class="ops-panel-head">
        <h2>Backup health</h2>
        <button
          type="button"
          class="ops-icon-btn"
          onclick={() => void refreshBackup()}
          disabled={backupLoading}
          aria-label="Refresh backup health"
        >
          {#if backupLoading}
            <span aria-hidden="true">…</span>
          {:else}
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          {/if}
        </button>
      </header>
      <StateBox
        loading={backupLoading}
        error={backupError}
        empty={backup === null}
        emptyText="No backup health available."
      >
        {#if backup}
          <dl class="ops-fields">
            <div><dt>Last backup</dt><dd>{formatDate(backup.last_backup_at)}</dd></div>
            <div>
              <dt>Restore drill</dt>
              <dd>
                {#if backup.last_restore_drill_ok}
                  passed
                {:else}
                  not yet drilled — Phase F
                {/if}
              </dd>
            </div>
          </dl>
          {#if !backup.last_restore_drill_ok}
            <p class="ops-callout ops-callout--warning" role="note">
              The restore-drill harness ships in Phase F. The current value is
              honestly false — no drill has run yet.
            </p>
          {/if}
        {/if}
      </StateBox>
    </section>
  </div>

  <!-- ── Hard purge (destructive — isolated, full visual separation) ──── -->
  <section class="ops-destructive" aria-label="Hard purge (destructive)">
    <header class="ops-destructive-head">
      <span class="ops-destructive-icon" aria-hidden="true">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </span>
      <div class="ops-destructive-title">
        <h2>Hard purge</h2>
        <p class="ops-destructive-sub">Destroys content keys irreversibly.</p>
      </div>
    </header>
    <p class="ops-destructive-flow">
      Preview the targets, read the full warning, type the server nonce verbatim,
      re-authenticate, then confirm. Every gate must pass before the execute fires.
    </p>
    <div class="ops-destructive-form">
      <label for="purge-input" class="ops-field-label">Object ids (one per line)</label>
      <textarea
        id="purge-input"
        class="ops-textarea"
        rows="4"
        placeholder={'sha256:...\nsha256:...'}
        bind:value={purgeInput}
        disabled={purgePreviewing}
      ></textarea>
      <div class="ops-destructive-actions">
        <button
          type="button"
          class="ops-btn ops-btn--danger"
          onclick={() => void startPurgePreview()}
          disabled={purgePreviewing}
        >
          {purgePreviewing ? 'Loading…' : 'Preview purge'}
        </button>
      </div>
    </div>
  </section>
</section>

{#if purgeDialogOpen && purgeWarning && purgePreview}
  <DestructiveDialog
    warning={purgeWarning}
    preview={purgePreviewItems}
    nonce={purgePreview.nonce}
    requiresReauth={purgeWarning.requires_recent_reauth}
    onConfirm={onPurgeExecute}
    onCancel={cancelPurgePreview}
    acting={purgeExecuting}
  />
{/if}

<style>
  /* ── Page shell ─────────────────────────────────────────────────────── */
  .ops-page {
    padding: var(--space-lg) 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .ops-page-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .ops-page-head h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-headline);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--text-headline-tracking);
    line-height: var(--text-headline-leading);
  }

  .ops-tagline {
    margin: var(--space-xs) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-body);
    max-width: var(--content-measure);
  }

  /* ── Shared button vocabulary (token-driven, ≥44px touch targets) ───── */
  .ops-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-xs);
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }

  .ops-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ops-btn--ghost:hover:not(:disabled) {
    background: var(--overlay-ink-06);
    border-color: var(--color-ink-faint);
  }

  .ops-btn--danger {
    background: var(--overlay-danger-strong);
    border-color: var(--color-danger);
    color: var(--text-primary);
    font-weight: var(--weight-semibold);
  }

  .ops-btn--danger:hover:not(:disabled) {
    background: var(--color-danger);
  }

  .ops-refresh-all {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: var(--overlay-ink-06);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .ops-refresh-all:hover {
    background: var(--overlay-ink-10);
  }

  .ops-refresh-all svg {
    flex-shrink: 0;
  }

  /* Icon button — square ≥44×44, visually compact via icon */
  .ops-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px;
    padding: var(--space-xs);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .ops-icon-btn:hover:not(:disabled) {
    background: var(--overlay-ink-06);
    color: var(--text-primary);
  }

  .ops-icon-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── Panel grid (breaks identical-card-grid anti-pattern via tiers) ── */
  .ops-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-md);
  }

  @media (min-width: 48rem) {
    .ops-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (min-width: 64rem) {
    .ops-grid {
      grid-template-columns: repeat(3, 1fr);
    }
    /* Trust (primary) + Clients (table) span 2 cols — breaks the grid's
       uniformity. The remaining panels (Jobs, Evals, Backup) are 1 col. */
    .ops-panel--primary,
    .ops-panel--wide {
      grid-column: span 2;
    }
  }

  /* ── Panel base (Flat-By-Default: no shadow at rest) ───────────────── */
  .ops-panel {
    padding: var(--space-md);
    border: var(--border-hairline);
    border-radius: var(--radius-lg);
    background: var(--surface-flat);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  /* Primary tier — Trust gets a hair more visual weight (surface-elevated)
     without crossing into "different card chassis" territory. */
  .ops-panel--primary {
    background: var(--surface-raised);
  }

  .ops-panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-sm);
  }

  .ops-panel-head h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    letter-spacing: 0;
    line-height: var(--text-title-leading);
  }

  /* Section labels — sentence case (kills the tracked-uppercase eyebrow).
     A count badge carries emphasis when there's something to act on. */
  .ops-section-label {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin: 0 0 var(--space-xs);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    letter-spacing: 0;
  }

  .ops-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.5rem;
    height: 1.25rem;
    padding: 0 var(--space-xs);
    border-radius: var(--radius-pill);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    font-weight: var(--weight-medium);
    line-height: 1;
  }

  .ops-count--danger {
    background: var(--overlay-danger-strong);
    color: var(--text-primary);
  }

  .ops-count--warning {
    background: var(--overlay-accent-soft);
    color: var(--color-accent);
  }

  /* ── Trust internal layout (2-col on desktop) ──────────────────────── */
  .ops-trust-body {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-md);
  }

  @media (min-width: 40rem) {
    .ops-trust-body {
      grid-template-columns: 1fr 1fr;
    }
  }

  .ops-trust-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .ops-trust-trace {
    margin-top: var(--space-sm);
    padding-top: var(--space-sm);
    border-top: 1px solid var(--color-hairline);
  }

  .ops-trace-reason {
    margin: 0 0 var(--space-xs);
    color: var(--text-primary);
    font-size: var(--text-body);
    word-break: break-word;
  }

  .ops-meta {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-mono);
  }

  /* ── Flag list (contradictions + stale) ────────────────────────────── */
  .ops-flag-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .ops-flag {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    align-items: center;
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
    background: var(--overlay-ink-04);
    font-size: var(--text-body);
  }

  .ops-flag--danger {
    background: var(--overlay-danger-soft);
  }

  .ops-flag--warning {
    background: var(--overlay-accent-soft);
  }

  .ops-flag-ids {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    align-items: baseline;
  }

  .ops-flag-sep {
    color: var(--text-tertiary);
  }

  .ops-flag-days {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--color-accent);
    font-weight: var(--weight-medium);
  }

  /* Body-size tertiary text fails AA at 15px. Use secondary (4.83:1) —
   * the "None." voice is already quiet via the word itself. */
  .ops-empty-inline {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-body);
  }

  /* ── Inline stat row (kills hero-metric template) ────────────────────
     Machine counts are mono (Mono-Marks-Machine Rule); labels are
     sentence-case Inter, sitting inline — not stacked big-over-small. */
  .ops-stat-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-sm);
    padding: var(--space-sm) 0;
  }

  .ops-stat {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-xs);
  }

  .ops-stat-value {
    font-family: var(--font-mono);
    font-size: 1.25rem;
    font-weight: var(--weight-medium);
    color: var(--text-primary);
    line-height: 1;
  }

  .ops-stat-label {
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-secondary);
  }

  .ops-stat--danger .ops-stat-value {
    color: var(--color-danger);
  }

  .ops-stat-sep {
    color: var(--text-tertiary);
    font-size: var(--text-body);
  }

  /* ── Clients table (horizontal scroll on mobile) ───────────────────── */
  .ops-table-scroll {
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
  }

  .ops-clients-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-body);
  }

  .ops-clients-table th,
  .ops-clients-table td {
    text-align: left;
    padding: var(--space-sm);
    border-bottom: 1px solid var(--color-hairline);
    word-break: break-word;
    vertical-align: top;
  }

  .ops-clients-table th {
    /* Sentence case (kills tracked-uppercase eyebrow). Label weight only. */
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    letter-spacing: 0;
    text-transform: none;
  }

  .ops-clients-table tbody tr:last-child td {
    border-bottom: none;
  }

  /* ── Evals form (stacks on mobile, row on tablet+) ─────────────────── */
  .ops-evals-form {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-sm);
    margin-bottom: var(--space-sm);
  }

  @media (min-width: 30rem) {
    .ops-evals-form {
      grid-template-columns: max-content 1fr auto;
      align-items: center;
    }
  }

  .ops-field-label {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    letter-spacing: 0;
  }

  .ops-input {
    width: 100%;
    min-height: 44px;
    padding: var(--space-sm) var(--space-sm);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    transition: border-color var(--duration-fast) var(--ease-out-quart);
  }

  .ops-input::placeholder {
    color: var(--text-tertiary);
  }

  .ops-input:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .ops-input:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  /* ── Fields (dl — Evals + Backup) ──────────────────────────────────── */
  .ops-fields {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: var(--space-xs) var(--space-md);
    font-size: var(--text-body);
  }

  .ops-fields div {
    display: flex;
    gap: var(--space-sm);
    align-items: baseline;
  }

  .ops-fields dt {
    color: var(--text-secondary);
    font-size: var(--text-label);
    min-width: 7rem;
  }

  .ops-fields dd {
    margin: 0;
    color: var(--text-primary);
  }

  /* ── Callout (info/warning — full border, never a side-stripe) ─────── */
  .ops-callout {
    margin: var(--space-sm) 0 0;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    font-size: var(--text-body);
    color: var(--text-secondary);
  }

  .ops-callout--warning {
    border-color: var(--color-accent);
    background: var(--overlay-accent-soft);
    color: var(--text-primary);
  }

  /* ── Hard purge (destructive — isolated, distinct visual tier) ───────
     Full danger border (never a side-stripe). Separated from the benign
     panels by spacing and by a full-width chassis change. */
  .ops-destructive {
    margin-top: var(--space-md);
    padding: var(--space-lg);
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-lg);
    background: var(--overlay-danger-soft);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .ops-destructive-head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-md);
  }

  .ops-destructive-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    background: var(--overlay-danger-strong);
    color: var(--text-primary);
  }

  .ops-destructive-title h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    letter-spacing: 0;
  }

  .ops-destructive-sub {
    margin: var(--space-xs) 0 0;
    color: var(--color-danger);
    font-size: var(--text-body);
    font-weight: var(--weight-medium);
  }

  .ops-destructive-flow {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-body);
    max-width: var(--content-measure);
    line-height: var(--text-body-leading);
  }

  .ops-destructive-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .ops-textarea {
    width: 100%;
    min-height: 88px;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    line-height: var(--text-mono-leading);
    resize: vertical;
    transition: border-color var(--duration-fast) var(--ease-out-quart);
  }

  .ops-textarea::placeholder {
    color: var(--text-tertiary);
  }

  .ops-textarea:focus {
    outline: none;
    border-color: var(--color-danger);
  }

  .ops-textarea:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .ops-destructive-actions {
    display: flex;
    gap: var(--space-sm);
  }

  /* ── Mono inline (machine output — Mono-Marks-Machine Rule) ────────── */
  .ops-mono {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    word-break: break-all;
  }
</style>
