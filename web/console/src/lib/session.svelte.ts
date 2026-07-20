/**
 * Svelte 5 rune-based session store for the Console.
 *
 * Holds the CSRF token + login state + a transient flash banner. Svelte 5
 * runes (`$state`/`$derived`) work in `.svelte.ts` modules — the compiler
 * transforms them, and importing modules read reactive `.svelte.ts` exports
 * as live bindings. No SvelteKit, no external store library.
 *
 * The store is intentionally per-instance: `createSessionStore()` returns a
 * fresh object. App.svelte creates one and threads it into Login + nav.
 */

import { setCsrfToken } from './api'

/** Flash banner severity — mirrors the three UI severities used in nav. */
export type FlashKind = 'success' | 'error' | 'info'

/** One transient banner message. `null` means "no banner visible". */
export interface Flash {
  kind: FlashKind
  text: string
}

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
  /** Current flash banner, or `null` if none. */
  readonly flash: Flash | null
  /** Cache the CSRF token + username (and sync the token to the API client). */
  setCsrf: (token: string | null, username?: string | null) => void
  /** Clear the session (logout). */
  clear: () => void
  /** Show a transient banner. Replaces any existing flash. */
  pushFlash: (kind: FlashKind, text: string) => void
  /** Dismiss the current banner. */
  clearFlash: () => void
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
  let flash = $state<Flash | null>(null)
  // Auto-dismiss timer id for the current flash. We track it so a new flash
  // replaces (not stacks with) the previous timer — otherwise rapid re-pushes
  // leak timeouts and the last-spawned one fires early. Cleared on dismiss.
  let flashTimer: ReturnType<typeof setTimeout> | null = null
  const FLASH_AUTO_DISMISS_MS = 6000

  function clearFlashTimer(): void {
    if (flashTimer !== null) {
      clearTimeout(flashTimer)
      flashTimer = null
    }
  }

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
    get flash(): Flash | null {
      return flash
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
      flash = null
      setCsrfToken(null)
      clearFlashTimer()
    },
    pushFlash(kind: FlashKind, text: string): void {
      // Replace any pending auto-dismiss timer before staging a new one. This
      // keeps at most one in-flight timer per store and prevents leaks when
      // the caller pushes a fresh banner while the old one is still counting
      // down. The manual `x` button (clearFlash) also cancels the timer.
      clearFlashTimer()
      flash = { kind, text }
      flashTimer = setTimeout(() => {
        flashTimer = null
        flash = null
      }, FLASH_AUTO_DISMISS_MS)
    },
    clearFlash(): void {
      flash = null
      clearFlashTimer()
    },
  }
}
