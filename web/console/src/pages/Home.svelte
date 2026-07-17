<script lang="ts">
  /**
   * Home — landing page.
   *
   *   - Brand H1 + tagline.
   *   - Quick-search form → stages the query via `quickSearch.ts` and
   *     `navigate('search')`; the Search page consumes it on mount.
   *   - "Recent claims" preview: pulls `api.inbox()` (the only read endpoint
   *     that works without a query) and shows up to 5 pending proposals as
   *     subject+predicate rows. Clicking a row stages the subject and jumps
   *     to Entity.
   *
   * 4-state coverage via `<StateBox>`:
   *   - loading  → `loading=true` while `inbox()` is in flight.
   *   - error    → any non-401 thrown; rendered as `state-error`.
   *   - empty    → inbox returned `[]`; `emptyText` invites the user to
   *                navigate to Search.
   *   - permission → 401 from `inbox()` → call `session.clear()` so
   *                  App.svelte flips to the login form, and show a focused
   *                  "Session expired" banner. This is the page's 4th state
   *                  (rendered via an inline branch above the StateBox, since
   *                  it's a terminal state — the shell re-renders as login).
   */
  import { onMount } from 'svelte'
  import { inbox, ApiError, type ProposalSummary } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import { navigate } from '../lib/router'
  import {
    setPendingQuery,
    setPendingSubject,
  } from '../lib/quickSearch'
  import StateBox from '../components/StateBox.svelte'
  import { formatValue } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  let quick = $state('')

  // Inbox preview state.
  let loading = $state(false)
  let error = $state<string | null>(null)
  let proposals = $state<ProposalSummary[]>([])
  // Terminal "session expired" state — when set, we render the permission
  // branch instead of the StateBox and rely on App.svelte to swap to login.
  let sessionExpired = $state(false)

  onMount(() => {
    void loadInbox()
  })

  async function loadInbox(): Promise<void> {
    loading = true
    error = null
    sessionExpired = false
    try {
      proposals = await inbox()
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) {
        // Session cookie expired (24h TTL) — the API client has already
        // cleared its CSRF cache (nit #1). Flip the session store too so
        // the shell re-renders the login form; show a focused banner first.
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      error =
        cause instanceof ApiError
          ? `Failed to load recent claims (${cause.code}).`
          : 'Failed to load recent claims — is the backend running on :8080?'
    } finally {
      loading = false
    }
  }

  function onQuickSearch(event: SubmitEvent): void {
    event.preventDefault()
    const trimmed = quick.trim()
    if (!trimmed) return
    setPendingQuery(trimmed)
    navigate('search')
  }

  function openSubject(subject: string): void {
    setPendingSubject(subject)
    navigate('entity')
  }

  // Top 5 most recent pending proposals (server returns them in event-seq
  // order; the slice is a defensive cap).
  let recent = $derived(proposals.slice(0, 5))
</script>

<section class="page page-home">
  <h1>Brain Console</h1>
  <p class="tagline">Review semantic claims, search the ledger, and explore entity timelines.</p>

  <form class="quick" onsubmit={onQuickSearch} role="search">
    <label for="quick" class="visually-hidden">Quick search</label>
    <input
      id="quick"
      type="search"
      placeholder="Search subjects, predicates, values…"
      bind:value={quick}
    />
    <button type="submit">Search</button>
  </form>

  <section class="recent" aria-labelledby="recent-h">
    <h2 id="recent-h">Recent claims</h2>

    {#if sessionExpired}
      <p class="state state-error" role="alert">Session expired — sign in again.</p>
    {:else}
      <StateBox
        loading={loading}
        error={error}
        empty={recent.length === 0}
        emptyText="No pending proposals. Use Search to explore the ledger."
      >
        <ul class="proposal-list">
          {#each recent as p (p.proposal_id)}
            <li>
              <button
                type="button"
                class="proposal-row"
                onclick={() => openSubject(p.subject)}
              >
                <span class="subject">{p.subject}</span>
                <span class="predicate">{p.predicate}</span>
                <span class="value">{formatValue(p.value)}</span>
              </button>
            </li>
          {/each}
        </ul>
      </StateBox>
    {/if}
  </section>
</section>

<style>
  .page {
    padding: 1.5rem 0;
  }

  .tagline {
    margin: 0.25rem 0 1.25rem;
    opacity: 0.75;
  }

  .quick {
    display: flex;
    gap: 0.5rem;
    margin: 0 0 1.5rem;
  }

  .quick input {
    flex: 1;
    padding: 0.5rem 0.625rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .quick button {
    padding: 0.5rem 0.875rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .recent h2 {
    font-size: 1.15rem;
    margin: 0 0 0.5rem;
  }

  .proposal-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.4rem;
  }

  .proposal-row {
    width: 100%;
    display: grid;
    grid-template-columns: 1fr 1fr 1.5fr;
    gap: 0.5rem;
    text-align: left;
    padding: 0.5rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.06);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .proposal-row:hover {
    background: rgba(127, 127, 127, 0.15);
  }

  .subject {
    font-weight: 600;
  }

  .predicate {
    opacity: 0.85;
  }

  .value {
    opacity: 0.7;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .state {
    margin: 1rem 0;
    padding: 0.75rem 1rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
  }

  .state-error {
    background: rgba(190, 70, 70, 0.15);
    border-color: rgba(190, 70, 70, 0.5);
  }
</style>
