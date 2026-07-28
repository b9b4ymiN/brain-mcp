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
  import {
    search,
    galaxy,
    ApiError,
    type SearchHit,
    type GalaxyPayload,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import { navigate } from '../lib/router'
  import {
    consumePendingQuery,
    setPendingSubject,
  } from '../lib/quickSearch'
  import StateBox from '../components/StateBox.svelte'
  import PageHead from '../components/PageHead.svelte'
  import { formatValue } from '../lib/format'

  interface Props {
    session: SessionStore
    toasts: ToastStore
  }

  let { session, toasts }: Props = $props()

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
  let lastQuery = $state('')
  let sessionExpired = $state(false)
  // Post-result re-filter (client-side, narrows the server's top_k hits).
  let resultFilter = $state('')
  let filteredResults = $derived.by<SearchHit[]>(() => {
    const q = resultFilter.trim().toLowerCase()
    if (q.length === 0) return results
    return results.filter((hit) => {
      // Unified filter: cover both page fields (slug/title/excerpt) and claim
      // fields (subject/predicate/value/domain). Fields are optional on the
      // hit type so missing fields contribute '' rather than throwing.
      const haystack = [
        hit.slug ?? '',
        hit.title ?? '',
        hit.summary ?? '',
        hit.subject ?? '',
        hit.predicate ?? '',
        String(hit.value ?? ''),
        hit.domain ?? '',
        hit.origin ?? '',
        hit.provenance ?? '',
      ]
        .join(' ')
        .toLowerCase()
      return haystack.includes(q)
    })
  })

  // Monotonic request-id guard: each `runSearch` invocation bumps this and
  // stamps the call with the new value. A late-arriving response from a
  // superseded search is detected by comparing the stamp to the live value
  // after every `await`, and discarded instead of overwriting newer state.
  let searchSeq = 0

  // ── Example chips (dynamic) ────────────────────────────────────────────
  // The empty-state chips used to be hardcoded `CATL` / `battery` / `revenue`
  // — placeholders left over from CATL-first development that made the page
  // look like a single-company tool regardless of what the brain actually
  // holds. They now mirror Home's `topSubjects`: the most-connected confirmed
  // subjects in the galaxy subgraph (degree in the bounded graph). If the
  // galaxy is empty (no claims confirmed yet) the chips are hidden entirely
  // rather than advertising subjects that don't exist.
  //
  // Keyed by node `id` (entity UUID), NOT `label` — multiple entities can
  // share a label (e.g. three different "CATL" rows across domains in the
  // far-zoom supernode view), and a duplicate each-key trips Svelte's
  // `each_key_duplicate` guard at runtime.
  let topSubjects = $state<{ id: string; label: string; degree: number }[]>([])
  let suggestionsSeq = 0
  let destroyed = false

  onMount(() => {
    void loadSuggestions()
    // Pull a quick-search handoff from Home, if any. Consume-once: a refresh
    // of this page must NOT re-run the staged query.
    const pending = consumePendingQuery()
    if (pending) {
      query = pending
      void runSearch()
    }
  })

  async function loadSuggestions(): Promise<void> {
    const seq = ++suggestionsSeq
    try {
      const result = await galaxy({ zoom: 'far' })
      if (destroyed || seq !== suggestionsSeq) return
      const degree = new Map<string, number>()
      for (const edge of result.edges) {
        degree.set(edge.source, (degree.get(edge.source) ?? 0) + 1)
        degree.set(edge.target, (degree.get(edge.target) ?? 0) + 1)
      }
      topSubjects = result.nodes
        .map((n) => ({ id: n.id, label: n.label, degree: degree.get(n.id) ?? 0 }))
        .filter((n) => n.degree > 0)
        .sort((a, b) => b.degree - a.degree)
        .slice(0, 6)
    } catch {
      // Suggestions are an optional affordance on the empty state — any
      // failure (backend down, 401, etc.) just leaves the chips hidden.
      // Search itself still works; the user can type their own query.
      topSubjects = []
    }
  }

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
      lastQuery = trimmed
    } catch (cause) {
      if (seq !== searchSeq) return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        toasts.push('error', 'Session expired', 'Please sign in again.')
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

  /** Page rows don't fit the Entity page (which is claim-centric). For now
   *  we just stage the slug for the next page view — there's no dedicated
   *  page-viewer route yet, so we fall back to opening the wiki URI in a
   *  new tab. A future Page Viewer page would slot in here cleanly. */
  function openPage(slug: string): void {
    if (!slug) return
    // The wiki URI form is wiki://<wiki>/<slug>; the Console's own router
    // doesn't render pages today, so we open the rendered Hugo mirror in a
    // new tab. The URL pattern matches the Hugo CMS scaffold.
    window.open(`/page/${slug}.html`, '_blank', 'noopener')
  }
</script>

<section class="page page-search">
  <PageHead kicker="Sector scan" title="Search" titleSize="headline" />

  <form class="search-form" onsubmit={onSubmit}>
    <label for="query" class="field-required">Query <span aria-hidden="true">*</span></label>
    <input
      id="query"
      type="search"
      required
      maxlength="256"
      placeholder="Search claims by subject, predicate, or value…"
      bind:value={query}
    />

    <label for="domain" class="field-optional">Domain <span class="opt-hint">optional</span></label>
    <input id="domain" type="text" maxlength="64" bind:value={domain} />

    <label for="top_k" class="field-optional">Top K <span class="opt-hint">optional</span></label>
    <input
      id="top_k"
      type="number"
      min="1"
      max="100"
      maxlength="3"
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

  {#if hasSearched && !loading && !error && results.length > 0}
    <p class="results-echo">
      {results.length} {results.length === 1 ? 'result' : 'results'} for
      <strong>“{lastQuery}”</strong>
      ({results.filter((r) => r.kind === 'page').length} pages ·
      {results.filter((r) => r.kind === 'claim').length} claims)
    </p>
  {/if}

  <section class="results" aria-live="polite" aria-busy={loading}>
    {#if sessionExpired}
      <p class="state state-loading" role="status" aria-live="polite">
        Session ended — sign in again via the prompt.
      </p>
    {:else if !hasSearched}
      <div class="state state-empty">
        <p class="state-empty-headline">Run a search to see matching pages and claims.</p>
        <p class="state-empty-hint">Try a page title, a subject, or a value fragment.</p>
        {#if topSubjects.length > 0}
          <div class="example-chips" role="group" aria-label="Suggested subjects from the graph">
            {#each topSubjects as s (s.id)}
              <button type="button" class="example-chip" onclick={() => { query = s.label; void runSearch(); }}>
                <span aria-hidden="true">⌖</span> {s.label}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <StateBox
        loading={loading}
        error={error}
        empty={results.length === 0}
        emptyText="No pages or claims matched this query."
      >
        {#if resultFilter && filteredResults.length === 0}
          <p class="state state-empty" role="status">No results match "{resultFilter}".</p>
        {:else if filteredResults.length === 0}
          <p class="state state-empty" role="status">No pages or claims matched this query.</p>
        {:else}
          <ol class="hit-list" role="list">
            {#each filteredResults as hit (hit.kind === 'page' ? hit.slug : hit.claim_id)}
              {@const isPage = hit.kind === 'page'}
              {@const tagLabel = isPage ? 'page' : 'claim'}
              {@const tagKind = isPage ? (hit.claim_kind ?? 'page') : (hit.claim_kind ?? 'claim')}
              <li class="hit-card hit-card--{hit.kind}">
                <span class="hit-kind hit-kind--{hit.kind}" title={tagKind}>{tagLabel}</span>
                {#if isPage}
                  <button type="button" class="hit-body hit-body--page" onclick={() => openPage(hit.slug ?? '')}>
                    <span class="hit-title">{hit.title ?? hit.slug ?? '(untitled)'}</span>
                    <span class="hit-meta">
                      <code class="hit-slug">{hit.slug}</code>
                      {#if hit.summary}<span class="hit-summary">· {hit.summary}</span>{/if}
                    </span>
                    {#if hit.excerpt}
                      <p class="hit-excerpt">{@html hit.excerpt}</p>
                    {/if}
                  </button>
                {:else}
                  <button type="button" class="hit-body hit-body--claim" onclick={() => openEntity(hit.subject ?? '')}>
                    <span class="hit-title">{hit.subject} <span class="hit-pred">· {hit.predicate}</span></span>
                    <span class="hit-value">{formatValue(hit.value)}</span>
                    <span class="hit-meta">
                      {#if hit.domain}<span class="hit-domain">{hit.domain}</span>{/if}
                      {#if hit.provenance}<span class="hit-prov">{hit.provenance}</span>{/if}
                    </span>
                  </button>
                {/if}
                <span class="hit-score" title="Normalized relevance score">{((hit.score ?? 0) * 100).toFixed(0)}%</span>
              </li>
            {/each}
          </ol>
        {/if}
      </StateBox>
    {/if}
  </section>
</section>

<style>
  /* ── Search — token-driven, responsive, focus-visible fixed ──────────
   *  - 7-col form grid → stacks on mobile (3 rows of label+input, then
   *    the submit button full-width). Was unreadable under ~640px.
   *  - .hit-card outline:none → replaced with inset box-shadow focus
   *    ring (the prior stripped focus signal failed WCAG 2.4.7).
   *  - Sub-44px touch targets → 44px min on inputs + submit.
   *  - 8 hardcoded grays + 2 reds → tokens.
   *  - .predicate italic + opacity → mono + token (predicate is machine
   *    output; the Mono-Marks-Machine Rule). */

  /* Page chrome (kicker/h1) lives in PageHead now. Top padding is on
   * .shell-main; .page adds only bottom padding for the page rhythm. */
  .page {
    padding-bottom: var(--space-lg);
  }

  /* ── Post-result filter bar ──────────────────────────────────────── */
  .result-filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-bottom: var(--space-sm);
    flex-wrap: wrap;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .filter-input-wrap {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 12rem;
  }

  .filter-icon {
    position: absolute;
    left: var(--space-sm);
    color: var(--text-tertiary);
    pointer-events: none;
  }

  .filter-input {
    width: 100%;
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm) var(--space-xs) calc(var(--space-sm) + 24px);
    border-radius: var(--radius-md);
    border: var(--border-holo);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    transition: border-color var(--duration-fast) var(--ease-out-quart),
      box-shadow var(--duration-fast) var(--ease-out-quart);
  }

  .filter-input::placeholder { color: var(--text-tertiary); }
  .filter-input:focus {
    outline: none;
    border-color: var(--holo-cyan);
    box-shadow: var(--focus-ring);
  }

  .filter-count {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  /* Form: stacks by default (mobile-first), 7-col grid on tablet+. */
  .search-form {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-sm);
    align-items: center;
    margin: 0 0 var(--space-md);
  }

  .search-form label {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }

  /* Required field — slightly louder than optional to guide first-time
   * users to the one field that matters. */
  .search-form .field-required {
    color: var(--text-primary);
    font-weight: var(--weight-semibold);
  }

  .search-form .field-required span {
    color: var(--color-accent);
  }

  /* Optional fields — visually demoted via the "optional" hint suffix. */
  .search-form .field-optional .opt-hint {
    font-family: var(--font-mono);
    font-size: 0.625rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-tertiary);
    margin-left: var(--space-xs);
  }

  /* Results echo — confirms what the user searched for + how many hits. */
  .results-echo {
    margin: 0 0 var(--space-sm);
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-secondary);
  }

  .results-echo strong {
    color: var(--text-primary);
    font-weight: var(--weight-semibold);
  }

  .search-form input {
    width: 100%;
    min-height: 44px;
    padding: var(--space-sm) var(--space-sm);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-body);
    /* 16px prevents iOS Safari auto-zoom-on-focus (which fires when the
     * input font is < 16px). Body text elsewhere can stay at 15px. */
    font-size: 16px;
    transition: border-color var(--duration-fast) var(--ease-out-quart),
      box-shadow var(--duration-fast) var(--ease-out-quart);
  }

  .search-form input::placeholder {
    color: var(--text-tertiary);
  }

  /* Hover affordance: subtle border shift. Keyboard focus: full ring via
   * the shared --focus-ring token (was border-color only, inconsistent
   * with .hit-card's inset shadow fix). */
  .search-form input:hover:not(:focus):not(:disabled) {
    border-color: var(--color-ink-faint);
  }

  .search-form input:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .search-form button {
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

  .search-form button:hover:not(:disabled) {
    /* Keep AA on hover — glow instead of darken. */
    background: var(--color-accent);
    box-shadow: var(--glow-accent);
  }

  .search-form button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  @media (min-width: 48rem) {
    .search-form {
      grid-template-columns: max-content 1fr max-content 1fr max-content 1fr auto;
      align-items: end;
    }
    .search-form button {
      align-self: stretch;
    }
  }

  /* ── Unified hit list (pages + claims, single ranked list) ───────────
   * Each row carries a kind tag (page/claim) on the left, the body button
   * (flexes), and the normalized score on the right. Page rows and claim
   * rows share the card shell; the kind tag's color distinguishes them. */
  .hit-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-sm);
  }

  .hit-card {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: start;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-holo);
    background: var(--surface-holo);
    color: var(--text-primary);
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }
  .hit-card:hover {
    background: var(--surface-holo-raised);
    border-color: color-mix(in oklch, var(--holo-cyan) 55%, var(--color-hairline));
  }

  /* Kind tag — pill, color-coded by source. */
  .hit-kind {
    flex-shrink: 0;
    padding: 2px var(--space-xs);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    font-weight: var(--weight-medium);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    align-self: flex-start;
    margin-top: 2px;
  }
  .hit-kind--page {
    background: var(--surface-accent-soft);
    color: var(--color-accent);
    border: 1px solid var(--color-accent);
  }
  .hit-kind--claim {
    background: var(--surface-info-soft);
    color: var(--color-info);
    border: 1px solid var(--color-info);
  }

  /* Body button — full row click target. */
  .hit-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0;
    background: transparent;
    border: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
    font: inherit;
    min-width: 0;
  }
  .hit-body:hover { color: var(--text-primary); }
  .hit-body:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--color-accent);
    border-radius: var(--radius-sm);
  }

  .hit-title {
    font-family: var(--font-display);
    font-size: var(--text-body);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
  }
  .hit-pred {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    font-weight: var(--weight-regular);
  }
  .hit-value {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-primary);
    word-break: break-word;
  }
  .hit-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    color: var(--text-secondary);
    font-size: var(--text-mono);
    font-family: var(--font-mono);
  }
  .hit-slug {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--holo-cyan);
    word-break: break-all;
  }
  .hit-summary {
    color: var(--text-secondary);
    font-size: var(--text-mono);
  }
  .hit-excerpt {
    margin: var(--space-xs) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-body);
    line-height: 1.4;
    max-height: 4.2em;
    overflow: hidden;
  }
  .hit-excerpt :global(mark) {
    background: var(--surface-accent-soft);
    color: var(--text-primary);
    padding: 0 2px;
    border-radius: 2px;
  }

  /* Score on the right — mono, faint. */
  .hit-score {
    flex-shrink: 0;
    align-self: flex-start;
    margin-top: 2px;
    padding: 2px var(--space-xs);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    background: var(--color-ink-deep);
    border-radius: var(--radius-sm);
  }

  /* ── State banners ────────────────────────────────────────────────── */
  .state {
    margin: var(--space-md) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  .state-empty {
    /* Was a one-line italic gray-on-void nothing-here. Now a teaching
     * panel: opaque accent-soft surface, headline, hint, example chips. */
    background: var(--surface-accent-soft);
    border-color: var(--color-accent);
    color: var(--text-primary);
    font-style: normal;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    align-items: flex-start;
  }

  .state-empty-headline {
    margin: 0;
    font-weight: var(--weight-semibold);
    font-size: var(--text-body);
  }

  .state-empty-hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-label);
  }

  .example-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
  }

  .example-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    /* Touch target — WCAG 2.5.5 (was 36px). */
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--surface-active-nav);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .example-chip:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .example-chip span {
    color: var(--color-accent);
    font-size: 0.9em;
  }
</style>
