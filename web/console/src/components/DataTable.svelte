<script lang="ts" generics="Row extends Record<string, unknown>">
  /**
   * DataTable — the reusable data-UX primitive for the cosmic console.
   *
   * Used by: Entity (claims table), Search (hit list — as a card variant),
   * Inbox (proposal list — as a card variant), Operations (clients table).
   * One component, four surfaces.
   *
   * Capabilities (all client-side — the API exposes no sort/limit/offset):
   *   - Sort: click a sortable column header → ascending, click again →
   *     descending, click again → reset to original order. Sort state
   *     persists in component state (not URL — these are working
   *     surfaces, not shareable views).
   *   - Filter: a single search input above the table. Matches against
   *     the configured `searchable` columns (defaults to all string
   *     columns). Case-insensitive substring match.
   *   - Pagination: "Show N more" pager. Default page size 10, expanded
   *     in increments of 10. No traditional 1/2/3 page nav — the
   *     "show more" pattern is lighter and matches the cosmic voice
   *     (less chrome).
   *   - Compact mode: optional toggle. When on, hides columns flagged
   *     `hideInCompact`. Persisted to localStorage per-table (keyed by
   *     the `tableId` prop).
   *   - Empty/loading/error: passed as props; the table renders the
   *     StateBox states itself so callers don't have to.
   *
   * Voice: holo-panel "data terminal". Cyan-tinted surface + scan line
   * (via HoloPanel when wrapped, or token-equivalent when bare), mono
   * sort/filter chrome, rows that read as data readouts.
   *
   * Accessibility:
   *   - Sortable headers use `aria-sort` (ascending/descending/none).
   *   - Column headers are real `<th scope="col">`.
   *   - Filter input has a label (visually hidden).
   *   - Row affordances (click/hover) honor focus-visible.
   *   - Touch targets ≥44px on all controls.
   *
   * Reduced motion: the sort/filter transitions are state-only (no
   * layout animation); nothing to disable.
   */
  import { flushSync } from 'svelte'

  interface Column<Row> {
    /** Column key — must match a key on Row. */
    key: string & keyof Row
    /** Header label (sentence case — never uppercase eyebrow). */
    label: string
    /**
     * Cell renderer. Receives the row; returns the display value
     * (string, number, or Svelte-renderable). Default: `String(row[key])`.
     * Use this for mono formatting, ellipsis, badges, etc.
     */
    render?: (row: Row) => unknown
    /** Sortable? Default true. Set false for non-comparable columns. */
    sortable?: boolean
    /** Default sort direction when first clicked. Default 'asc'. */
    defaultSort?: 'asc' | 'desc'
    /**
     * Hide this column in compact mode. Use for constant-per-page columns
     * (Subject on Entity, Domain when uniform) so the table fits without
     * horizontal scroll on narrow viewports.
     */
    hideInCompact?: boolean
    /**
     * Right-align numeric columns (counts, confidence, sizes). Default
     * false. When true, header + cells use `text-align: right` + tabular
     * figures.
     */
    numeric?: boolean
  }

  interface Props<Row> {
    /** Stable id for localStorage persistence (compact mode). */
    tableId: string
    /** Column definitions. */
    columns: Column<Row>[]
    /** Row data. */
    rows: Row[]
    /** Row key — unique id per row for the {#each} keyed block. */
    rowKey: (row: Row) => string
    /** Optional: searchable column keys. Defaults to all string columns. */
    searchableKeys?: string[]
    /** Initial visible row count (default 10). */
    pageSize?: number
    /** Loading state — renders the loading row instead of data. */
    loading?: boolean
    /** Error message — renders the error row. */
    error?: string | null
    /** Empty message — renders the empty row when rows is []. */
    emptyText?: string
    /** Show the filter input (default true). */
    filterable?: boolean
    /** Show the compact toggle (default false — opt in). */
    compactable?: boolean
    /** Show "show more" pager (default true). */
    pageable?: boolean
    /** Row click handler — when set, rows are clickable + get hover. */
    onRowClick?: (row: Row) => void
    /** ARIA label for the table region. */
    ariaLabel?: string
  }

  let {
    tableId,
    columns,
    rows,
    rowKey,
    searchableKeys,
    pageSize = 10,
    loading = false,
    error = null,
    emptyText = 'No rows.',
    filterable = true,
    compactable = false,
    pageable = true,
    onRowClick,
    ariaLabel,
  }: Props<Row> = $props()

  // ── Sort state ──────────────────────────────────────────────────────
  let sortKey = $state<string | null>(null)
  let sortDir = $state<'asc' | 'desc' | null>(null)

  function toggleSort(col: Column<Row>): void {
    if (col.sortable === false) return
    if (sortKey !== col.key) {
      sortKey = col.key
      sortDir = col.defaultSort ?? 'asc'
    } else if (sortDir === 'asc') {
      sortDir = 'desc'
    } else if (sortDir === 'desc') {
      // Third click resets.
      sortKey = null
      sortDir = null
    } else {
      sortDir = 'asc'
    }
  }

  function ariaSort(col: Column<Row>): 'ascending' | 'descending' | 'none' {
    if (sortKey !== col.key || sortDir === null) return 'none'
    return sortDir === 'asc' ? 'ascending' : 'descending'
  }

  // ── Filter state ────────────────────────────────────────────────────
  let filterText = $state('')
  let filterKeys = $derived(
    (searchableKeys ?? columns.filter((c) => c.sortable !== false).map((c) => c.key)) as string[],
  )

  // ── Compact mode (persisted) ────────────────────────────────────────
  const STORAGE_KEY = `bc-table-compact-${tableId}`
  let compact = $state(false)
  if (typeof localStorage !== 'undefined') {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored === 'true') compact = true
  }
  function toggleCompact(): void {
    compact = !compact
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(STORAGE_KEY, String(compact))
    }
  }

  let visibleColumns = $derived(
    compact ? columns.filter((c) => !c.hideInCompact) : columns,
  )

  // ── Derived: filtered + sorted rows ─────────────────────────────────
  let processedRows = $derived.by<Row[]>(() => {
    let out = rows
    // Filter
    const q = filterText.trim().toLowerCase()
    if (q.length > 0) {
      out = out.filter((row) =>
        filterKeys.some((k) => {
          const v = (row as Record<string, unknown>)[k]
          return v != null && String(v).toLowerCase().includes(q)
        }),
      )
    }
    // Sort
    if (sortKey !== null && sortDir !== null) {
      const key = sortKey as keyof Row
      const dir = sortDir === 'asc' ? 1 : -1
      out = [...out].sort((a, b) => {
        const av = a[key]
        const bv = b[key]
        if (av == null && bv == null) return 0
        if (av == null) return 1
        if (bv == null) return -1
        if (typeof av === 'number' && typeof bv === 'number') {
          return (av - bv) * dir
        }
        return String(av).localeCompare(String(bv)) * dir
      })
    }
    return out
  })

  // ── Pagination (show-more pattern) ──────────────────────────────────
  let visibleCount = $state(pageSize)

  // Reset visible count when filter/sort changes (so the user sees fresh
  // top results, not a stale window into the old ordering).
  $effect(() => {
    // Read dependencies so Svelte re-runs this when they change.
    void filterText
    void sortKey
    void sortDir
    void rows.length
    visibleCount = pageSize
  })

  let visibleRows = $derived(processedRows.slice(0, visibleCount))
  let hasMore = $derived(visibleCount < processedRows.length)
  function showMore(): void {
    visibleCount += pageSize
  }

  // ── Cell rendering ──────────────────────────────────────────────────
  function renderCell(col: Column<Row>, row: Row): unknown {
    if (col.render) return col.render(row)
    const v = (row as Record<string, unknown>)[col.key]
    return v == null ? '—' : String(v)
  }

  // Is the cell a mono-formatted string (set by caller via a wrapper)?
  // Convention: render functions can return { __mono: true, text: string }
  // to opt into mono styling. Simpler than passing a per-cell className.
  // For now: callers return plain strings; we render all numeric columns
  // as mono (the Mono-Marks-Machine Rule).
