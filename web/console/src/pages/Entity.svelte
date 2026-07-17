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
   * 4-state coverage via `<StateBox>`:
   *   - loading    → `loading=true` while `getSubject()` is in flight.
   *   - error      → any non-401 thrown; rendered as `state-error`.
   *   - empty      → subject has zero current claims.
   *   - permission → 401 → `session.clear()` + focused banner; App.svelte
   *                  re-renders the login form.
   *
   * Per-row timeline expansion is its OWN micro state machine: a row may be
   * `idle | loading | error | open(timeline[])`. We keep these in a map keyed
   * by claim_id so multiple rows can expand independently.
   */
  import { onMount } from 'svelte'
  import {
    getSubject,
    timeline,
    ApiError,
    type SubjectClaim,
    type ClaimView,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import { consumePendingSubject } from '../lib/quickSearch'
  import StateBox from '../components/StateBox.svelte'
  import { formatValue, formatDate } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // Picker form.
  let subjectInput = $state('')
  let activeSubject = $state('')
  let submitting = $state(false)

  // Claims table state.
  let loading = $state(false)
  let error = $state<string | null>(null)
  let claims = $state<SubjectClaim[]>([])
  let hasLoaded = $state(false)
  let sessionExpired = $state(false)

  // Per-row timeline expansion state.
  type RowState =
    | { kind: 'idle' }
    | { kind: 'loading' }
    | { kind: 'error'; message: string }
    | { kind: 'open'; entries: ClaimView[] }

  let expanded = $state<Record<string, RowState>>({})

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
    activeSubject = trimmed
    submitting = true
    loading = true
    error = null
    sessionExpired = false
    hasLoaded = true
    expanded = {}
    try {
      const response = await getSubject({ subject: trimmed })
      claims = response.claims
    } catch (cause) {
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
      loading = false
      submitting = false
    }
  }

  function onSubmit(event: SubmitEvent): void {
    event.preventDefault()
    void viewSubject(subjectInput)
  }

  async function toggleRow(claim: SubjectClaim): Promise<void> {
    const id = claim.claim_id
    const current = expanded[id]
    // Already open or in flight → collapse back to idle.
    if (current && current.kind !== 'idle') {
      expanded = { ...expanded, [id]: { kind: 'idle' } }
      return
    }
    expanded = { ...expanded, [id]: { kind: 'loading' } }
    try {
      const entries = await timeline({
        domain: claim.domain,
        subject: claim.subject,
        predicate: claim.predicate,
      })
      expanded = { ...expanded, [id]: { kind: 'open', entries } }
    } catch (cause) {
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

  {#if sessionExpired}
    <p class="state state-error" role="alert">Session expired — sign in again.</p>
  {:else if !hasLoaded}
    <p class="state state-empty">Enter a subject to view its claims.</p>
  {:else}
    <StateBox
      loading={loading}
      error={error}
      empty={claims.length === 0}
      emptyText={`No claims found for subject "${activeSubject}".`}
    >
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
            <tr
              class="row"
              tabindex="0"
              role="button"
              aria-expanded={(expanded[claim.claim_id]?.kind ?? 'idle') === 'open'}
              aria-label={`Toggle timeline for ${claim.subject} ${claim.predicate}`}
              onclick={() => void toggleRow(claim)}
              onkeydown={(e) => onRowKeydown(e, claim)}
            >
              <td>{claim.subject}</td>
              <td>{claim.predicate}</td>
              <td>{formatValue(claim.value)}</td>
              <td>{claim.domain}</td>
              <td>{claim.kind}</td>
              <td>{claim.origin}</td>
              <td>{claim.provenance}</td>
              <td>{confidencePct(claim.confidence)}</td>
              <td>{claim.entity_id ?? '—'}</td>
            </tr>
            <tr class="expand-row">
              <td colspan="9">
                {#if expanded[claim.claim_id]}
                  {#snippet rowContent()}
                    {@const row = expanded[claim.claim_id]}
                    {#if row.kind === 'loading'}
                      <p class="state state-loading" role="status">Loading timeline…</p>
                    {:else if row.kind === 'error'}
                      <p class="state state-error" role="alert">{row.message}</p>
                    {:else if row.kind === 'open'}
                      {#if row.entries.length === 0}
                        <p class="state state-empty">No timeline entries.</p>
                      {:else}
                        <ul class="timeline-list">
                          {#each row.entries as entry (entry.claim_id)}
                            <li>
                              <dl>
                                <div><dt>Status</dt><dd>{entry.status}</dd></div>
                                <div><dt>Kind</dt><dd>{entry.claim_kind}</dd></div>
                                <div><dt>Confirmed seq</dt><dd>{entry.confirmed_event_seq}</dd></div>
                                <div><dt>Valid from</dt><dd>{formatDate(entry.valid_from)}</dd></div>
                                <div><dt>Valid to</dt><dd>{formatDate(entry.valid_to)}</dd></div>
                                <div><dt>Provenance</dt><dd>{entry.provenance_kind}</dd></div>
                                <div><dt>Confidence (bp)</dt><dd>{entry.confidence_basis_points}</dd></div>
                              </dl>
                            </li>
                          {/each}
                        </ul>
                      {/if}
                    {/if}
                  {/snippet}
                  {@render rowContent()}
                {:else}
                  <span class="hint">Click to expand timeline.</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </StateBox>
  {/if}
</section>

<style>
  .page {
    padding: 1.5rem 0;
  }

  .subject-form {
    display: grid;
    grid-template-columns: max-content 1fr auto;
    gap: 0.5rem;
    align-items: center;
    margin: 0 0 1.25rem;
  }

  .subject-form input {
    padding: 0.5rem 0.625rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .subject-form button {
    padding: 0.5rem 0.875rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .claims-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
  }

  .claims-table th,
  .claims-table td {
    text-align: left;
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid rgba(127, 127, 127, 0.25);
    vertical-align: top;
    word-break: break-word;
  }

  .claims-table th {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.65;
  }

  .row {
    cursor: pointer;
    outline: none;
  }

  .row:hover,
  .row:focus-visible {
    background: rgba(127, 127, 127, 0.12);
  }

  .expand-row > td {
    padding: 0.5rem 0.75rem 0.75rem;
    background: rgba(127, 127, 127, 0.04);
  }

  .hint {
    opacity: 0.55;
    font-style: italic;
    font-size: 0.85rem;
  }

  .timeline-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }

  .timeline-list dl {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: 0.2rem 1rem;
  }

  .timeline-list div {
    display: flex;
    gap: 0.4rem;
  }

  .timeline-list dt {
    opacity: 0.6;
    font-size: 0.8rem;
    min-width: 6rem;
  }

  .timeline-list dd {
    margin: 0;
  }

  .state {
    margin: 0.5rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
  }

  .state-loading {
    opacity: 0.75;
  }

  .state-error {
    background: rgba(190, 70, 70, 0.15);
    border-color: rgba(190, 70, 70, 0.5);
  }

  .state-empty {
    opacity: 0.7;
    font-style: italic;
  }
</style>
