<script lang="ts">
  /**
   * TimelineCard — compact HoloPanel variant for activity feed entries.
   * 40×40 icon (semantic color) + body slot. No scan line (compact variant).
   */
  import type { Snippet } from 'svelte'

  interface Props {
    /** 40×40 inline SVG. Pass via {#snippet icon()}. */
    icon: Snippet
    /** CSS var name e.g. '--color-success'. */
    iconColorVar: string
    /** Already formatRelative()'d timestamp string. */
    timestamp: string
    /** Title — usually the slug path. */
    title: string
    /** Optional meta line — e.g. "+5 −2 · Alice". */
    meta?: string
    /** Optional subtitle — e.g. commit subject. */
    subtitle?: string
    /** Optional rich body snippet. */
    children?: Snippet
  }
  let { icon, iconColorVar, timestamp, title, meta, subtitle, children }: Props = $props()
</script>

<li class="timeline-card">
  <span class="card-icon" style="color: var({iconColorVar})" aria-hidden="true">
    {@render icon()}
  </span>
  <div class="card-body">
    <p class="card-ts">{timestamp}</p>
    <p class="card-title">{title}</p>
    {#if meta}<p class="card-meta">{meta}</p>{/if}
    {#if subtitle}<p class="card-subtitle">{subtitle}</p>{/if}
    {#if children}{@render children()}{/if}
  </div>
</li>

<style>
  .timeline-card {
    display: flex; gap: var(--space-md); align-items: flex-start;
    padding: var(--space-sm) var(--space-md);
    background: var(--surface-holo);
    border: var(--border-holo); border-radius: var(--radius-md);
  }
  .card-icon {
    flex-shrink: 0; width: 40px; height: 40px;
    display: inline-flex; align-items: center; justify-content: center;
    /* Subtle tint behind the icon — inherited from currentColor (set by iconColorVar). */
    background: color-mix(in oklch, var(--color-surface-flat) 70%, currentColor 8%);
    border-radius: var(--radius-md);
  }
  .card-icon :global(svg) { width: 24px; height: 24px; }
  .card-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .card-ts {
    margin: 0; font-family: var(--font-mono); font-size: var(--text-mono);
    color: var(--text-secondary);
  }
  .card-title {
    margin: 0; font-family: var(--font-mono); font-size: var(--text-body);
    color: var(--text-primary); word-break: break-all;
  }
  .card-meta { margin: 0; font-size: var(--text-body); color: var(--text-secondary); }
  .card-subtitle { margin: 0; font-size: var(--text-body); color: var(--text-secondary); font-style: italic; }
</style>
