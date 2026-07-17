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
  <h1>Brain Console</h1>

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

<main class="shell-main">
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
    <Inbox />
  {:else if currentPage === 'entity'}
    <Entity {session} />
  {:else if currentPage === 'operations'}
    <Operations />
  {/if}
</main>

<style>
  .shell {
    max-width: 56rem;
    margin: 0 auto;
    padding: 1.5rem 1.5rem 0;
  }

  h1 {
    margin: 0 0 0.5rem;
    font-size: 1.75rem;
    font-weight: 650;
  }

  nav ul {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    padding: 0;
    margin: 0 0 1rem;
  }

  nav a {
    display: inline-block;
    padding: 0.4rem 0.75rem;
    border-radius: 0.375rem;
    text-decoration: none;
    color: inherit;
    opacity: 0.75;
  }

  nav a:hover {
    opacity: 1;
    background: rgba(127, 127, 127, 0.15);
  }

  nav a.active {
    opacity: 1;
    background: rgba(127, 127, 127, 0.25);
    font-weight: 600;
  }

  .logout {
    margin: 0 0 1rem;
  }

  .shell-main {
    max-width: 56rem;
    margin: 0 auto;
    padding: 0 1.5rem 2rem;
  }

  .login {
    max-width: 24rem;
    padding: 1.5rem 0;
  }

  .login form {
    display: grid;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  .login input {
    padding: 0.5rem 0.625rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .login button {
    padding: 0.5rem 0.875rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .hint {
    opacity: 0.7;
    margin: 0.25rem 0 0;
  }

  .flash {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 0.75rem;
    margin: 0 0 1rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.4);
  }

  .flash-success {
    background: rgba(60, 160, 90, 0.18);
  }

  .flash-error {
    background: rgba(190, 70, 70, 0.18);
  }

  .flash-info {
    background: rgba(80, 130, 200, 0.18);
  }

  .flash button {
    background: transparent;
    border: none;
    color: inherit;
    cursor: pointer;
    font-weight: 600;
  }
</style>
