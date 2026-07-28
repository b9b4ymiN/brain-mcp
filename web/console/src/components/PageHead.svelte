<script lang="ts">
  /**
   * PageHead — the single shared page-chrome header for content pages.
   *
   * Replaces the per-page `<header class="page-head">` + `.page-kicker` +
   * `.refresh-all` CSS that was copy-pasted across 8 pages (Today, Status,
   * Activity, Config, Operations, Search, Inbox, Entity). Two divergent
   * conventions had drifted from that duplication:
   *
   *   - Convention A (Today/Status/Activity/Config/Operations): a flex
   *     `<header class="page-head">` with kicker + h1 + tagline on the left
   *     and an optional actions slot (Refresh button, etc.) on the right.
   *   - Convention B (Search/Inbox/Entity): a bare `<p class="page-kicker">`
   *     + `<h1>` with no header wrapper, no actions slot.
   *
   * The drift caused real bugs: Convention A pages forgot the root
   * `.page { padding: var(--space-lg) 0 }` rule that Convention B had, so
   * their content rendered flush against the sticky shell header. This
   * component ends that by owning the chrome once.
   *
   * Top spacing: the gap below the sticky shell header is owned by
   * `.shell-main` (padding-top: var(--space-lg)). PageHead itself does NOT
   * add top padding — that would double up. PageHead only owns its own
   * internal layout (kicker → h1 → tagline + optional actions on the right).
   *
   * Props:
   *   - kicker:     short mono eyebrow above the title (e.g. "Sector scan").
   *   - title:      the page <h1>. Required.
   *   - tagline:    optional one-line description below the title.
   *   - titleSize:  'title' (default, 1.125rem) or 'headline' (1.5rem).
   *                 Operations/Search/Inbox/Entity use the larger headline
   *                 scale; the dashboard pages use title. Kept as a prop
   *                 rather than collapsing to one size to avoid visual
   *                 regressions on the larger pages.
   *   - actions:    optional snippet rendered on the right (Refresh, View all).
   */
  import type { Snippet } from 'svelte'

  interface Props {
    kicker: string
    title: string
    tagline?: string
    titleSize?: 'title' | 'headline'
    actions?: Snippet
  }

  let { kicker, title, tagline = '', titleSize = 'title', actions }: Props = $props()
</script>

<header class="page-head" class:size-headline={titleSize === 'headline'}>
  <div class="page-head-text">
    <p class="page-kicker">{kicker}</p>
    <h1>{title}</h1>
    {#if tagline}<p class="page-tagline">{tagline}</p>{/if}
  </div>
  {#if actions}
    <div class="page-head-actions">
      {@render actions()}
    </div>
  {/if}
</header>

<style>
  /* The chrome lives here once. Pages no longer redefine this.
   *
   * Layout: text block on the left grows; actions pinned to the right.
   * flex-wrap so on narrow viewports the actions drop below the title
   * instead of overflowing horizontally. */
  .page-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .page-head-text {
    min-width: 0;
    /* flex: 1 so the text block takes the space and actions sit at the
     * right edge, but still allows wrap when content is too wide. */
    flex: 1;
  }

  /* Kicker — mono eyebrow in the holo-cyan instrument voice.
   * Sentence case (no tracked-uppercase eyebrow tell per DESIGN.md). */
  .page-kicker {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--holo-cyan);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  /* h1 — display voice, title size by default. Margin-top gives the kicker
   * breathing room above the title (mirrors the per-page rule this replaces).
   * The `.size-headline` modifier (Operations/Search/Inbox/Entity) bumps to
   * the headline scale (1.5rem) those pages used before consolidation. */
  .page-head h1 {
    margin: var(--space-xs) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
  }

  .page-head.size-headline h1 {
    font-size: var(--text-headline);
    line-height: var(--text-headline-leading);
    letter-spacing: var(--text-headline-tracking);
  }

  .page-tagline {
    margin: var(--space-xs) 0 0;
    color: var(--text-secondary);
    max-width: var(--content-measure);
  }

  /* Actions slot — right-aligned. Inline-flex so multiple buttons stack
   * horizontally with consistent gap. */
  .page-head-actions {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    flex-shrink: 0;
  }

  /* Action buttons rendered inside the slot (Refresh, View all, etc.)
   * inherit the shared vocabulary so callers don't redefine it. The
   * `.action-btn` class is opt-in — callers can also pass their own
   * fully-styled button if they need a different variant. */
  .page-head-actions :global(.action-btn) {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    min-height: 44px; /* touch target — WCAG 2.5.5 */
    padding: var(--space-xs) var(--space-sm);
    background: var(--surface-active-nav);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .page-head-actions :global(.action-btn:hover:not(:disabled)) {
    background: var(--surface-raised);
  }

  .page-head-actions :global(.action-btn:disabled) {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .page-head-actions :global(.action-btn:focus-visible) {
    outline: none;
    box-shadow: var(--focus-ring);
  }
</style>
