<script lang="ts">
  /**
   * NavSystemMenu — the "System ▾" dropdown for the 3 admin/health pages
   * (Activity / Status / Config). Uses native click-outside handling (NOT
   * a modal — product ban). Closes on selection, escape, outside-click,
   * route change.
   *
   * IA-B' (2026-07-25 Console expansion): primary nav (6) in the top bar +
   * this dropdown (3) for admin surfaces.
   */
  import { SYSTEM_PAGES, PAGE_LABELS, navigate, type SystemPage } from '../lib/router'

  interface Props {
    /** Currently-active page, or null if primary nav is active. */
    activePage: SystemPage | null
  }
  let { activePage }: Props = $props()

  let open = $state(false)
  let containerEl: HTMLDivElement | null = null

  function toggle(e: MouseEvent) {
    e.stopPropagation()
    open = !open
  }

  function choose(p: SystemPage) {
    open = false
    navigate(p)
  }

  function onWindowClick(e: MouseEvent) {
    if (open && containerEl && !containerEl.contains(e.target as Node)) {
      open = false
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false
  }
</script>

<svelte:window onclick={onWindowClick} onkeydown={onKey} />

<div class="nav-system" bind:this={containerEl}>
  <button
    type="button"
    class="system-toggle"
    aria-haspopup="true"
    aria-expanded={open}
    onclick={toggle}
  >
    <span>System</span>
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="6 9 12 15 18 9" />
    </svg>
  </button>
  {#if open}
    <div class="system-menu" role="menu" aria-label="System pages">
      {#each SYSTEM_PAGES as p (p)}
        <button
          type="button"
          role="menuitem"
          class="system-item"
          class:active={activePage === p}
          onclick={() => choose(p)}
        >
          {PAGE_LABELS[p]}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .nav-system { position: relative; }

  .system-toggle {
    display: inline-flex; align-items: center; gap: var(--space-xs);
    padding: var(--space-xs) var(--space-sm);
    background: transparent;
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-family: var(--font-body); font-size: var(--text-body);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
                color var(--duration-fast) var(--ease-out-quart),
                border-color var(--duration-fast) var(--ease-out-quart);
  }
  .system-toggle:hover {
    background: var(--surface-active-nav);
    color: var(--text-primary);
  }
  .system-toggle[aria-expanded="true"] {
    color: var(--text-primary);
    border-color: var(--holo-cyan);
  }

  .system-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    min-width: 140px;
    padding: var(--space-xs);
    background: var(--surface-overlay);
    border: var(--border-holo);
    border-radius: var(--radius-md);
    box-shadow: var(--glow-cyan);
    display: flex; flex-direction: column; gap: 2px;
    z-index: 100;   /* above sticky header */
  }

  .system-item {
    padding: var(--space-xs) var(--space-sm);
    background: transparent; border: none;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    text-align: left;
    font-family: var(--font-body); font-size: var(--text-body);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
                color var(--duration-fast) var(--ease-out-quart);
  }
  .system-item:hover {
    background: var(--surface-active-nav);
    color: var(--text-primary);
  }
  .system-item.active {
    color: var(--color-accent);
  }
</style>