</script>

<div class="bc-data-table" aria-label={ariaLabel}>
  {#if filterable || compactable}
    <div class="table-chrome">
      {#if filterable}
        <div class="table-filter">
          <label for="filter-{tableId}" class="filter-label">Filter</label>
          <div class="filter-input-wrap">
            <svg class="filter-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              id="filter-{tableId}"
              type="search"
              class="filter-input"
              placeholder="Filter rows…"
              bind:value={filterText}
            />
          </div>
        </div>
      {/if}
      {#if compactable}
        <button
          type="button"
          class="compact-toggle"
          onclick={toggleCompact}
          aria-pressed={compact}
          title={compact ? 'Show all columns' : 'Hide constant columns'}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
          </svg>
          <span>{compact ? 'Expand' : 'Compact'}</span>
        </button>
      {/if}
      {#if filterText}
        <span class="filter-count">{processedRows.length} match{processedRows.length === 1 ? '' : 'es'}</span>
      {/if}
    </div>
  {/if}

  {#if loading}
    <div class="table-state table-state--loading" role="status" aria-live="polite">
      <span class="state-pulse" aria-hidden="true"></span>
      <span>Loading…</span>
    </div>
  {:else if error}
    <div class="table-state table-state--error" role="alert">{error}</div>
  {:else if processedRows.length === 0}
    <div class="table-state table-state--empty">
      {filterText ? `No rows match "${filterText}".` : emptyText}
    </div>
  {:else}
    <div class="table-scroll">
      <table class="data-table">
        <thead>
          <tr>
            {#each visibleColumns as col (col.key)}
              <th
                scope="col"
                class:sortable={col.sortable !== false}
                class:numeric={col.numeric}
                class:sorted={sortKey === col.key}
                aria-sort={ariaSort(col)}
                onclick={() => toggleSort(col)}
              >
                <span class="th-label">{col.label}</span>
                {#if col.sortable !== false}
                  <span class="sort-indicator" aria-hidden="true">
                    {#if sortKey === col.key}
                      {#if sortDir === 'asc'}▲{:else if sortDir === 'desc'}▼{/if}
                    {:else}
                      <span class="sort-idle">⇅</span>
                    {/if}
                  </span>
                {/if}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each visibleRows as row (rowKey(row))}
            <tr
              class="data-row"
              class:clickable={!!onRowClick}
              onclick={onRowClick ? () => onRowClick(row) : undefined}
            >
              {#each visibleColumns as col (col.key)}
                <td class:numeric={col.numeric} class:mono={col.numeric}>
                  {renderCell(col, row)}
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if pageable && hasMore}
      <div class="table-pager">
        <span class="pager-info">
          Showing {visibleRows.length} of {processedRows.length}
        </span>
        <button type="button" class="pager-btn" onclick={showMore}>
          Show {Math.min(pageSize, processedRows.length - visibleCount)} more
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  /* ── DataTable — holo "data terminal" voice ──────────────────────────
   * The table reads as a holographic data readout. Cyan-tinted surface
   * matches HoloPanel; mono sort/filter chrome; rows hover-brighten. */

  .bc-data-table {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    width: 100%;
    min-width: 0;
  }

  /* ── Chrome (filter + compact toggle) ─────────────────────────────── */
  .table-chrome {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }

  .table-filter {
    flex: 1;
    min-width: 12rem;
  }

  .filter-label {
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

  .filter-input::placeholder {
    color: var(--text-tertiary);
  }

  .filter-input:focus {
    outline: none;
    border-color: var(--holo-cyan);
    box-shadow: var(--focus-ring);
  }

  .compact-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-holo);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .compact-toggle:hover {
    background: var(--overlay-ink-04);
    color: var(--text-primary);
  }

  .compact-toggle[aria-pressed='true'] {
    background: var(--overlay-ink-06);
    color: var(--holo-cyan);
    border-color: color-mix(in oklch, var(--holo-cyan) 45%, var(--color-hairline));
  }

  .filter-count {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  /* ── Table itself ─────────────────────────────────────────────────── */
  .table-scroll {
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    border: var(--border-holo);
    border-radius: var(--radius-md);
    background: var(--surface-holo);
    box-shadow: var(--glow-cyan);
  }

  .data-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-primary);
  }

  .data-table th,
  .data-table td {
    text-align: left;
    padding: var(--space-sm);
    border-bottom: 1px solid var(--color-hairline);
    vertical-align: top;
    word-break: break-word;
  }

  /* Sentence-case header. Sortable headers get a pointer cursor + the
   * sort indicator. Active sort column gets a subtle cyan tint. */
  .data-table th {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    letter-spacing: 0;
    background: var(--surface-sunken);
    position: sticky;
    top: 0;
    z-index: var(--z-base);
    white-space: nowrap;
  }

  .data-table th.sortable {
    cursor: pointer;
    user-select: none;
  }

  .data-table th.sortable:hover {
    color: var(--text-primary);
  }

  .data-table th.sorted {
    color: var(--holo-cyan);
    background: color-mix(in oklch, var(--surface-sunken) 88%, var(--holo-cyan));
  }

  .th-label {
    display: inline;
  }

  .sort-indicator {
    display: inline-block;
    margin-left: var(--space-xs);
    font-size: 0.7em;
    color: var(--holo-cyan);
    min-width: 0.75em;
  }

  .sort-idle {
    color: var(--text-tertiary);
    opacity: 0.5;
  }

  .data-table th.numeric,
  .data-table td.numeric {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .data-table td.mono {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
  }

  .data-table tbody tr:last-child td {
    border-bottom: none;
  }

  .data-row {
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .data-row:hover {
    background: var(--overlay-ink-04);
  }

  .data-row.clickable {
    cursor: pointer;
  }

  .data-row.clickable:focus-within {
    box-shadow: inset 0 0 0 2px var(--color-accent);
    outline: none;
  }

  /* ── State banners ────────────────────────────────────────────────── */
  .table-state {
    padding: var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .table-state--loading {
    color: var(--text-secondary);
  }

  .table-state--error {
    background: var(--overlay-danger-soft);
    border-color: var(--color-danger);
  }

  .table-state--empty {
    color: var(--text-secondary);
    font-style: italic;
  }

  .state-pulse {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-accent);
    animation: bc-table-pulse 1.4s var(--ease-breathe) infinite;
  }

  @keyframes bc-table-pulse {
    0%, 100% { opacity: 0.4; transform: scale(0.8); }
    50%      { opacity: 1;   transform: scale(1.1); }
  }

  /* ── Pager ────────────────────────────────────────────────────────── */
  .table-pager {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }

  .pager-info {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-tertiary);
  }

  .pager-btn {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
    padding: var(--space-xs) var(--space-md);
    border: var(--border-holo);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }

  .pager-btn:hover {
    background: var(--overlay-ink-06);
    border-color: color-mix(in oklch, var(--holo-cyan) 50%, var(--color-hairline));
  }

  /* ── Mobile ───────────────────────────────────────────────────────── */
  @media (max-width: 48rem) {
    .table-chrome {
      gap: var(--space-xs);
    }

    .compact-toggle span {
      display: none;
    }

    .compact-toggle {
      padding: var(--space-xs);
      min-width: 44px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .state-pulse {
      animation: none;
      opacity: 0.8;
    }
  }
</style>
