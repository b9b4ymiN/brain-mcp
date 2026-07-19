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
  /* Token-driven. The prior tracked-uppercase `.diff-heading` and
   * `.diff-row-head` were the saturated AI-scaffold eyebrow tell —
   * sentence-case Inter now, emphasis via weight only. Hardcoded
   * rgba(127,127,127,X) + rgba(190,70,70,X) → tokens. */

  .diff-preview {
    margin: var(--space-xs) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--overlay-ink-04);
  }

  /* Sentence-case title — no uppercase eyebrow (DESIGN.md No-Eyebrow Rule). */
  .diff-heading {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-body);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    letter-spacing: 0;
    text-transform: none;
    line-height: var(--text-title-leading);
  }

  .diff-empty {
    margin: 0;
    color: var(--text-secondary);
    font-style: italic;
    font-family: var(--font-body);
    font-size: var(--text-body);
  }

  .diff-table {
    display: grid;
    gap: var(--space-xs);
  }

  .diff-row {
    display: grid;
    grid-template-columns: minmax(7rem, 1fr) 1fr auto 1fr;
    gap: var(--space-sm);
    align-items: start;
    padding: var(--space-xs) 0;
    word-break: break-word;
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-primary);
  }

  /* Header row — sentence case, label weight only (kills tracked eyebrow). */
  .diff-row-head {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    letter-spacing: 0;
    text-transform: none;
    border-bottom: 1px solid var(--color-hairline);
    padding-bottom: var(--space-xs);
  }

  .diff-field {
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
  }

  .diff-before {
    color: var(--text-secondary);
    text-decoration: line-through;
    /* Token-driven strike color (was rgba(190,70,70,0.55)). */
    text-decoration-color: var(--color-danger);
  }

  .diff-after {
    color: var(--text-primary);
    font-weight: var(--weight-medium);
  }

  .diff-arrow {
    color: var(--text-tertiary);
    text-align: center;
    user-select: none;
  }
</style>
