<script lang="ts">
  /**
   * Search — `/api/v1/search` read view.
   *
   * Form params: `query` (required), `domain` (optional), `top_k` (optional,
   * default 10 — matches the Rust default in `src/api.rs::search`). The server
   * lowercases the query itself; we send it verbatim so the user sees what
   * they typed.
   *
   * Result cards show every `SearchHit` field. Clicking a card (or focusing
   * it and pressing Enter) stages the subject for the Entity page and
   * navigates there.
   *
   * 4-state coverage via `<StateBox>`:
   *   - loading    → `loading=true` while `api.search()` is in flight.
   *   - error      → any non-401 thrown; rendered as `state-error`.
   *   - empty      → server returned zero results for this query.
   *   - permission → 401 → `session.clear()` + focused banner; App.svelte
   *                  re-renders the login form.
   *
   * On mount, consumes any pending quick-search query staged by Home and, if
   * present, auto-runs the search (so quick-search → land-on-Search with
   * results already on screen).
   */
  import { onMount } from 'svelte'
  import { search, ApiError, type SearchHit } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import { navigate } from '../lib/router'
  import {
    consumePendingQuery,
    setPendingSubject,
  } from '../lib/quickSearch'
  import StateBox from '../components/StateBox.svelte'
  import { formatValue } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // Form state.
  let query = $state('')
  let domain = $state('')
  let topK = $state<number | null>(null)
  let submitting = $state(false)

  // Results state.
  let loading = $state(false)
  let error = $state<string | null>(null)
  let results = $state<SearchHit[]>([])
  let hasSearched = $state(false)
  let sessionExpired = $state(false)

  // Monotonic request-id guard: each `runSearch` invocation bumps this and
  // stamps the call with the new value. A late-arriving response from a
  // superseded search is detected by comparing the stamp to the live value
  // after every `await`, and discarded instead of overwriting newer state.
  let searchSeq = 0

  onMount(() => {
    // Pull a quick-search handoff from Home, if any. Consume-once: a refresh
    // of this page must NOT re-run the staged query.
    const pending = consumePendingQuery()
    if (pending) {
      query = pending
      void runSearch()
    }
  })

  async function runSearch(): Promise<void> {
    const trimmed = query.trim()
    if (!trimmed) return
    // Stamp this call so a newer search can invalidate our stale responses.
    const seq = ++searchSeq
    submitting = true
    loading = true
    error = null
    sessionExpired = false
    hasSearched = true
    try {
      const response = await search({
        query: trimmed,
        domain: domain.trim() || undefined,
        top_k: topK ?? undefined,
      })
      // Discard stale response — a newer search supersedes us.
      if (seq !== searchSeq) return
      results = response.results
    } catch (cause) {
      if (seq !== searchSeq) return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      results = []
      error =
        cause instanceof ApiError
          ? `Search failed (${cause.code}).`
          : 'Search failed — is the backend running on :8080?'
    } finally {
      // Only the most-recent search clears the loading flag — a stale
      // completion would otherwise un-stick a newer in-flight search.
      if (seq === searchSeq) {
        loading = false
        submitting = false
      }
    }
  }

  function onSubmit(event: SubmitEvent): void {
    event.preventDefault()
    void runSearch()
  }

  function openEntity(subject: string): void {
    setPendingSubject(subject)
    navigate('entity')
  }
</script>

<section class="page page-search">
  <h1>Search</h1>

  <form class="search-form" onsubmit={onSubmit}>
    <label for="query">Query</label>
    <input
      id="query"
      type="search"
      required
      placeholder="subject, predicate, or value fragment…"
      bind:value={query}
    />

    <label for="domain">Domain (optional)</label>
    <input id="domain" type="text" bind:value={domain} />

    <label for="top_k">Top K (optional)</label>
    <input
      id="top_k"
      type="number"
      min="1"
      max="100"
      placeholder="10"
      value={topK ?? ''}
      oninput={(e) => {
        const v = (e.currentTarget as HTMLInputElement).value
        topK = v === '' ? null : Number(v)
      }}
    />

    <button type="submit" disabled={submitting}>
      {submitting ? 'Searching…' : 'Search'}
    </button>
  </form>

  <section class="results" aria-live="polite" aria-busy={loading}>
    {#if sessionExpired}
      <p class="state state-error" role="alert">Session expired — sign in again.</p>
    {:else if !hasSearched}
      <p class="state state-empty">Run a search to see matching claims.</p>
    {:else}
      <StateBox
        loading={loading}
        error={error}
        empty={results.length === 0}
        emptyText="No claims matched this query."
      >
        <ul class="hit-list">
          {#each results as hit (hit.claim_id)}
            <li>
              <a
                class="hit-card"
                href="#/entity"
                aria-label={`Open entity ${hit.subject}`}
                onclick={(e) => {
                  e.preventDefault()
                  openEntity(hit.subject)
                }}
              >
                <header class="hit-head">
                  <span class="subject">{hit.subject}</span>
                  <span class="predicate">{hit.predicate}</span>
                </header>
                <dl class="hit-fields">
                  <div><dt>Value</dt><dd>{formatValue(hit.value)}</dd></div>
                  <div><dt>Domain</dt><dd>{hit.domain}</dd></div>
                  <div><dt>Origin</dt><dd>{hit.origin}</dd></div>
                  <div><dt>Provenance</dt><dd>{hit.provenance}</dd></div>
                  <div><dt>Entity ID</dt><dd>{hit.entity_id ?? '—'}</dd></div>
                </dl>
              </a>
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

  .search-form {
    display: grid;
    grid-template-columns: max-content 1fr max-content 1fr max-content 1fr auto;
    gap: 0.5rem;
    align-items: center;
    margin: 0 0 1.25rem;
  }

  .search-form input {
    padding: 0.5rem 0.625rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .search-form button {
    padding: 0.5rem 0.875rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .hit-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.6rem;
  }

  .hit-card {
    padding: 0.75rem 1rem;
    border-radius: 0.5rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.06);
    cursor: pointer;
    outline: none;
  }

  .hit-card:hover,
  .hit-card:focus-visible {
    background: rgba(127, 127, 127, 0.15);
    border-color: rgba(127, 127, 127, 0.65);
  }

  .hit-head {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.4rem;
  }

  .subject {
    font-weight: 600;
  }

  .predicate {
    opacity: 0.85;
    font-style: italic;
  }

  .hit-fields {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.25rem 1rem;
  }

  .hit-fields div {
    display: flex;
    gap: 0.4rem;
  }

  .hit-fields dt {
    opacity: 0.6;
    font-size: 0.85rem;
    min-width: 5.5rem;
  }

  .hit-fields dd {
    margin: 0;
    word-break: break-word;
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

  .state-empty {
    opacity: 0.7;
    font-style: italic;
  }
</style>
