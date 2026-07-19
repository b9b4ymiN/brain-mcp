<script lang="ts">
  /**
   * HoloPanel — the shared "floating holographic readout" panel layout.
   *
   * Every content panel on Operations / Inbox / Entity / Search renders
   * as a HoloPanel. It sits ABOVE the SpaceBackdrop (z-index: 2) and
   * reads as a holographic instrument display: a faint cyan-tinted
   * surface, a 1px cyan-leaning border, a thin animated scan line that
   * sweeps the panel on mount (and repeats slowly), and corner ticks
   * that frame the panel as a HUD element.
   *
   * Voice: Death Star schematic + Apple Vision Pro spatial UI. Calm,
   * precise, instrument-like — not gamey neon.
   *
   * Slot: the panel body. Header (title + optional action) is via props
   * so the layout (title left, action right) is consistent across pages.
   *
   * Props:
   *   - title: the panel heading (h2). Rendered in Space Grotesk — wait,
   *     Bricolage Grotesque. Sentence-case.
   *   - label: optional mono kicker above the title ("TRUST SCANNER" /
   *     "REVIEW QUEUE") — instrument-panel voice. NOT the saturated AI
   *     eyebrow (one per panel, deliberate, mono).
   *   - variant: 'default' | 'primary' | 'danger' | 'success' — controls
   *     border + glow tint. Primary gets a stronger cyan glow (Trust on
   *     Operations, the selected proposal on Inbox). Danger gets red
   *     (Hard Purge, destructive). Success gets green (confirmed state).
   *   - ariaLabel, ariaLive, ariaBusy: pass-through for the section.
   *
   * Reduced motion: scan line becomes static (no sweep).
   */
  import type { Snippet } from 'svelte'

  interface Props {
    title?: string
    label?: string
    variant?: 'default' | 'primary' | 'danger' | 'success'
    actions?: Snippet
    children: Snippet
    ariaLabel?: string
    ariaLive?: 'polite' | 'off'
    ariaBusy?: boolean
  }

  let {
    title,
    label,
    variant = 'default',
    actions,
    children,
    ariaLabel,
    ariaLive = 'off',
    ariaBusy = false,
  }: Props = $props()
</script>

<section
  class="holo-panel holo-panel--{variant}"
  aria-label={ariaLabel}
  aria-live={ariaLive}
  aria-busy={ariaBusy}
