/**
 * Svelte 5 rune-based session store for the Console.
 *
 * Holds the CSRF token + login state + the signed-in username. Svelte 5
 * runes (`$state`/`$derived`) work in `.svelte.ts` modules — the compiler
 * transforms them, and importing modules read reactive `.svelte.ts` exports
 * as live bindings. No SvelteKit, no external store library.
 *
 * The store is intentionally per-instance: `createSessionStore()` returns a
 * fresh object. App.svelte creates one and threads it into Login + nav.
 *
 * Phase G (2026-07-20): the transient flash banner fields were removed.
 * Transient user notifications now flow through the toast store
 * (`web/console/src/lib/toast.svelte.ts` + `components/Toaster.svelte`),
 * which supports a stack of dismissible cards instead of a single slot.
 */

import { setCsrfToken } from './api'

/**
 * Reactive session state. Returned by `createSessionStore()`. Mutators are
 * idempotent and safe to call from any component that holds the store.
 */
export interface SessionStore {
  /** The cached CSRF token (or `null` when logged out). */
  readonly csrf: string | null
  /** Derived: `true` iff `csrf` is non-null. */
  readonly isLoggedIn: boolean
  /**
   * The signed-in username (Phase G, 2026-07-20), or `null` when logged out.
   * Surfaced for personalized greetings, reauth form pre-fill, and audit UI.
   * Not used as an auth credential on its own.
   */
  readonly username: string | null
  /** Cache the CSRF token + username (and sync the token to the API client). */
  setCsrf: (token: string | null, username?: string | null) => void
  /** Clear the session (logout). */
  clear: () => void
}

/**
 * Create a fresh, isolated session store. App.svelte calls this once on
 * mount and passes the result down to consumers.
 *
 * NOTE: Svelte 5 runes are file-scoped — the `$state`/`$derived` here are
 * compiled into reactivity primitives by the Svelte 5 Vite plugin, so this
 * `.svelte.ts` module is consumed as a live reactive object, not a snapshot.
 */
export function createSessionStore(): SessionStore {
  let csrf = $state<string | null>(null)
  let username = $state<string | null>(null)

  return {
    get csrf(): string | null {
      return csrf
    },
    get isLoggedIn(): boolean {
      return csrf !== null
    },
    get username(): string | null {
      return username
    },
    setCsrf(token: string | null, newUser?: string | null): void {
      csrf = token
      // Explicit `null` clears the username; `undefined` preserves it (so a
      // CSRF-only refresh doesn't wipe the displayed username).
      if (newUser !== undefined) {
        username = newUser
      }
      // Keep the API client's module-level CSRF cache in sync so mutation
      // POSTs automatically attach the right X-CSRF-Token header.
      setCsrfToken(token)
    },
    clear(): void {
      csrf = null
      username = null
      setCsrfToken(null)
    },
  }
}
