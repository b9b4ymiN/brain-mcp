/**
 * toast.svelte.ts — toast notification store for the Console (Phase G,
 * 2026-07-20). Replaces the single-slot flash banner with a small stack
 * of dismissible cards at the top-right corner.
 *
 * Svelte 5 runes (`$state`) work in `.svelte.ts` modules — the compiler
 * transforms them, and importing modules read reactive exports as live
 * bindings. Same pattern as `session.svelte.ts`.
 *
 * Design:
 *   - Stack up to MAX_VISIBLE toasts. Pushing past the cap drops the oldest.
 *   - Per-toast auto-dismiss timer (not a single global timer like the
 *     flash had) so each toast dismisses independently.
 *   - Default durations by kind: success/info/warning = 6s, error = 0
 *     (sticky — important errors need explicit dismissal).
 *   - Duplicate suppression: a toast with the same (kind, title, message)
 *     within DEDUP_WINDOW_MS is ignored. Stops error-storm spam.
 *
 * Voice: opaque --surface-*-soft cards (the Phase A opaque tokens, NOT the
 * deprecated --overlay-*-soft washes), 3px semantic-colored left border,
 * mono kicker for the kind label, sans body. Reads as cockpit HUD callouts.
 */

export type ToastKind = 'success' | 'error' | 'info' | 'warning'

export interface Toast {
  /** Unique monotonic id (used as the Svelte each-key + dismiss target). */
  id: number
  kind: ToastKind
  /** Short headline — bold, one line. */
  title: string
  /** Optional body — secondary detail, can wrap. */
  message?: string
  /** Auto-dismiss after this many ms; 0 = sticky (manual dismiss only). */
  duration: number
}

export interface ToastStore {
  /** Reactive ordered list of active toasts (oldest first). */
  readonly toasts: Toast[]
  /**
   * Push a toast. Returns its id so callers can dismiss it programmatically
   * (e.g. a "Working…" toast that the success toast replaces).
   *
   * Pass `duration: 0` to make it sticky. Omit `message` for a headline-only
   * toast. Duplicates (same kind+title+message within DEDUP_WINDOW_MS) are
   * silently ignored and return the id of the existing toast.
   */
  push: (
    kind: ToastKind,
    title: string,
    message?: string,
    duration?: number,
  ) => number
  /** Dismiss the toast with the given id (no-op if already gone). */
  dismiss: (id: number) => void
  /** Dismiss all toasts (used on logout + on session-expiry cascade). */
  clear: () => void
}

const MAX_VISIBLE = 4
const DEDUP_WINDOW_MS = 1000

const DEFAULT_DURATION: Record<ToastKind, number> = {
  success: 6000,
  info: 6000,
  warning: 6000,
  // Errors are sticky — important failures must be explicitly dismissed.
  // Auto-dismissing an error risks the user missing it.
  error: 0,
}

export function createToastStore(): ToastStore {
  let toasts = $state<Toast[]>([])
  let nextId = 1
  /** Tracked timers so we can cancel them on dismiss/clear/replace. */
  const timers = new Map<number, ReturnType<typeof setTimeout>>()
  /** Recent pushes for duplicate suppression. */
  const recent = new Map<string, number>()

  function clearTimer(id: number): void {
    const t = timers.get(id)
    if (t !== undefined) {
      clearTimeout(t)
      timers.delete(id)
    }
  }

  function dismiss(id: number): void {
    clearTimer(id)
    toasts = toasts.filter((t) => t.id !== id)
  }

  function push(
    kind: ToastKind,
    title: string,
    message?: string,
    duration?: number,
  ): number {
    // Duplicate suppression — same kind+title+message within the window.
    const dedupKey = `${kind}|${title}|${message ?? ''}`
    const now = Date.now()
    const lastSeen = recent.get(dedupKey)
    if (lastSeen !== undefined && now - lastSeen < DEDUP_WINDOW_MS) {
      // Find the existing toast with this key (it may still be visible).
      const existing = toasts.find(
        (t) => t.kind === kind && t.title === title && (t.message ?? '') === (message ?? ''),
      )
      if (existing) return existing.id
    }
    recent.set(dedupKey, now)
    // Light cleanup of stale dedup entries (keeps the Map bounded).
    if (recent.size > 32) {
      for (const [k, t] of recent) {
        if (now - t > DEDUP_WINDOW_MS * 4) recent.delete(k)
      }
    }

    const id = nextId++
    const actualDuration = duration ?? DEFAULT_DURATION[kind]
    const toast: Toast = { id, kind, title, message, duration: actualDuration }

    // Cap the stack — drop the oldest visible toast.
    if (toasts.length >= MAX_VISIBLE) {
      const oldest = toasts[0]
      if (oldest) clearTimer(oldest.id)
    }
    toasts = [...toasts.slice(-(MAX_VISIBLE - 1)), toast]

    if (actualDuration > 0) {
      const timer = setTimeout(() => dismiss(id), actualDuration)
      timers.set(id, timer)
    }
    return id
  }

  function clear(): void {
    for (const id of [...timers.keys()]) clearTimer(id)
    toasts = []
  }

  return {
    get toasts(): Toast[] {
      return toasts
    },
    push,
    dismiss,
    clear,
  }
}