>
  <!-- Scan line — animated sweep across the panel. Above content, below
       text (pointer-events: none so it never blocks interaction). -->
  <div class="holo-scan" aria-hidden="true"></div>
  <!-- Corner ticks — HUD framing (4 L-shaped marks at the corners). -->
  <span class="holo-tick holo-tick--tl" aria-hidden="true"></span>
  <span class="holo-tick holo-tick--tr" aria-hidden="true"></span>
  <span class="holo-tick holo-tick--bl" aria-hidden="true"></span>
  <span class="holo-tick holo-tick--br" aria-hidden="true"></span>

  {#if title || label || actions}
    <header class="holo-head">
      <div class="holo-head-text">
        {#if label}
          <p class="holo-label">{label}</p>
        {/if}
        {#if title}
          <h2 class="holo-title">{title}</h2>
        {/if}
      </div>
      {#if actions}
        <div class="holo-actions">
          {@render actions()}
        </div>
      {/if}
    </header>
  {/if}

  <div class="holo-body">
    {@render children()}
  </div>
</section>

<style>
  /* HoloPanel base — token-driven. The cyan tint reads as a holographic
   * readout against the cosmic backdrop; surface stays dark enough that
   * text contrast is unaffected (text AA ≥4.5:1 verified — the tint is
   * in the background layer, not the text color). */
  .holo-panel {
    position: relative;
    z-index: 2;
    padding: var(--space-md);
    border: var(--border-holo);
    border-radius: var(--radius-lg);
    background: var(--surface-holo);
    /* Subtle inner glow that sells the "this is a projection" feel. */
    box-shadow: var(--glow-cyan),
      inset 0 0 0 1px oklch(0.78 0.13 195 / 0.06);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    overflow: hidden;
  }

  /* Variants — change the border/glow tint, never the surface. */
  .holo-panel--primary {
    border: 1px solid color-mix(in oklch, var(--holo-cyan) 55%,
      var(--color-hairline));
    box-shadow: 0 0 24px oklch(0.78 0.13 195 / 0.25),
      inset 0 0 0 1px oklch(0.78 0.13 195 / 0.1);
    background: color-mix(in oklch, var(--color-surface-flat) 82%,
      var(--holo-cyan));
  }

  .holo-panel--danger {
    border: 1px solid var(--color-danger);
    box-shadow: var(--glow-danger),
      inset 0 0 0 1px oklch(0.62 0.20 25 / 0.08);
    background: color-mix(in oklch, var(--color-surface-flat) 85%,
      var(--color-danger));
  }

  .holo-panel--success {
    border: 1px solid var(--color-success);
    box-shadow: var(--glow-success);
  }

  /* ── Scan line ──────────────────────────────────────────────────────
   * A 1px tall gradient line that sweeps top→bottom on a 4s loop.
   * Low alpha so it's ambient, never flickering. */
  .holo-scan {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 60%;
    background: linear-gradient(
      to bottom,
      transparent 0%,
      oklch(0.78 0.13 195 / 0.0) 80%,
      oklch(0.78 0.13 195 / 0.10) 95%,
      oklch(0.78 0.13 195 / 0.18) 100%
    );
    pointer-events: none;
    animation: holo-scan var(--duration-scan) linear infinite;
    will-change: transform;
  }

  @keyframes holo-scan {
    0%   { transform: translateY(-100%); }
    100% { transform: translateY(200%); }
  }

  /* ── Corner ticks (HUD framing) ─────────────────────────────────────
   * 4 L-shaped marks at the corners. Cyan at low alpha, 12px arms. */
  .holo-tick {
    position: absolute;
    width: 10px;
    height: 10px;
    border-color: oklch(0.78 0.13 195 / 0.55);
    pointer-events: none;
  }

  .holo-tick--tl {
    top: 6px;
    left: 6px;
    border-top: 1px solid;
    border-left: 1px solid;
  }

  .holo-tick--tr {
    top: 6px;
    right: 6px;
    border-top: 1px solid;
    border-right: 1px solid;
  }

  .holo-tick--bl {
    bottom: 6px;
    left: 6px;
    border-bottom: 1px solid;
    border-left: 1px solid;
  }

  .holo-tick--br {
    bottom: 6px;
    right: 6px;
    border-bottom: 1px solid;
    border-right: 1px solid;
  }

  /* Danger variant → red ticks (warns the user). */
  .holo-panel--danger .holo-tick {
    border-color: oklch(0.62 0.20 25 / 0.7);
  }

  /* ── Header ───────────────────────────────────────────────────────── */
  .holo-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }

  .holo-head-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
  }

  /* Mono kicker — instrument-panel voice. ONE per panel (the brief calls
   * this out as deliberate brand system, not the saturated eyebrow). */
  .holo-label {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    font-weight: var(--weight-medium);
    color: var(--holo-cyan);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  /* Danger / success variants: keep the semantic hue but pair it with the
   * primary text color so the warning label passes AA on the tinted surface.
   * Pure --color-danger (L=0.62) on the danger-tinted panel measured 3.72:1
   * — failed AA at the 13px mono label size. */
  .holo-panel--danger .holo-label {
    color: var(--text-primary);
  }

  .holo-panel--success .holo-label {
    color: var(--text-primary);
  }

  .holo-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    letter-spacing: 0;
    line-height: var(--text-title-leading);
  }

  .holo-actions {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    flex-shrink: 0;
  }

  /* Body — content. Inherits nothing special; consumer styles their
   * tables / lists / dl against tokens. */
  .holo-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    min-width: 0;
  }

  /* ── Reduced motion: scan line static ─────────────────────────────── */
  @media (prefers-reduced-motion: reduce) {
    .holo-scan {
      animation: none;
      display: none;
    }
  }
</style>
