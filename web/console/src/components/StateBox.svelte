<script lang="ts">
  /**
   * Reusable 4-state shell for read pages (Home / Search / Entity).
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
    margin: 1rem 0;
    padding: 0.75rem 1rem;
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
