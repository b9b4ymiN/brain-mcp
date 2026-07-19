<script lang="ts">
  /**
   * Entity — subject-centric view.
   *
   *   - Subject picker at top → calls `api.getSubject({subject})` and renders
   *     the returned claims as a table.
   *   - Each row expands inline to show the timeline for that
   *     subject+predicate (calls `api.timeline({domain, subject, predicate})`,
   *     domain borrowed from the row's own claim).
   *   - On mount, consumes a pending subject staged by Home/Search for the
   *     "click → land on loaded entity" handoff.
   *
   * Task E3.3 (Part D + E): the entity page now hosts the entity-level
   * destructive UI (`<EntityDestructivePanel>`) and the four-questions
   * provenance synthesis (`<ProvenancePanel>`). Both render only when an
   * `entity_id` is present on the loaded claims (otherwise there is no
   * entity to merge/split/answer about).
   *
   * 4-state coverage via `<StateBox>`:
   *   - loading    → `loading=true` while `getSubject()` is in flight.
   *   - error      → any non-401 thrown; rendered as `state-error`.
   *   - empty      → subject has zero current claims.
   *   - permission → 401 → `session.clear()` + focused banner; App.svelte
   *                  re-renders the login form.
   *
   * Per-row timeline expansion is its OWN micro state machine: a row may be
   * `idle | loading | error | open(timeline[])`. We keep these in a map keyed
   * by claim_id so multiple rows can expand independently. The
   * `ProvenancePanel` consumes a derived `Record<predicate, ClaimView[]>` so
   * it reuses the timeline data we already fetched for row expansion.
   */
  import { onMount } from 'svelte'
  import {
    getSubject,
    timeline,
    opsClients,
    ApiError,
    type SubjectClaim,
    type ClaimView,
    type GalaxyNode,
    type ClientActivity,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import { consumePendingSubject, setPendingSubject } from '../lib/quickSearch'
  import StateBox from '../components/StateBox.svelte'
  import GalaxyGraph from '../components/GalaxyGraph.svelte'
  import EntityDestructivePanel from '../components/EntityDestructivePanel.svelte'
  import ProvenancePanel from '../components/ProvenancePanel.svelte'
  import { formatValue, formatDate } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // Picker form.
  let subjectInput = $state('')
  let activeSubject = $state('')
  let submitting = $state(false)

  // View mode toggle — Galaxy is a SUB-VIEW of Entity (the 5-page
  // ConsolePage Rust enum is locked at 5; Galaxy is never a 6th page).
  // The plan resolution (see module-level task spec) is: same Entity page,
  // swap the claims table for an embedded Galaxy graph focused on this
  // entity (zoom=close, focus=current entity_id) when the user toggles.
  let viewMode = $state<'table' | 'galaxy'>('table')

  // Claims table state.
  let loading = $state(false)
  let error = $state<string | null>(null)
  let claims = $state<SubjectClaim[]>([])
  let hasLoaded = $state(false)
  let sessionExpired = $state(false)

  // Monotonic request-id guard for `viewSubject`: if the user submits a
  // second subject while the first `getSubject()` is still in flight, the
  // stale response is discarded rather than overwriting the newer state.
  let viewSeq = 0

  // Per-row timeline expansion state.
  type RowState =
    | { kind: 'idle' }
    | { kind: 'loading' }
    | { kind: 'error'; message: string }
    | { kind: 'open'; entries: ClaimView[] }

  let expanded = $state<Record<string, RowState>>({})

  // ── Task E3.3: /ops/clients for the provenance panel ─────────────────────
  // Fetched once per subject view (best-effort: failure leaves the panel's
  // "Client that edited" facet empty rather than blocking the entity view).
  let clients = $state<ClientActivity[]>([])
  let clientsError = $state<string | null>(null)
  let clientsSeq = 0

  onMount(() => {
    const pending = consumePendingSubject()
    if (pending) {
      subjectInput = pending
      void viewSubject(pending)
    }
  })

  async function viewSubject(subject: string): Promise<void> {
    const trimmed = subject.trim()
    if (!trimmed) return
    // Sequence-stamp this request so a later submit can invalidate us.
    const seq = ++viewSeq
    activeSubject = trimmed
    submitting = true
    loading = true
    error = null
    sessionExpired = false
    hasLoaded = true
    expanded = {}
    try {
      const response = await getSubject({ subject: trimmed })
      // Discard stale response — a newer subject request supersedes us.
      if (seq !== viewSeq) return
      claims = response.claims
      // E3.3 Part E: best-effort client fetch for the provenance panel. Fire
      // in parallel — failure here must not block the entity view.
      void refreshClients()
    } catch (cause) {
      if (seq !== viewSeq) return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      claims = []
      error =
        cause instanceof ApiError
          ? `Failed to load subject (${cause.code}).`
          : 'Failed to load subject — is the backend running on :8080?'
    } finally {
      // Only the most-recent request is allowed to clear loading state —
      // otherwise a stale completion would un-stick a newer in-flight load.
      if (seq === viewSeq) {
        loading = false
        submitting = false
      }
    }
  }

  async function refreshClients(): Promise<void> {
    const seq = ++clientsSeq
    clientsError = null
    try {
      const result = await opsClients()
      if (seq !== clientsSeq) return
      clients = result
    } catch (cause) {
      if (seq !== clientsSeq) return
      // Non-fatal: provenance panel degrades gracefully.
      clients = []
      clientsError =
        cause instanceof ApiError
          ? `Client activity unavailable (${cause.code}).`
          : null
    }
  }

  function onSubmit(event: SubmitEvent): void {
    event.preventDefault()
    void viewSubject(subjectInput)
  }

  async function toggleRow(claim: SubjectClaim): Promise<void> {
    const id = claim.claim_id
    const current = expanded[id]
    // Already open or in flight → collapse: DELETE the key entirely (was
    // previously set to {kind:'idle'} which kept aria-expanded=true and
    // mounted an empty phantom expand-row). Deleting means the {#if
    // rowState} block unmounts the expand-row, and aria-expanded resolves
    // to false (rowState is undefined).
    if (current && current.kind !== 'idle') {
      const next = { ...expanded }
      delete next[id]
      expanded = next
      return
    }
    expanded = { ...expanded, [id]: { kind: 'loading' } }
    try {
      const entries = await timeline({
        domain: claim.domain,
        subject: claim.subject,
        predicate: claim.predicate,
      })
      // Async-toggle race guard: the user may have collapsed the row
      // (deleted the key) while the fetch was in flight. If `loading`
      // is no longer the current state for this id, do NOT overwrite —
      // their collapse intent wins. This also covers the case of a second toggle that
      // re-opened the row (it would be `loading` again — different in-flight
      // request owns that slot, not us).
      if (expanded[id]?.kind !== 'loading') return
      expanded = { ...expanded, [id]: { kind: 'open', entries } }
    } catch (cause) {
      // Same race guard in the catch branch: a mid-flight collapse must not
      // be clobbered by a late-arriving error banner.
      if (expanded[id]?.kind !== 'loading') return
      const message =
        cause instanceof ApiError
          ? `Timeline failed (${cause.code}).`
          : 'Timeline failed — is the backend running on :8080?'
      expanded = { ...expanded, [id]: { kind: 'error', message } }
    }
  }

  function onRowKeydown(event: KeyboardEvent, claim: SubjectClaim): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      void toggleRow(claim)
    }
  }

  function confidencePct(c: number): string {
    // SubjectClaim.confidence is 0..1 (server divides basis-points / 10_000).
    // Clamp for display; never negative, never >100.
    const pct = Math.round(Math.max(0, Math.min(1, c)) * 100)
    return `${pct}%`
  }

  /**
   * Galaxy node → Entity navigation. The current Entity page only knows the
   * active SUBJECT (string), not the entity_id UUID. We mirror the Home/Search
   * handoff pattern: stage the clicked node's label as the pending subject
   * (the galaxy `label` is the claim subject), then re-enter Entity. When
   * GalaxyGraph is embedded with `focus=entity_id`, the side-panel "Open as
   * entity" button is the click source; we flip back to table view + stage
   * the subject so the table loads for the clicked entity.
   */
  function onGalaxyNodeClick(node: GalaxyNode): void {
    setPendingSubject(node.label)
    viewMode = 'table'
    // Force a fresh load of the new subject. If the user is ALREADY on this
    // subject (same subject, just clicked a self-node), the early-return in
    // viewSubject would skip the load — that's fine; the existing table is
    // still correct.
    void viewSubject(node.label)
  }

  // E3.3 Part D: derive the current entity_id (if any) so we can mount the
  // destructive panel + provenance panel only when there is a real entity.
  let activeEntityId = $derived(
    claims.find((c) => c.entity_id !== null)?.entity_id ?? null,
  )

  // E3.3 Part E: assemble the predicate → ClaimView[] map for the provenance
  // panel. Built from the row-expansion `expanded` map so we reuse timeline
  // data the user has already fetched; predicates the user hasn't expanded
  // contribute an empty entry (panel shows "no time bounds").
  let timelineByPredicate = $derived.by<Record<string, ClaimView[]>>(() => {
    const out: Record<string, ClaimView[]> = {}
    for (const claim of claims) {
      const row = expanded[claim.claim_id]
      out[claim.predicate] = row?.kind === 'open' ? row.entries : []
    }
    return out
  })

  // After a destructive mutation (merge/split/retract), refetch the subject
  // + clients so the table + provenance reflect the new state.
  function onEntityMutated(): void {
    if (activeSubject) void viewSubject(activeSubject)
  }
