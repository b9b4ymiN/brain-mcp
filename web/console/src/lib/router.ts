/**
 * Minimal hash-based router for the Console.
 *
 * No dependency. Hash routing (rather than history API) avoids any server-
 * side rewrite requirement: `vite preview` and the Rust `ServeDir` fallback
 * both serve `index.html` on `/` and treat `#/...` as a client-only concern.
 *
 * Page identifiers mirror the Rust `llm_wiki::console::ConsolePage` enum's
 * `#[serde(rename_all = "snake_case")]` form so any future server-driven
 * navigation aligns without translation.
 */

/** The five primary Console pages. Mirrors Rust `ConsolePage` (snake_case). */
export type ConsolePage = 'home' | 'search' | 'inbox' | 'entity' | 'operations'

/**
 * Page → hash mapping. The hash is always `#/<page>`. Iterating this record
 * drives the nav bar render in `App.svelte`.
 */
export const ROUTES: Record<ConsolePage, string> = {
  home: '#/home',
  search: '#/search',
  inbox: '#/inbox',
  entity: '#/entity',
  operations: '#/operations',
}

/** All pages in nav-display order. */
export const PAGES: readonly ConsolePage[] = ['home', 'search', 'inbox', 'entity', 'operations']

/** Human-readable label for each page (used in nav + headings). */
export const PAGE_LABELS: Record<ConsolePage, string> = {
  home: 'Home',
  search: 'Search',
  inbox: 'Inbox',
  entity: 'Entity',
  operations: 'Operations',
}

/**
 * Parse `location.hash` into a `ConsolePage`. Defaults to `home` for empty
 * or unrecognized hashes. Trims a leading `#/`.
 */
export function parseHash(hash: string = location.hash): ConsolePage {
  // Strip leading '#' and any leading '/' — tolerate `#home`, `#/home`,
  // `#/home/`, and query-string suffixes.
  const trimmed = hash.replace(/^#\/?/, '').split(/[/?]/)[0]
  if (isConsolePage(trimmed)) {
    return trimmed
  }
  return 'home'
}

/** Type guard: is this string a known `ConsolePage` value? */
function isConsolePage(value: string): value is ConsolePage {
  return value === 'home' ||
    value === 'search' ||
    value === 'inbox' ||
    value === 'entity' ||
    value === 'operations'
}

/** Navigate to a page by setting `location.hash`. No history noise. */
export function navigate(page: ConsolePage): void {
  const next = ROUTES[page]
  if (location.hash !== next) {
    location.hash = next
  }
}

/**
 * Subscribe to route changes. The callback fires immediately with the
 * current page (so callers don't need a separate initial read), then once
 * per `hashchange`. Returns an unsubscribe function.
 */
export function onRouteChange(cb: (page: ConsolePage) => void): () => void {
  const handler = (): void => cb(parseHash())
  // Initial fire so the caller renders the current route without waiting
  // for the first hashchange event.
  cb(parseHash())
  window.addEventListener('hashchange', handler)
  return () => window.removeEventListener('hashchange', handler)
}
