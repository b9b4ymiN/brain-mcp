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

<section class="page page-operations">
  <h1>Operations</h1>
  <p class="tagline">
    Trust + clients + jobs + evals + backup health. Each card is independently
    refreshed.
  </p>

  <div class="toolbar">
    <button type="button" onclick={() => void refreshAll()}>
      Refresh all
    </button>
  </div>

  <div class="grid">
    <!-- ── Trust ───────────────────────────────────────────────────────── -->
    <section class="card card-trust" aria-label="Trust surface">
      <header class="card-head">
        <h2>Trust</h2>
        <button
          type="button"
          class="mini"
          onclick={() => void refreshTrust()}
          disabled={trustLoading}
          aria-label="Refresh trust"
        >
          {trustLoading ? '…' : 'Refresh'}
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
          {#if trustData.contradictions.length > 0}
            <h3 class="subhead">Contradictions ({trustData.contradictions.length})</h3>
            <ul class="flag-list">
              {#each trustData.contradictions as flag, i (`c-${i}`)}
                {#if flag.kind === 'contradiction'}
                  <li class="flag flag-contradiction">
                    <span class="badge">contradiction</span>
                    <span class="flag-ids">
                      {#each flag.claim_ids as id, j (`c-${i}-${j}`)}
                        <span class="mono">{id}</span>{#if j < flag.claim_ids.length - 1}<span>, </span>{/if}
                      {/each}
                    </span>
                  </li>
                {/if}
              {/each}
            </ul>
          {/if}
          {#if trustData.stale.length > 0}
            <h3 class="subhead">Stale ({trustData.stale.length})</h3>
            <ul class="flag-list">
              {#each trustData.stale as flag, i (`s-${i}`)}
                {#if flag.kind === 'stale'}
                  <li class="flag flag-stale">
                    <span class="badge">stale · {flag.days_since_modified}d</span>
                    <span class="mono">{flag.claim_id}</span>
                  </li>
                {/if}
              {/each}
            </ul>
          {/if}
          {#if trustData.retrieval_trace}
            <h3 class="subhead">Retrieval trace</h3>
            <p class="trace-reason">{trustData.retrieval_trace.reason}</p>
            <p class="trace-meta">
              Included: {trustData.retrieval_trace.included_claim_ids.length}
              · Excluded: {trustData.retrieval_trace.excluded_claim_ids.length}
            </p>
          {/if}
        {/if}
      </StateBox>
    </section>

    <!-- ── Clients ─────────────────────────────────────────────────────── -->
    <section class="card card-clients" aria-label="Registered clients">
      <header class="card-head">
        <h2>Clients</h2>
        <button
          type="button"
          class="mini"
          onclick={() => void refreshClients()}
          disabled={clientsLoading}
          aria-label="Refresh clients"
        >
          {clientsLoading ? '…' : 'Refresh'}
        </button>
      </header>
      <StateBox
        loading={clientsLoading}
        error={clientsError}
        empty={clients.length === 0}
        emptyText="No registered clients."
      >
        <table class="clients-table">
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
                <td class="mono">{c.client_id}</td>
                <td>{c.capabilities.join(', ')}</td>
                <td>{formatDate(c.last_active_at)}</td>
                <td>{c.mutation_count}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </StateBox>
    </section>

    <!-- ── Jobs ────────────────────────────────────────────────────────── -->
    <section class="card card-jobs" aria-label="Job queue">
      <header class="card-head">
        <h2>Jobs</h2>
        <button
          type="button"
          class="mini"
          onclick={() => void refreshJobs()}
          disabled={jobsLoading}
          aria-label="Refresh jobs"
        >
          {jobsLoading ? '…' : 'Refresh'}
        </button>
      </header>
      <StateBox
        loading={jobsLoading}
        error={jobsError}
        empty={jobs === null}
        emptyText="No job summary available."
      >
        {#if jobs}
          <div class="big-numbers" role="group" aria-label="Job counts">
            <div class="big-number">
              <span class="big-number-value">{jobs.active}</span>
              <span class="big-number-label">Active</span>
            </div>
            <div class="big-number">
              <span class="big-number-value">{jobs.queued}</span>
              <span class="big-number-label">Queued</span>
            </div>
            <div class="big-number">
              <span class="big-number-value">{jobs.failed}</span>
              <span class="big-number-label">Failed</span>
            </div>
          </div>
        {/if}
      </StateBox>
    </section>

    <!-- ── Evals ───────────────────────────────────────────────────────── -->
    <section class="card card-evals" aria-label="Domain evals">
      <header class="card-head">
        <h2>Evals</h2>
      </header>
      <form class="evals-form" onsubmit={refreshEvals}>
        <label for="evals-domain">Domain</label>
        <input
          id="evals-domain"
          type="text"
          placeholder="e.g. stocks"
          bind:value={evalsDomain}
          disabled={evalsLoading}
        />
        <button type="submit" disabled={evalsLoading}>
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
          <dl class="evals-fields">
            <div><dt>Cases</dt><dd>{evals.case_count}</dd></div>
            <div><dt>Passed</dt><dd>{evals.passed}</dd></div>
            <div>
              <dt>Abstention invariant</dt>
              <dd>{evals.abstention_passed ? 'passed' : 'failed'}</dd>
            </div>
            <div><dt>Run at</dt><dd>{formatDate(evals.run_at)}</dd></div>
          </dl>
        {/if}
      </StateBox>
    </section>

    <!-- ── Backup health ──────────────────────────────────────────────── -->
    <section class="card card-backup" aria-label="Backup health">
      <header class="card-head">
        <h2>Backup health</h2>
        <button
          type="button"
          class="mini"
          onclick={() => void refreshBackup()}
          disabled={backupLoading}
          aria-label="Refresh backup health"
        >
          {backupLoading ? '…' : 'Refresh'}
        </button>
      </header>
      <StateBox
        loading={backupLoading}
        error={backupError}
        empty={backup === null}
        emptyText="No backup health available."
      >
        {#if backup}
          <dl class="backup-fields">
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
            <p class="callout" role="note">
              The restore-drill harness ships in Phase F. The current value is
              honestly false — no drill has run yet.
            </p>
          {/if}
        {/if}
      </StateBox>
    </section>

    <!-- ── Hard purge ─────────────────────────────────────────────────── -->
    <section
      class="card card-purge"
      aria-label="Hard purge (destructive)"
    >
      <header class="card-head">
        <h2>Hard purge</h2>
      </header>
      <p class="warning-callout" role="note">
        Hard purge destroys content keys irreversibly. The flow is: preview →
        read warning + type the server nonce + re-authenticate → confirm.
      </p>
      <label for="purge-input">Object ids (one per line)</label>
      <textarea
        id="purge-input"
        rows="4"
        placeholder={'sha256:...\nsha256:...'}
        bind:value={purgeInput}
        disabled={purgePreviewing}
      ></textarea>
      <div class="row">
        <button
          type="button"
          onclick={() => void startPurgePreview()}
          disabled={purgePreviewing}
        >
          {purgePreviewing ? 'Loading…' : 'Preview'}
        </button>
      </div>
    </section>
  </div>
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
  .page {
    padding: 1.5rem 0;
  }

  .tagline {
    margin: 0.25rem 0 1rem;
    opacity: 0.75;
  }

  .toolbar {
    margin: 0 0 1rem;
  }

  .toolbar button,
  .mini {
    padding: 0.35rem 0.7rem;
    border-radius: 0.3rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
    font-size: 0.82rem;
  }

  .toolbar button:hover:not(:disabled),
  .mini:hover:not(:disabled),
  .toolbar button:focus-visible:not(:disabled),
  .mini:focus-visible:not(:disabled) {
    background: rgba(127, 127, 127, 0.25);
  }

  .mini:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: 0.85rem;
  }

  .card {
    padding: 0.85rem 1rem;
    border-radius: 0.5rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.04);
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .card-purge {
    border-color: rgba(190, 70, 70, 0.55);
    background: rgba(190, 70, 70, 0.05);
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .card-head h2 {
    margin: 0;
    font-size: 1rem;
  }

  .subhead {
    margin: 0.5rem 0 0.25rem;
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.7;
  }

  .flag-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }

  .flag {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
    padding: 0.3rem 0.45rem;
    border-radius: 0.3rem;
    background: rgba(127, 127, 127, 0.06);
    font-size: 0.85rem;
  }

  .badge {
    padding: 0.15rem 0.4rem;
    border-radius: 0.25rem;
    background: rgba(127, 127, 127, 0.25);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }

  .flag-contradiction {
    background: rgba(190, 70, 70, 0.12);
  }

  .flag-contradiction .badge {
    background: rgba(190, 70, 70, 0.4);
  }

  .flag-stale {
    background: rgba(190, 130, 70, 0.1);
  }

  .flag-stale .badge {
    background: rgba(190, 130, 70, 0.35);
  }

  .flag-ids {
    display: flex;
    flex-wrap: wrap;
    gap: 0.15rem;
  }

  .trace-reason {
    margin: 0.25rem 0 0.15rem;
    font-size: 0.85rem;
    word-break: break-word;
  }

  .trace-meta {
    margin: 0;
    font-size: 0.8rem;
    opacity: 0.7;
  }

  .clients-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }

  .clients-table th,
  .clients-table td {
    text-align: left;
    padding: 0.4rem 0.5rem;
    border-bottom: 1px solid rgba(127, 127, 127, 0.25);
    word-break: break-word;
    vertical-align: top;
  }

  .clients-table th {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.65;
  }

  .big-numbers {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.5rem;
  }

  .big-number {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 0.6rem 0.4rem;
    border-radius: 0.4rem;
    background: rgba(127, 127, 127, 0.08);
  }

  .big-number-value {
    font-size: 1.75rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .big-number-label {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.7;
  }

  .evals-form {
    display: grid;
    grid-template-columns: max-content 1fr auto;
    gap: 0.4rem;
    align-items: center;
    margin-bottom: 0.5rem;
  }

  .evals-form input,
  .evals-form button {
    padding: 0.4rem 0.55rem;
    border-radius: 0.3rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .evals-form button {
    background: rgba(127, 127, 127, 0.15);
    cursor: pointer;
  }

  .evals-form button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .evals-fields,
  .backup-fields {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.25rem 1rem;
    font-size: 0.85rem;
  }

  .evals-fields div,
  .backup-fields div {
    display: flex;
    gap: 0.4rem;
  }

  .evals-fields dt,
  .backup-fields dt {
    opacity: 0.6;
    font-size: 0.78rem;
    min-width: 7rem;
  }

  .evals-fields dd,
  .backup-fields dd {
    margin: 0;
  }

  .callout {
    margin: 0.5rem 0 0;
    padding: 0.4rem 0.6rem;
    border-radius: 0.3rem;
    border: 1px solid rgba(190, 130, 70, 0.5);
    background: rgba(190, 130, 70, 0.12);
    font-size: 0.82rem;
  }

  .warning-callout {
    margin: 0 0 0.5rem;
    padding: 0.5rem 0.7rem;
    border-radius: 0.35rem;
    border: 1px solid rgba(190, 70, 70, 0.5);
    background: rgba(190, 70, 70, 0.12);
    font-size: 0.85rem;
  }

  label {
    font-size: 0.8rem;
    opacity: 0.85;
  }

  textarea {
    width: 100%;
    padding: 0.45rem 0.55rem;
    border-radius: 0.35rem;
    border: 1px solid rgba(127, 127, 127, 0.5);
    background: inherit;
    color: inherit;
    font: inherit;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    resize: vertical;
  }

  .row {
    display: flex;
    gap: 0.4rem;
    margin-top: 0.4rem;
  }

  .row button {
    padding: 0.45rem 0.8rem;
    border-radius: 0.35rem;
    border: 1px solid rgba(190, 70, 70, 0.55);
    background: rgba(190, 70, 70, 0.18);
    color: inherit;
    font: inherit;
    cursor: pointer;
    font-weight: 600;
  }

  .row button:hover:not(:disabled),
  .row button:focus-visible:not(:disabled) {
    background: rgba(190, 70, 70, 0.28);
  }

  .row button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.82em;
  }
</style>
