<script lang="ts">
  /**
   * DiffPreview — renders a list of field-level before/after diffs for the
   * Inbox review workflow (Task E1.3, mirrors `src/console.rs::DiffPreview`).
   *
   * Render contract:
   *   - Every value goes through `formatValue` and is bound as TEXT (Svelte
   *     text binding auto-escapes). NO raw-HTML bindings anywhere — XSS
   *     payloads in evidence excerpts / claim values land as literal text.
   *     The XSS E2E test (E1.4) relies on this invariant.
   *   - Empty diff list → a "No changes (identical value)" note. Approving
   *     a proposal whose diff is empty is a no-op on the data but still
   *     legal; we surface it so the reviewer knows nothing will change.
   *
   * Accessibility: container carries `role="table"` with an explicit
   * `aria-label="Diff preview"` so screen readers announce the diff as a
   * tabular structure; each row is a `role="row"` with `role="cell"`
   * children, and the header row uses `role="columnheader"`. This is the
   * ARIA-APG layout for a data table that's semantically a table but
   * rendered with custom markup.
   */
  import type { DiffPreview } from '../lib/review'
  import { formatValue } from '../lib/format'

  interface Props {
    diffs: DiffPreview[]
  }

  let { diffs }: Props = $props()
</script>

<section class="diff-preview" aria-label="Diff preview">
  <h3 class="diff-heading">Diff preview</h3>

  {#if diffs.length === 0}
    <p class="diff-empty">No changes (identical value).</p>
  {:else}
    <div class="diff-table" role="table" aria-label="Diff preview rows">
      <div class="diff-row diff-row-head" role="row">
        <span role="columnheader">Field</span>
        <span role="columnheader">Before</span>
        <span role="columnheader" aria-hidden="true"></span>
        <span role="columnheader">After</span>
      </div>
      {#each diffs as d (d.field)}
        <div class="diff-row" role="row">
          <span role="cell" class="diff-field">{d.field}</span>
          <span role="cell" class="diff-before">{formatValue(d.before)}</span>
          <span role="cell" class="diff-arrow" aria-hidden="true">→</span>
          <span role="cell" class="diff-after">{formatValue(d.after)}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .diff-preview {
    margin: 0.5rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.04);
  }

  .diff-heading {
    margin: 0 0 0.4rem;
    font-size: 0.9rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.75;
  }

  .diff-empty {
    margin: 0;
    opacity: 0.7;
    font-style: italic;
  }

  .diff-table {
    display: grid;
    gap: 0.2rem;
  }

  .diff-row {
    display: grid;
    grid-template-columns: minmax(7rem, 1fr) 1fr auto 1fr;
    gap: 0.5rem;
    align-items: start;
    padding: 0.25rem 0;
    word-break: break-word;
  }

  .diff-row-head {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.65;
    border-bottom: 1px solid rgba(127, 127, 127, 0.25);
    padding-bottom: 0.4rem;
  }

  .diff-field {
    font-weight: 600;
    opacity: 0.85;
  }

  .diff-before {
    opacity: 0.65;
    text-decoration: line-through;
    text-decoration-color: rgba(190, 70, 70, 0.55);
  }

  .diff-after {
    color: inherit;
    font-weight: 500;
  }

  .diff-arrow {
    opacity: 0.6;
    text-align: center;
    user-select: none;
  }
</style>
