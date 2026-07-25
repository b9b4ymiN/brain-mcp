<script lang="ts">
  /**
   * Timeline — wrapper for a vertical card timeline. Renders a cyan
   * hairline connector down the left edge of the children slot. Purely
   * structural; children are TimelineCard or any block.
   */
  import type { Snippet } from 'svelte'

  interface Props { children: Snippet }
  let { children }: Props = $props()
</script>

<ol class="timeline" role="list">
  {@render children()}
</ol>

<style>
  .timeline {
    list-style: none; margin: 0; padding: 0;
    display: flex; flex-direction: column; gap: var(--space-sm);
    position: relative;
  }
  /* Cyan connector — sits behind cards, runs full height. */
  .timeline::before {
    content: ''; position: absolute;
    left: 20px;   /* aligns with TimelineCard's icon center (40px icon / 2 + padding) */
    top: 0; bottom: 0; width: 1px;
    background: color-mix(in oklch, var(--holo-cyan) 35%, transparent);
    z-index: 0;
    pointer-events: none;
  }
  /* Children sit above the connector. */
  .timeline > :global(*) { position: relative; z-index: 1; }
</style>
