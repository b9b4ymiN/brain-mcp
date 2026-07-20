<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { login as apiLogin, logout as apiLogout, ApiError } from './lib/api'
  import { createSessionStore } from './lib/session.svelte'
  import { createToastStore } from './lib/toast.svelte'
  import {
    navigate,
    onRouteChange,
    parseHash,
    PAGES,
    PAGE_LABELS,
    type ConsolePage,
  } from './lib/router'
  import Home from './pages/Home.svelte'
  import Search from './pages/Search.svelte'
  import Inbox from './pages/Inbox.svelte'
  import Entity from './pages/Entity.svelte'
  import Operations from './pages/Operations.svelte'
  import SpaceBackdrop from './components/SpaceBackdrop.svelte'
  import HudFrame from './components/HudFrame.svelte'
  import Toaster from './components/Toaster.svelte'
  import { systemStatus } from './lib/systemStatus.svelte'

  // One session store for the whole shell. Threads into Login + nav + banner.
  const session = createSessionStore()

  // One toast store for the whole shell (Phase G, 2026-07-20). Mounted once;
  // threaded into every page so any component can push a notification. The
  // Toaster component renders the active stack at z-toast:500.
  const toasts = createToastStore()

  // System status store (cockpit HUD readout) — Home pushes galaxy
  // counts into it; HudFrame + the cockpit footer render it. No
  // prop-drilling. Plain object — Svelte 5 wraps it.
  let status = $state({
    nodeCount: 0,
    edgeCount: 0,
    dotVariant: 'nominal' as 'nominal' | 'warning' | 'alert',
    statusLine: 'BOOTING · STANDBY',
  })
  let unsubscribeStatus: (() => void) | null = null

  // Current route — initialized from parseHash(), updated by hashchange.
  let currentPage = $state<ConsolePage>(parseHash())

  // Login form local state (Phase G, 2026-07-20: username + password).
  let username = $state('')
  let password = $state('')
  let submitting = $state(false)

  let unsubscribe: (() => void) | null = null
  onMount(() => {
    unsubscribe = onRouteChange((page) => {
      currentPage = page
    })
    unsubscribeStatus = systemStatus.subscribe((s) => {
      status = s
    })
  })
  onDestroy(() => {
    if (unsubscribe) unsubscribe()
    if (unsubscribeStatus) unsubscribeStatus()
  })

  // Whether a given page is the active route (for nav active-state styling).
  function isActive(page: ConsolePage): boolean {
    return currentPage === page
  }

  async function handleLogin(): Promise<void> {
    if (submitting) return
    const trimmedUser = username.trim()
    if (!trimmedUser || !password) {
      toasts.push('error', 'Sign-in incomplete', 'Enter your username and password.')
      return
    }
    submitting = true
    try {
      const result = await apiLogin({ username: trimmedUser, password })
      session.setCsrf(result.csrf_token, trimmedUser)
      toasts.push('success', `Welcome, ${trimmedUser}`)
      navigate('home')
    } catch (cause) {
      if (cause instanceof ApiError && cause.code === 'unauthorized') {
        toasts.push('error', 'Wrong username or password', 'Check your credentials and try again.')
      } else if (cause instanceof ApiError) {
        toasts.push('error', 'Sign-in failed', `Server returned: ${cause.code}`)
      } else {
        toasts.push('error', 'Sign-in failed', 'Is the backend running on :8080?')
      }
    } finally {
      submitting = false
    }
  }

  async function handleLogout(): Promise<void> {
    try {
      await apiLogout()
    } catch {
      // Even if the server call fails (network, session already expired),
      // clear local state so the user is dropped back to the login screen.
    }
    session.clear()
    toasts.clear()
    username = ''
    password = ''
    navigate('home')
    // Logout was silent before (no feedback at all). A brief info toast
    // confirms the action without nagging.
    toasts.push('info', 'Signed out')
  }
</script>

