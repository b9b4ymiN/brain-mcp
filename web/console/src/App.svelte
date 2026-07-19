<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { login as apiLogin, logout as apiLogout, ApiError } from './lib/api'
  import { createSessionStore } from './lib/session.svelte'
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

  // One session store for the whole shell. Threads into Login + nav + banner.
  const session = createSessionStore()

  // Current route — initialized from parseHash(), updated by hashchange.
  let currentPage = $state<ConsolePage>(parseHash())

  // Login form local state.
  let secret = $state('')
  let submitting = $state(false)

  let unsubscribe: (() => void) | null = null
  onMount(() => {
    unsubscribe = onRouteChange((page) => {
      currentPage = page
    })
  })
  onDestroy(() => {
    if (unsubscribe) unsubscribe()
  })

  // Whether a given page is the active route (for nav active-state styling).
  function isActive(page: ConsolePage): boolean {
    return currentPage === page
  }

  async function handleLogin(): Promise<void> {
    if (submitting) return
    const trimmed = secret.trim()
    if (!trimmed) {
      session.pushFlash('error', 'Enter the bootstrap secret to sign in.')
      return
    }
    submitting = true
    try {
      const result = await apiLogin(trimmed)
      session.setCsrf(result.csrf_token)
      session.pushFlash('success', 'Signed in.')
      navigate('home')
    } catch (cause) {
      const message =
        cause instanceof ApiError && cause.code === 'unauthorized'
          ? 'Wrong secret — try again.'
          : cause instanceof ApiError
            ? `Sign-in failed (${cause.code}).`
            : 'Sign-in failed — is the backend running on :8080?'
      session.pushFlash('error', message)
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
    secret = ''
    navigate('home')
  }
</script>

<header class="shell">
  <a href="#main-content" class="skip-link">Skip to content</a>
  <p class="shell-brand">Brain Console</p>

  {#if session.flash}
    <div class="flash flash-{session.flash.kind}" role="status" aria-live="polite">
      <span>{session.flash.text}</span>
      <button type="button" onclick={() => session.clearFlash()} aria-label="Dismiss">x</button>
    </div>
  {/if}

  {#if session.isLoggedIn}
    <nav aria-label="Primary">
      <ul>
        {#each PAGES as page (page)}
          <li>
            <a
              href={`#/${page}`}
              class:active={isActive(page)}
              aria-current={isActive(page) ? 'page' : undefined}
              onclick={(e) => {
                e.preventDefault()
                navigate(page)
              }}
            >
              {PAGE_LABELS[page]}
            </a>
          </li>
        {/each}
      </ul>
    </nav>
    <button type="button" class="logout" onclick={handleLogout}>Sign out</button>
  {/if}
</header>

<main class="shell-main" class:home-current={currentPage === 'home'} id="main-content" tabindex="-1">
  {#if !session.isLoggedIn}
    <section class="login">
      <h2>Sign in</h2>
      <p class="hint">Enter the console bootstrap secret to continue.</p>
      <form
        onsubmit={(e) => {
          e.preventDefault()
          void handleLogin()
        }}
      >
        <label for="secret">Bootstrap secret</label>
        <input
          id="secret"
          type="password"
          autocomplete="current-password"
          bind:value={secret}
          disabled={submitting}
          placeholder="secret"
        />
        <button type="submit" disabled={submitting}>
          {submitting ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </section>
  {:else if currentPage === 'home'}
    <Home {session} />
  {:else if currentPage === 'search'}
    <Search {session} />
  {:else if currentPage === 'inbox'}
    <Inbox {session} />
  {:else if currentPage === 'entity'}
    <Entity {session} />
  {:else if currentPage === 'operations'}
    <Operations {session} />
  {/if}
</main>

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

  .shell {
    max-width: var(--shell-max-width);
    margin: 0 auto;
    padding: var(--space-lg) var(--space-lg) 0;
  }

  /* Shell wordmark — a <p>, not <h1>. Each page owns its own <h1>
   * (Home: the wordmark; others: the page title). Two <h1> per page
   * violated the one-h1-per-page convention (WCAG 1.3.1). */
  .shell-brand {
    margin: 0 0 var(--space-sm);
    font-family: var(--font-display);
    font-size: var(--text-headline);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--text-headline-tracking);
    line-height: var(--text-headline-leading);
    color: var(--text-primary);
  }

  nav ul {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    padding: 0;
    margin: 0 0 var(--space-md);
  }

  nav a {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    text-decoration: none;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    transition: background var(--duration-fast) var(--ease-out-quart),
      color var(--duration-fast) var(--ease-out-quart);
  }

  nav a:hover {
    color: var(--text-primary);
    background: var(--overlay-ink-04);
  }

  nav a.active {
    color: var(--text-primary);
    background: var(--overlay-ink-06);
  }

  .logout {
    margin: 0 0 var(--space-md);
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  .logout:hover {
    background: var(--overlay-ink-06);
    color: var(--text-primary);
  }

  .shell-main {
    max-width: var(--shell-max-width);
    margin: 0 auto;
    padding: 0 var(--space-lg) var(--space-xl);
    outline: none;
  }

  /* Home is full-bleed — the galaxy hero IS the page. Drop the max-width
   * + horizontal padding so the hero can break out to the viewport edges.
   * Vertical padding stays so the hero doesn't touch the header. */
  .shell-main.home-current {
    max-width: none;
    padding: 0 0 var(--space-xl);
  }

  .login {
    max-width: 24rem;
    padding: var(--space-lg) 0;
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

  .login input::placeholder {
    color: var(--text-tertiary);
  }

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

  .login button:hover:not(:disabled) {
    background: var(--color-accent-deep);
  }

  .login button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .hint {
    color: var(--text-secondary);
    margin: var(--space-xs) 0 0;
    font-size: var(--text-body);
  }

  .flash {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    margin: 0 0 var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    font-family: var(--font-body);
    font-size: var(--text-body);
    color: var(--text-primary);
  }

  .flash-success {
    background: var(--overlay-success-soft);
  }

  .flash-error {
    background: var(--overlay-danger-soft);
  }

  .flash-info {
    background: var(--overlay-info-soft);
  }

  .flash button {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    min-width: 44px;
    min-height: 44px;
  }

  /* Mobile: shell padding tightens */
  @media (max-width: 40rem) {
    .shell,
    .shell-main {
      padding-left: var(--space-md);
      padding-right: var(--space-md);
    }
  }
</style>