</script>

<section class="page page-entity">
  <h1>Entity</h1>

  <form class="subject-form" onsubmit={onSubmit}>
    <label for="subject">Subject</label>
    <input
      id="subject"
      type="text"
      required
      placeholder="e.g. some-entity-id"
      bind:value={subjectInput}
    />
    <button type="submit" disabled={submitting}>
      {submitting ? 'Loading…' : 'View'}
    </button>
  </form>

  {#if hasLoaded}
    <div class="view-toggle" role="group" aria-label="Entity view mode">
      <button
        type="button"
        class:active={viewMode === 'table'}
        aria-pressed={viewMode === 'table'}
        onclick={() => (viewMode = 'table')}
      >
        Claims table
      </button>
      <button
        type="button"
        class:active={viewMode === 'galaxy'}
        aria-pressed={viewMode === 'galaxy'}
        onclick={() => (viewMode = 'galaxy')}
        disabled={claims.length === 0}
        title={claims.length === 0 ? 'Load a subject first' : 'Graph view focused on this entity'}
      >
        Galaxy view
      </button>
    </div>
  {/if}

  {#if sessionExpired}
    <p class="state state-error" role="alert">Session expired — sign in again.</p>
  {:else if !hasLoaded}
    <p class="state state-empty">Enter a subject to view its claims.</p>
  {:else if viewMode === 'galaxy'}
    {@const focusId = activeEntityId}
    <GalaxyGraph
      {session}
      zoom="close"
      focus={focusId ?? undefined}
      domain={claims[0]?.domain}
      onNodeClick={onGalaxyNodeClick}
    />
    {#if focusId === null}
      <p class="state state-empty">
        Showing a domain-wide galaxy — no entity_id on the loaded claims.
      </p>
    {/if}
  {:else}
    <StateBox
      loading={loading}
      error={error}
      empty={claims.length === 0}
      emptyText={`No claims found for subject "${activeSubject}".`}
    >
      <div class="table-scroll">
        <table class="claims-table">
          <thead>
            <tr>
              <th>Subject</th>
              <th>Predicate</th>
              <th>Value</th>
              <th>Domain</th>
              <th>Kind</th>
              <th>Origin</th>
              <th>Provenance</th>
              <th>Confidence</th>
              <th>Entity ID</th>
            </tr>
          </thead>
          <tbody>
            {#each claims as claim (claim.claim_id)}
              {@const rowState = expanded[claim.claim_id]}
              {@const isOpen = rowState?.kind === 'open' || rowState?.kind === 'loading'}
              <tr
                class="row"
                class:row-open={isOpen}
                tabindex="0"
                aria-expanded={isOpen}
                aria-label={`Toggle timeline for ${claim.subject} ${claim.predicate}`}
                onclick={() => void toggleRow(claim)}
                onkeydown={(e) => onRowKeydown(e, claim)}
              >
                <td>{claim.subject}</td>
                <td><code class="mono">{claim.predicate}</code></td>
                <td>{formatValue(claim.value)}</td>
                <td>{claim.domain}</td>
                <td>{claim.kind}</td>
                <td>{claim.origin}</td>
                <td>{claim.provenance}</td>
                <td><span class="mono">{confidencePct(claim.confidence)}</span></td>
                <td>{claim.entity_id ?? '—'}</td>
              </tr>
              {#if rowState}
                <tr class="expand-row">
                  <td colspan="9">
                    {#if rowState.kind === 'loading'}
                      <p class="state state-loading" role="status">Loading timeline…</p>
                    {:else if rowState.kind === 'error'}
                      <p class="state state-error" role="alert">{rowState.message}</p>
                    {:else if rowState.kind === 'open'}
                      {#if rowState.entries.length === 0}
                        <p class="state state-empty">No timeline entries.</p>
                      {:else}
                        <ul class="timeline-list">
                          {#each rowState.entries as entry (entry.claim_id)}
                            <li>
                              <dl>
                                <div><dt>Status</dt><dd>{entry.status}</dd></div>
                                <div><dt>Kind</dt><dd>{entry.claim_kind}</dd></div>
                                <div><dt>Confirmed seq</dt><dd><span class="mono">{entry.confirmed_event_seq}</span></dd></div>
                                <div><dt>Valid from</dt><dd>{formatDate(entry.valid_from)}</dd></div>
                                <div><dt>Valid to</dt><dd>{formatDate(entry.valid_to)}</dd></div>
                                <div><dt>Provenance</dt><dd>{entry.provenance_kind}</dd></div>
                                <div><dt>Confidence (bp)</dt><dd><span class="mono">{entry.confidence_basis_points}</span></dd></div>
                              </dl>
                            </li>
                          {/each}
                        </ul>
                      {/if}
                    {/if}
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
    </StateBox>

    {#if activeEntityId}
      <ProvenancePanel
        subject={activeSubject}
        {claims}
        {timelineByPredicate}
        {clients}
      />
      <EntityDestructivePanel
        {session}
        entityId={activeEntityId}
        claims={claims.map((c) => ({
          claim_id: c.claim_id,
          predicate: c.predicate,
          value: c.value,
        }))}
        onMutated={onEntityMutated}
      />
    {:else if claims.length > 0}
      <p class="state state-empty">
        No entity_id on these claims — destructive actions need a resolved entity.
      </p>
    {/if}
  {/if}
</section>

<style>
  /* ── Entity — token-driven, kills the audit findings ─────────────────
   *  - <tr role="button"> dropped (invalid per ARIA — td/tr accept only
   *    row/gridcell/rowheader/header roles). Row stays keyboard-focusable
   *    via tabindex=0 with aria-expanded; the click + Enter/Space handler
   *    (onRowKeydown) provides the activation semantics.
   *  - phantom expand-row: was always-rendered with "Click to expand"
   *    hint; now mounts only when `expanded[claim_id]` is set, so the
   *    table's row count matches the visible claims.
   *  - 9-col table overflow on mobile: wrapped in .table-scroll
   *    (overflow-x: auto) so it scrolls horizontally instead of breaking
   *    the layout.
   *  - tracked-uppercase eyebrow on th → sentence-case Inter.
   *  - 11 hardcoded rgba(127,127,127,X) + 2 rgba(190,70,70,X) → tokens.
   *  - Sub-44px touch targets → 44px min on subject-form button,
   *    view-toggle buttons keep the toolbar compact but hit 44px via
   *    min-height. */

  .page {
    padding: var(--space-lg) 0;
  }

  h1 {
    margin: 0 0 var(--space-md);
    font-family: var(--font-display);
    font-size: var(--text-headline);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--text-headline-tracking);
    line-height: var(--text-headline-leading);
  }

  .subject-form {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-sm);
    align-items: center;
    margin: 0 0 var(--space-md);
  }

  @media (min-width: 30rem) {
    .subject-form {
      grid-template-columns: max-content 1fr auto;
    }
  }

  .subject-form label {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }

  .subject-form input {
    width: 100%;
    min-height: 44px;
    padding: var(--space-sm) var(--space-sm);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    transition: border-color var(--duration-fast) var(--ease-out-quart);
  }

  .subject-form input::placeholder {
    color: var(--text-tertiary);
  }

  .subject-form input:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .subject-form button {
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--color-accent);
    color: var(--text-on-accent);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .subject-form button:hover:not(:disabled) {
    background: var(--color-accent-deep);
  }

  .subject-form button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  /* ── View toggle (table ↔ galaxy) ─────────────────────────────────── */
  .view-toggle {
    display: inline-flex;
    border-radius: var(--radius-md);
    overflow: hidden;
    border: var(--border-hairline);
    margin: 0 0 var(--space-md);
  }

  .view-toggle button {
    min-height: 44px;
    padding: var(--space-xs) var(--space-md);
    border: none;
    border-right: 1px solid var(--color-hairline);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .view-toggle button:last-child {
    border-right: none;
  }

  .view-toggle button:hover:not(:disabled) {
    background: var(--overlay-ink-06);
    color: var(--text-primary);
  }

  .view-toggle button.active {
    background: var(--overlay-ink-15);
    color: var(--text-primary);
    font-weight: var(--weight-semibold);
  }

  .view-toggle button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* ── Claims table — scroll wrapper for the 9-col overflow on mobile ─ */
  .table-scroll {
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: var(--surface-flat);
  }

  .claims-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-primary);
  }

  .claims-table th,
  .claims-table td {
    text-align: left;
    padding: var(--space-sm) var(--space-sm);
    border-bottom: 1px solid var(--color-hairline);
    vertical-align: top;
    word-break: break-word;
  }

  /* Sentence-case th — kills the tracked-uppercase eyebrow. */
  .claims-table th {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    letter-spacing: 0;
    text-transform: none;
    background: var(--surface-sunken);
    position: sticky;
    top: 0;
    z-index: var(--z-base);
  }

  .claims-table tbody tr:last-child td {
    border-bottom: none;
  }

  /* Expandable row — keyboard-focusable (tabindex=0), aria-expanded.
   * role="button" was INVALID on <tr>; we keep tabindex+aria-expanded
   * which is the accessible pattern for an expandable table row. */
  .row {
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .row:hover {
    background: var(--overlay-ink-04);
  }

  /* Keyboard focus ring via inset box-shadow (the global :focus-visible
   * ring is for outline elements; rows use inset to stay inside the
   * table bounds). */
  .row:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--color-accent);
  }

  .row-open {
    background: var(--overlay-ink-06);
  }

  .expand-row > td {
    padding: var(--space-sm) var(--space-md) var(--space-md);
    background: var(--surface-sunken);
    border-bottom: 1px solid var(--color-hairline);
  }

  /* Mono inline (predicate, confidence, seq, bp — machine output). */
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    word-break: break-all;
  }

  /* ── Timeline list (expanded row content) ─────────────────────────── */
  .timeline-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-sm);
  }

  .timeline-list dl {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: var(--space-xs) var(--space-md);
  }

  .timeline-list div {
    display: flex;
    gap: var(--space-sm);
    align-items: baseline;
  }

  .timeline-list dt {
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    min-width: 6rem;
  }

  .timeline-list dd {
    margin: 0;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  /* ── State banners ────────────────────────────────────────────────── */
  .state {
    margin: var(--space-xs) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  .state-loading {
    color: var(--text-secondary);
  }

  .state-error {
    background: var(--overlay-danger-soft);
    border-color: var(--color-danger);
  }

  .state-empty {
    color: var(--text-secondary);
    font-style: italic;
  }
</style>