<SpaceBackdrop />
<HudFrame dotVariant={status.dotVariant}>
  {#snippet statusSlot()}
    {status.statusLine}
  {/snippet}
</HudFrame>
<Toaster {toasts} />

<header class="shell" class:logged-in={session.isLoggedIn}>
  <a href="#main-content" class="skip-link">Skip to content</a>

  <div class="shell-bar">
    <a
      class="brand"
      href="#/home"
      onclick={(e) => {
        e.preventDefault()
        navigate('home')
      }}
      aria-label="Brain Console — home"
    >
      <span class="brand-sigil" aria-hidden="true">
        <svg width="28" height="28" viewBox="0 0 32 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <!-- Brain sigil: a stylized node-graph mark — central amber node
               with 4 orbital nodes, one elliptical orbit ring. Reads as
               "knowledge graph" + "star system" simultaneously. -->
          <ellipse cx="16" cy="16" rx="13" ry="6" transform="rotate(-20 16 16)" />
          <circle cx="16" cy="16" r="3" fill="currentColor" stroke="none" />
          <circle cx="3" cy="14" r="1.6" fill="currentColor" stroke="none" />
          <circle cx="27" cy="11" r="1.6" fill="currentColor" stroke="none" />
          <circle cx="22" cy="24" r="1.6" fill="currentColor" stroke="none" />
          <circle cx="8" cy="23" r="1.6" fill="currentColor" stroke="none" />
        </svg>
      </span>
      <span class="brand-text">
        <span class="brand-word">Brain Console</span>
        <span class="brand-kicker">Observer's Deck</span>
      </span>
    </a>

    {#if session.isLoggedIn}
      <nav class="primary-nav" aria-label="Primary">
        {#each PAGES as page (page)}
          <a
            href={`#/${page}`}
            class="nav-item"
            class:active={isActive(page)}
            aria-current={isActive(page) ? 'page' : undefined}
            onclick={(e) => {
              e.preventDefault()
              navigate(page)
            }}
          >
            <span class="nav-label">{PAGE_LABELS[page]}</span>
            <span class="nav-tick" aria-hidden="true"></span>
          </a>
        {/each}
      </nav>

      <button type="button" class="logout" onclick={handleLogout}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
          <polyline points="16 17 21 12 16 7" />
          <line x1="21" y1="12" x2="9" y2="12" />
        </svg>
        <span>Sign out</span>
      </button>
    {/if}
  </div>
</header>

<main class="shell-main" class:home-current={currentPage === 'home'} id="main-content" tabindex="-1">
  {#if !session.isLoggedIn}
    <section class="login">
      <h2>Sign in</h2>
      <p class="hint">Enter your username and password to continue.</p>
      <form
        onsubmit={(e) => {
          e.preventDefault()
          void handleLogin()
        }}
      >
        <label for="username">Username</label>
        <input
          id="username"
          name="username"
          type="text"
          autocomplete="username"
          bind:value={username}
          disabled={submitting}
          placeholder="username"
        />
        <label for="password">Password</label>
        <input
          id="password"
          name="password"
          type="password"
          autocomplete="current-password"
          bind:value={password}
          disabled={submitting}
          placeholder="password"
        />
        <button type="submit" disabled={submitting}>
          {submitting ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </section>
  {:else if currentPage === 'home'}
    <Home {session} {toasts} />
  {:else if currentPage === 'search'}
    <Search {session} {toasts} />
  {:else if currentPage === 'inbox'}
    <Inbox {session} {toasts} />
  {:else if currentPage === 'entity'}
    <Entity {session} {toasts} />
  {:else if currentPage === 'operations'}
    <Operations {session} {toasts} />
  {/if}
</main>

{#if session.isLoggedIn && currentPage !== 'home'}
  <!-- Cockpit status-bar footer — anchors the open-bottomed reticle on
       non-Home pages. On Home the hero overlay already carries counts,
       so the footer collapses to nothing there. -->
  <footer class="cockpit-footer" aria-label="System status">
    <span class="footer-cell">
      <span class="footer-dot footer-dot--{status.dotVariant}" aria-hidden="true"></span>
      <span class="footer-label">{status.dotVariant === 'nominal' ? 'LINK' : status.dotVariant === 'warning' ? 'SYNC' : 'LOST'}</span>
    </span>
    <span class="footer-sep" aria-hidden="true">·</span>
    <span class="footer-cell">
      <span class="footer-key">NODES</span>
      <span class="footer-val">{status.nodeCount}</span>
    </span>
    <span class="footer-sep" aria-hidden="true">·</span>
    <span class="footer-cell">
      <span class="footer-key">EDGES</span>
      <span class="footer-val">{status.edgeCount}</span>
    </span>
    <span class="footer-sep" aria-hidden="true">·</span>
    <span class="footer-cell">
      <span class="footer-key">SECTOR</span>
      <span class="footer-val">{PAGE_LABELS[currentPage].toUpperCase()}</span>
    </span>
    <span class="footer-spacer"></span>
    <span class="footer-cell footer-cell--meta">
      <span class="footer-key">BUILD</span>
      <span class="footer-val">v{__APP_VERSION__}</span>
    </span>
  </footer>
{/if}

<style>
  /* Skip-to-content link — visible on focus only (a11y). */
  .skip-link {
    position: absolute;
    top: -100px;
    left: 0;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-accent);
    color: var(--text-on-accent);
    border-radius: var(--radius-md);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    text-decoration: none;
    z-index: var(--z-toast);
    transition: top var(--duration-fast) var(--ease-out-quart);
  }

  .skip-link:focus-visible {
    top: var(--space-sm);
    outline: none;
  }

  /* ── Shell — full-bleed cockpit header bar ───────────────────────────
   * The header is no longer a centered max-width column; it's a full-
   * viewport HUD bar that anchors the cosmic framing. Sticky so it
   * stays visible on long pages (Operations, Entity). Backed by a
   * subtle backdrop blur + cyan hairline bottom border with a soft cyan
   * glow underneath — the "instrument panel separator". */
  .shell {
    position: sticky;
    top: 0;
    z-index: var(--z-sticky);
    background:
      linear-gradient(180deg,
        color-mix(in oklch, var(--color-void) 92%, transparent) 0%,
        color-mix(in oklch, var(--color-void) 75%, transparent) 100%);
    backdrop-filter: blur(14px);
    border-bottom: 1px solid oklch(0.78 0.13 195 / 0.28);
    box-shadow:
      0 1px 0 oklch(0.78 0.13 195 / 0.12),
      0 4px 32px oklch(0.78 0.13 195 / 0.08);
  }

  .shell-bar {
    max-width: var(--shell-max-width);
    margin: 0 auto;
    padding: var(--space-sm) var(--space-md);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }

  /* ── Brand: sigil + wordmark + kicker ─────────────────────────────── */
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    text-decoration: none;
    color: var(--text-primary);
    padding: var(--space-xs) var(--space-xs);
    border-radius: var(--radius-md);
    transition: background var(--duration-fast) var(--ease-out-quart);
    flex-shrink: 0;
  }

  .brand:hover {
    background: var(--overlay-ink-04);
  }

  .brand-sigil {
    display: inline-flex;
    color: var(--color-accent);
    filter: drop-shadow(0 0 8px oklch(0.82 0.14 75 / 0.5));
  }

  .brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 1.0625rem;
    font-weight: var(--weight-semibold);
    letter-spacing: -0.01em;
    color: var(--text-primary);
  }

  .brand-kicker {
    font-family: var(--font-mono);
    font-size: 0.625rem;
    color: var(--holo-cyan);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    margin-top: 1px;
  }

  /* ── Primary nav — instrument selectors ─────────────────────────────
   * Each nav item is a pill with a tiny cyan tick mark above the label.
   * Active item: amber tick + amber label + subtle surface. */
  .primary-nav {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    margin: 0 auto;
    padding: 0;
  }

  .nav-item {
    position: relative;
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm);
    text-decoration: none;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    border-radius: var(--radius-md);
    transition: color var(--duration-fast) var(--ease-out-quart),
      background var(--duration-fast) var(--ease-out-quart);
  }

  .nav-tick {
    width: 16px;
    height: 1px;
    background: var(--color-hairline);
    transform: scaleX(0.6);
    transform-origin: center;
    transition: background var(--duration-fast) var(--ease-out-quart),
      transform var(--duration-fast) var(--ease-out-quart);
  }

  .nav-item:hover {
    color: var(--text-primary);
    background: var(--overlay-ink-04);
  }

  .nav-item:hover .nav-tick {
    background: var(--holo-cyan);
    transform: scaleX(1);
  }

  .nav-item.active {
    color: var(--text-primary);
    /* Opaque surface so the active state is actually visible. The old
     * --overlay-ink-06 wash measured 1.0:1 contrast (text-primary over a
     * 6% white tint over void) — the "you are here" cue was invisible.
     * Surface + inset amber underline reads as a lit cockpit panel. */
    background: var(--surface-active-nav);
    box-shadow: inset 0 -2px 0 var(--color-accent);
  }

  .nav-item.active .nav-tick {
    background: var(--color-accent);
    transform: scaleX(1);
    box-shadow: 0 0 8px var(--color-accent);
  }

  /* ── Logout ───────────────────────────────────────────────────────── */
  .logout {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    min-height: 44px;
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    cursor: pointer;
    flex-shrink: 0;
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  .logout:hover {
    /* Danger hint on hover: signals 'this ends the session' without
     * making the button look destructive at rest. Cockpit voice: the
     * ejector-seat handle is muted until you reach for it. */
    background: var(--surface-active-nav);
    color: var(--color-danger);
    border-color: var(--color-danger);
  }

  /* ── Main + flash ─────────────────────────────────────────────────── */
  .shell-main {
    position: relative;
    z-index: var(--z-base);
    max-width: var(--shell-max-width);
    margin: 0 auto;
    padding: 0 var(--space-lg) var(--space-xxl);
    outline: none;
  }

  .shell-main.home-current {
    max-width: none;
    padding: 0 0 var(--space-xxl);
  }

  /* When the cockpit footer is fixed-bottom, reserve space for it so it
   * never overlaps page content. Footer height ~28px + breathing room. */
  .shell-main:not(.home-current) {
    padding-bottom: calc(var(--space-xxl) + 28px);
  }

  /* ── Login form ───────────────────────────────────────────────────── */
  .login {
    max-width: 24rem;
    padding: var(--space-xl) 0;
    margin: 0 auto;
  }

  .login form {
    display: grid;
    gap: var(--space-sm);
    margin-top: var(--space-sm);
  }

  .login label {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }

  .login input {
    min-height: 44px;
    padding: var(--space-sm) var(--space-sm);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-body);
    transition: border-color var(--duration-fast) var(--ease-out-quart);
  }

  .login input::placeholder { color: var(--text-tertiary); }
  .login input:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .login button {
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: var(--color-accent);
    color: var(--text-on-accent);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .login button:hover:not(:disabled) { background: var(--color-accent-deep); }
  .login button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .hint {
    color: var(--text-secondary);
    margin: var(--space-xs) 0 0;
    font-size: var(--text-body);
  }

  /* ── Cockpit status-bar footer ──────────────────────────────────────
   * Full-viewport mono row at the bottom edge. position: fixed so it's
   * ALWAYS visible (sticky only worked when scrolled to bottom — user
   * feedback). Anchors the open-bottomed HudFrame reticle. Only on non-
   * Home pages (Home's hero overlay already carries the counts). */
  .cockpit-footer {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    background: color-mix(in oklch, var(--color-void) 92%, transparent);
    backdrop-filter: blur(12px);
    border-top: 1px solid oklch(0.78 0.13 195 / 0.25);
    box-shadow: 0 -1px 24px oklch(0.78 0.13 195 / 0.08);
    font-family: var(--font-mono);
    font-size: 0.6875rem;
    color: var(--text-secondary);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .footer-cell {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    white-space: nowrap;
  }

  .footer-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .footer-dot--nominal {
    background: var(--color-success);
    box-shadow: 0 0 6px var(--color-success);
  }

  .footer-dot--warning {
    background: var(--color-accent);
    box-shadow: 0 0 6px var(--color-accent);
    animation: footer-pulse 1.4s var(--ease-breathe) infinite;
  }

  .footer-dot--alert {
    background: var(--color-danger);
    box-shadow: 0 0 8px var(--color-danger);
    animation: footer-pulse 0.8s var(--ease-breathe) infinite;
  }

  @keyframes footer-pulse {
    0%, 100% { opacity: 1; }
    50%      { opacity: 0.4; }
  }

  .footer-key {
    color: var(--text-tertiary);
    font-weight: var(--weight-medium);
  }

  .footer-val {
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .footer-sep {
    color: var(--text-tertiary);
    opacity: 0.6;
  }

  .footer-spacer {
    flex: 1;
  }

  .footer-cell--meta {
    opacity: 0.7;
  }

  /* ── Mobile ─────────────────────────────────────────────────────────
   * Mobile header is a 2-row HUD: row 1 = brand (compact, sigil + word)
   *   + signout (icon-only); row 2 = horizontal-scrolling nav strip.
   * No wrapping, no cramped multi-row flex. */
  @media (max-width: 48rem) {
    .shell-bar {
      flex-wrap: wrap;
      gap: 0;
      padding: var(--space-xs) var(--space-sm);
      align-items: center;
    }

    /* Row 1: brand (left) + signout (right). */
    .brand {
      order: 1;
      flex: 1;
      min-width: 0;
    }

    .brand-word {
      font-size: 1rem;
    }

    .brand-kicker {
      display: none;
    }

    .logout {
      order: 2;
      flex-shrink: 0;
      padding: var(--space-xs);
      min-width: 44px;
      min-height: 44px;
    }

    /* Signout: icon-only on mobile (label hidden). */
    .logout span {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
    }

    /* Row 2: nav as horizontal-scroll strip with cyan divider. The right
     * edge fades into the void so users see there's more to scroll. */
    .primary-nav {
      order: 3;
      width: 100%;
      margin: 0;
      padding: var(--space-xs) 0;
      justify-content: flex-start;
      overflow-x: auto;
      -webkit-overflow-scrolling: touch;
      gap: var(--space-xs);
      border-top: 1px solid oklch(0.78 0.13 195 / 0.15);
      scrollbar-width: none;
      /* Fade the right edge — visible "more here →" affordance without
       * adding scroll arrows or chevrons that would clutter the HUD. */
      -webkit-mask-image: linear-gradient(to right, #000 0%, #000 88%, transparent 100%);
      mask-image: linear-gradient(to right, #000 0%, #000 88%, transparent 100%);
    }

    .primary-nav::-webkit-scrollbar {
      display: none;
    }

    .nav-item {
      flex-shrink: 0;
      padding: var(--space-xs) var(--space-sm);
    }

    /* Footer: hide SECTOR + BUILD cells on mobile to fit one row. */
    .footer-cell--meta,
    .cockpit-footer > .footer-cell:nth-of-type(4),
    .cockpit-footer > .footer-sep:nth-of-type(3) {
      display: none;
    }

    .shell-main,
    .shell-main:not(.home-current) {
      padding-left: var(--space-md);
      padding-right: var(--space-md);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .footer-dot {
      animation: none;
      opacity: 0.85;
    }
  }
</style>
