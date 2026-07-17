/**
 * Cross-page transient for "open this in the next page" handoffs.
 *
 * Two one-shot payloads:
 *   - `pendingQuery`   — set by Home's quick-search box, consumed by Search
 *                        on mount (so the user's typed query pre-populates +
 *                        auto-runs).
 *   - `pendingSubject` — set by Search result-card clicks + Home recent-row
 *                        clicks, consumed by Entity on mount (so the user
 *                        lands on a fully-loaded subject view).
 *
 * Read semantics are CONSUME-ONCE: `consumePendingQuery()` returns the staged
 * value and immediately clears it, so a later refresh of the destination page
 * doesn't re-fire the handoff with stale data. Writes overwrite (last-write
 * wins) since at most one navigation is in flight at a time.
 *
 * A dedicated module (rather than threading through `session` or `router`)
 * keeps the channel narrowly scoped to quick-nav handoffs — pages that don't
 * care never see it.
 */

let pendingQuery: string | null = null
let pendingSubject: string | null = null

/** Stage a quick-search query for the Search page to consume on mount. */
export function setPendingQuery(query: string): void {
  pendingQuery = query
}

/** Stage a subject for the Entity page to consume on mount. */
export function setPendingSubject(subject: string): void {
  pendingSubject = subject
}

/** Peek without consuming (used by tests / debugging). */
export function peekPendingQuery(): string | null {
  return pendingQuery
}

/** Peek without consuming (used by tests / debugging). */
export function peekPendingSubject(): string | null {
  return pendingSubject
}

/**
 * Return the staged query and clear it. Returns `null` when nothing staged.
 * Use from Search's mount effect to drive an auto-run.
 */
export function consumePendingQuery(): string | null {
  const q = pendingQuery
  pendingQuery = null
  return q
}

/**
 * Return the staged subject and clear it. Returns `null` when nothing staged.
 * Use from Entity's mount effect to drive an auto-load.
 */
export function consumePendingSubject(): string | null {
  const s = pendingSubject
  pendingSubject = null
  return s
}
