<script lang="ts">
  /**
   * ErrorBoundary — catches render errors in the wrapped subtree and shows
   * a fallback "signal lost" panel instead of a blank white screen.
   *
   * Svelte 5 has no built-in error boundary (unlike React). This component
   * uses a two-part strategy:
   *
   * 1. A `$state` `error` flag that, when set, swaps the children for the
   *    fallback. The flag is set by:
   *    - `window.onerror` (uncaught throws in event handlers, timers, etc.)
   *    - `window.onunhandledrejection` (unhandled promise rejections)
   *    These are wired in main.ts and call the `trap()` export below.
   *
   * 2. The children snippet is still rendered normally — Svelte's reactivity
   *    catches most errors at the `$effect` / render level and they bubble
   *    to the global handlers. This boundary catches what reaches the top.
   *
   * The fallback is styled to match the cosmic theme: a danger-tinted
   * HoloPanel-style card with a "Signal lost" headline, the error message
   * (truncated), and a Reload button.
   */
  import type { Snippet } from 'svelte'

  interface Props {
    children: Snippet
  }

  let { children }: Props = $props()

  let error = $state<Error | null>(null)

  /** Exported trap so main.ts's global handlers can push errors here. */
  export function trap(err: unknown): void {
    error = err instanceof Error ? err : new Error(String(err))
  }

  /** Exported reset so the Reload button can clear the boundary. */
  export function reset(): void {
    error = null
  }

  function reload(): void {
    reset()
    // Full reload is the safest recovery — guarantees clean state.
    if (typeof window !== 'undefined') window.location.reload()
  }
</script>

{#if error}
  <div class="error-boundary" role="alert" aria-live="assertive">
    <div class="error-card">
      <div class="error-icon" aria-hidden="true">⚠</div>
      <div class="error-body">
        <p class="error-kicker">Signal lost</p>
        <h2 class="error-title">An unexpected error occurred</h2>
        <p class="error-message">{error.message || 'Unknown error'}</p>
      </div>
      <button type="button" class="error-reload" onclick={reload}>
        Reload console
      </button>
    </div>
  </div>
{:else}
  {@render children()}
{/if}

<style>
  .error-boundary {
    position: fixed;
    inset: 0;
    z-index: var(--z-toast);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-lg);
    background: var(--surface-body);
  }

  .error-card {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--space-md);
    max-width: 36rem;
    padding: var(--space-lg) var(--space-xl);
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-lg);
    background: var(--surface-danger-soft);
    box-shadow: var(--glow-danger);
  }

  .error-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: color-mix(in oklch, var(--color-danger) 22%, transparent);
    color: var(--color-danger);
    font-family: var(--font-mono);
    font-size: 18px;
    font-weight: var(--weight-bold);
    line-height: 1;
  }

  .error-body {
    min-width: 0;
  }

  .error-kicker {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.6875rem;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--color-danger);
    opacity: 0.85;
  }

  .error-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-body);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
  }

  .error-message {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    word-break: break-word;
  }

  .error-reload {
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-danger-strong);
    border-radius: var(--radius-md);
    background: var(--color-danger-strong);
    color: #fff;
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-semibold);
    cursor: pointer;
    flex-shrink: 0;
    transition: box-shadow var(--duration-fast) var(--ease-out-quart);
  }

  .error-reload:hover {
    box-shadow: var(--glow-danger);
  }

  @media (max-width: 48rem) {
    .error-card {
      grid-template-columns: 1fr;
      text-align: center;
    }

    .error-icon {
      margin: 0 auto;
    }
  }
</style>
