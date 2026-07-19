<script lang="ts">
  /**
   * Reusable 4-state shell for read pages (Home / Search / Entity /
   * Operations / Inbox).
   *
   * Renders exactly one of: loading, error, empty, or the children snippet —
   * in that priority order. All dynamic strings are bound as text (auto-
   * escaped by Svelte); raw HTML injection is never used in this component.
   *
   * Caller pattern:
   *
   *   <StateBox loading={loading} error={error} empty={results.length === 0}
   *             emptyText="No results.">
   *     {@render children()}
   *   </StateBox>
   *
   * Parents that wrap a fetch-driven view may also set `aria-busy="true"` on
   * the surrounding container for screen readers; this component itself tags
   * the loading paragraph with `role="status"` and the error with `role=
   * "alert"` per ARIA-APG patterns.
   *
   * Style: fully token-driven (matches DESIGN.md). Prior versions used
   * `opacity: 0.75`/`0.7` to dim loading/empty — that multiplied against
   * the text color's contrast ratio and pushed some states below AA. The
   * dim voice is now carried by `--text-secondary` (a token at 4.9:1 on the
   * void) rather than alpha over the primary text.
   */
  import type { Snippet } from 'svelte'

  interface Props {
    loading: boolean
    error: string | null
    empty: boolean
    emptyText: string
    children: Snippet
  }

  let {
    loading,
    error,
    empty,
    emptyText,
    children,
  }: Props = $props()
</script>

{#if loading}
  <p class="state state-loading" role="status" aria-live="polite">Loading…</p>
{:else if error}
  <p class="state state-error" role="alert">{error}</p>
{:else if empty}
  <p class="state state-empty">{emptyText}</p>
{:else}
  {@render children()}
{/if}

<style>
  .state {
    margin: var(--space-md) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  /* Loading voice: secondary text color, NOT opacity dim. The prior
   * opacity:0.75 multiplied against the primary text's contrast and
   * pushed "Loading…" below AA on some panels. */
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
