<script lang="ts" module>
  /**
   * HudFrame — the cockpit HUD overlay mounted once in App.svelte.
   *
   * Frames the entire viewport with subtle cockpit-instrument cues:
   *
   *   - 4 corner brackets (targeting-style L marks) at the viewport edges.
   *   - A status readout top-right: mono text fed via the `status` slot,
   *     reads like a flight-instrument status line.
   *
   * Sits at z-index: var(--z-sticky) — above page content but below
   * modals/toasts. pointer-events: none on the wrapper, but auto on the
   * status readout (in case the consumer adds interactive controls).
   *
   * The corner brackets are pure decoration (aria-hidden); they sell the
   * "you're looking through a cockpit" framing without obstructing any
   * viewport region (they're 24px in the corners only).
   *
   * Reduced motion: no animation here by default (the brackets are static).
   * If a consumer adds a "ping" pulse to the status dot, that animation
   * collapses via the global @media (prefers-reduced-motion: reduce)
   * rule in app.css.
   */
</script>

<script lang="ts">
  import type { Snippet } from 'svelte'

  interface Props {
    /** Status slot — top-right mono readout. Caller passes a snippet
     * that renders e.g. `SYSTEMS NOMINAL · NODES 5 · SYNC ✓`. */
    status?: Snippet
    /** Status dot color — green (nominal), amber (warning), red (alert).
     * Default amber (we're in active operations). */
    dotVariant?: 'nominal' | 'warning' | 'alert'
  }

  let { status, dotVariant = 'nominal' }: Props = $props()
</script>

<div class="hud-frame" aria-hidden="true">
  <span class="hud-bracket hud-bracket--tl"></span>
  <span class="hud-bracket hud-bracket--tr"></span>
  <span class="hud-bracket hud-bracket--bl"></span>
  <span class="hud-bracket hud-bracket--br"></span>
</div>

{#if status}
  <div class="hud-status" role="status" aria-live="off">
    <span class="hud-dot hud-dot--{dotVariant}" aria-hidden="true"></span>
    {@render status()}
  </div>
{/if}

<style>
  /* The frame is fixed to the viewport — it doesn't scroll with content.
   * pointer-events: none so it never blocks clicks in the corners. */
  .hud-frame {
    position: fixed;
    inset: 0;
    z-index: var(--z-sticky);
    pointer-events: none;
  }

  /* Corner brackets — 24px L-marks. Cyan at low alpha. Read as cockpit
   * targeting reticle framing, not as a heavy border. */
  .hud-bracket {
    position: absolute;
    width: 24px;
    height: 24px;
    border-color: oklch(0.78 0.13 195 / 0.35);
  }

  .hud-bracket--tl {
    top: var(--space-sm);
    left: var(--space-sm);
    border-top: 1px solid;
    border-left: 1px solid;
  }

  .hud-bracket--tr {
    top: var(--space-sm);
    right: var(--space-sm);
    border-top: 1px solid;
    border-right: 1px solid;
  }

  .hud-bracket--bl {
    bottom: var(--space-sm);
    left: var(--space-sm);
    border-bottom: 1px solid;
    border-left: 1px solid;
  }

  .hud-bracket--br {
    bottom: var(--space-sm);
    right: var(--space-sm);
    border-bottom: 1px solid;
    border-right: 1px solid;
  }

  /* ── Status readout ────────────────────────────────────────────────
   * Top-right mono text + status dot. Reads like a flight-instrument
   * status line. pointer-events: auto so consumers can place interactive
   * controls here if needed. */
  .hud-status {
    position: fixed;
    top: var(--space-sm);
    right: calc(var(--space-sm) + 32px); /* clear of the bracket */
    z-index: var(--z-sticky);
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-pill);
    background: color-mix(in oklch, var(--color-void) 70%, transparent);
    backdrop-filter: blur(8px);
    border: 1px solid oklch(0.78 0.13 195 / 0.25);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    letter-spacing: 0.05em;
    max-width: calc(100vw - 80px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Status dot — pulsing on nominal, breathing on warning, fast on alert. */
  .hud-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .hud-dot--nominal {
    background: var(--color-success);
    box-shadow: 0 0 8px var(--color-success);
    animation: hud-pulse 2s var(--ease-breathe) infinite;
  }

  .hud-dot--warning {
    background: var(--color-accent);
    box-shadow: 0 0 8px var(--color-accent);
    animation: hud-pulse 1.4s var(--ease-breathe) infinite;
  }

  .hud-dot--alert {
    background: var(--color-danger);
    box-shadow: 0 0 10px var(--color-danger);
    animation: hud-pulse 0.8s var(--ease-breathe) infinite;
  }

  @keyframes hud-pulse {
    0%, 100% { opacity: 1; }
    50%      { opacity: 0.4; }
  }

  /* Mobile: hide the corner brackets (the viewport is small enough that
   * they feel like clutter, not framing). Keep the status readout. */
  @media (max-width: 48rem) {
    .hud-frame {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .hud-dot {
      animation: none;
      opacity: 0.85;
    }
  }
</style>
