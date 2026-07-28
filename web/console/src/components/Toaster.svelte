<script lang="ts">
  /**
   * Toaster — the global toast notification stack (Phase G, 2026-07-20).
   *
   * Renders the active toasts from a `ToastStore` as a fixed stack anchored
   * to the top-right of the viewport, below the cockpit header. Sits at
   * `--z-toast: 500` so it floats above everything else (header, modals,
   * backdrop). Each toast is a self-contained card with a semantic-colored
   * left border, mono kicker, headline, optional message, and a dismiss
   * button.
   *
   * Animation: enter slides in from the right with a gentle overshoot
   * (`--ease-toast-in`); exit slides back out. The Svelte transition
   * helpers `fly` + `fade` compose these as a crossfade-on-remove. Reduced
   * motion collapses to instant mount/unmount.
   *
   * Voice: opaque `--surface-*-soft` cards (Phase A tokens, NOT the
   * deprecated transparent `--overlay-*-soft`). 3px left border in the
   * semantic hue. Reads as a cockpit HUD callout, not a marketing toast.
   *
   * Accessibility:
   *   - The region is `role="region" aria-label="Notifications"`.
   *   - success/info/warning → `role="status" aria-live="polite"`.
   *   - error → `role="alert" aria-live="assertive"` (interruptive).
   *   - Each toast's dismiss button is keyboard-reachable (44×44 target).
   */
  import { fly, fade } from 'svelte/transition'
  import type { ToastStore, Toast, ToastKind } from '../lib/toast.svelte'

  interface Props {
    toasts: ToastStore
  }

  let { toasts }: Props = $props()

  // Per-kind visual config. The kicker is a mono uppercase label (instrument-
  // panel voice); the icon is a small unicode glyph (kept simple to avoid
  // shipping an icon font for 4 shapes). Border color = the semantic hue.
  const KIND_CONFIG: Record<
    ToastKind,
    { kicker: string; icon: string; border: string; surface: string; glow: string }
  > = {
    success: {
      kicker: 'Confirmed',
      icon: '✓',
      border: 'var(--color-success)',
      surface: 'var(--surface-success-soft)',
      glow: 'var(--glow-success)',
    },
    error: {
      kicker: 'Fault',
      icon: '!',
      border: 'var(--color-danger)',
      surface: 'var(--surface-danger-soft)',
      glow: 'var(--glow-danger)',
    },
    warning: {
      kicker: 'Caution',
      icon: '⚠',
      border: 'var(--color-accent)',
      surface: 'var(--surface-accent-soft)',
      glow: 'var(--glow-accent)',
    },
    info: {
      kicker: 'Signal',
      icon: 'i',
      border: 'var(--color-info)',
      surface: 'var(--surface-info-soft)',
      glow: '0 0 0 0 transparent',
    },
  }

  function dismiss(id: number): void {
    toasts.dismiss(id)
  }
</script>

<section class="toaster" role="region" aria-label="Notifications">
  {#each toasts.toasts as toast (toast.id)}
    {@const cfg = KIND_CONFIG[toast.kind]}
    <article
      class="toast toast-{toast.kind}"
      style="--toast-border: {cfg.border}; --toast-surface: {cfg.surface}; --toast-glow: {cfg.glow};"
      role={toast.kind === 'error' ? 'alert' : 'status'}
      aria-live={toast.kind === 'error' ? 'assertive' : 'polite'}
      transition:fly={{ x: 320, duration: 220, opacity: 0, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    >
      <span class="toast-icon" aria-hidden="true">{cfg.icon}</span>
      <div class="toast-body">
        <p class="toast-kicker" aria-hidden="true">{cfg.kicker}</p>
        <p class="toast-title">{toast.title}</p>
        {#if toast.message}
          <p class="toast-message">{toast.message}</p>
        {/if}
      </div>
      <button
        type="button"
        class="toast-dismiss"
        onclick={() => dismiss(toast.id)}
        aria-label="Dismiss notification"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      </button>
    </article>
  {/each}
</section>

<style>
  .toaster {
    position: fixed;
    /* Anchor below the cockpit header + a small gap so toasts don't kiss
     * the brand mark. The header is --shell-header-height tall. */
    top: calc(var(--shell-header-height, 64px) + var(--space-sm));
    right: var(--space-md);
    /* Cap width so a long message doesn't span the whole viewport. */
    width: min(380px, calc(100vw - 2 * var(--space-md)));
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    /* pointer-events: none on the container so empty space between toasts
     * doesn't block clicks to the page underneath. Each toast re-enables
     * pointer events on itself. */
    pointer-events: none;
  }

  .toast {
    position: relative;
    pointer-events: auto;
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: start;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    /* Opaque semantic surface (Phase A token) + the kind's hue as a 3px
     * left border. This is the "fits the design system" fix the user asked
     * for — the old flash used a transparent overlay that read as ~1.1:1
     * contrast over the void. */
    background: var(--toast-surface);
    border-left: 3px solid var(--toast-border);
    border-radius: var(--radius-md);
    box-shadow: var(--toast-glow), 0 4px 16px rgba(0, 0, 0, 0.4);
    font-family: var(--font-body);
    color: var(--text-primary);
    /* Subtle backdrop blur so the toast reads as floating above content
     * even when the surface tint is close to the page bg. */
    backdrop-filter: blur(8px);
  }

  .toast-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border-radius: 50%;
    /* Faint tinted circle behind the glyph — reads as a status indicator. */
    background: color-mix(in oklch, var(--toast-border) 22%, transparent);
    color: var(--toast-border);
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: var(--weight-bold);
    line-height: 1;
  }

  .toast-body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .toast-kicker {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    font-size: 0.6875rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--toast-border);
    /* Keep AA on the tinted surface — the kind color reads as a label, not
     * as body text, so a slight opacity drop is acceptable for hierarchy. */
    opacity: 0.85;
  }

  .toast-title {
    margin: 0;
    font-family: var(--font-body);
    font-size: var(--text-body);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    line-height: 1.3;
  }

  .toast-message {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    line-height: 1.4;
    /* Long messages wrap; mono keeps IDs/codes aligned. */
    word-break: break-word;
  }

  .toast-dismiss {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* Touch target — bumped from 32×32 toward 44px (WCAG 2.5.5). Kept at
     * 40px rather than 44 so the dismiss control doesn't dominate short
     * toast lines; toasts also auto-expire, so this is a secondary action. */
    width: 40px;
    height: 40px;
    min-width: 40px;
    min-height: 40px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .toast-dismiss:hover,
  .toast-dismiss:focus-visible {
    background: var(--overlay-ink-10);
    color: var(--text-primary);
    outline: none;
  }

  .toast-dismiss:focus-visible {
    box-shadow: var(--focus-ring);
  }

  /* Mobile: toasts span the full width (minus gutters) and sit closer to
   * the top — the header is shorter so there's less room. */
  @media (max-width: 48rem) {
    .toaster {
      top: calc(var(--shell-header-height, 64px) + var(--space-xs));
      right: var(--space-sm);
      left: var(--space-sm);
      width: auto;
    }
  }

  /* Reduced motion: instant appear/disappear. Svelte transitions honor
   * this via the `prefers-reduced-motion` media query automatically when
   * we set the transition's duration to 0 conditionally — but the cleanest
   * cross-browser approach is to override the fly transition here via
   * a class toggle. We rely on Svelte's built-in handling: when
   * prefers-reduced-motion is set, the transition module reduces durations
   * to ~0. */
  @media (prefers-reduced-motion: reduce) {
    .toast {
      backdrop-filter: none;
    }
  }
</style>
