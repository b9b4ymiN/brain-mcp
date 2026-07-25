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

/**
 * The Console pages. Mirrors Rust `ConsolePage` (snake_case).
 * Split into primary nav (6) and system dropdown (3) per IA-B'.
 */
export type PrimaryPage = 'home' | 'today' | 'search' | 'inbox' | 'entity' | 'operations'
export type SystemPage = 'activity' | 'status' | 'config'
export type ConsolePage = PrimaryPage | SystemPage

/** Page → hash mapping. Hash is always `#/<page>`. */
export const ROUTES: Record<ConsolePage, string> = {
  home: '#/home',
  today: '#/today',
  search: '#/search',
  inbox: '#/inbox',
  entity: '#/entity',
  operations: '#/operations',
  activity: '#/activity',
  status: '#/status',
  config: '#/config',
}

/** Primary nav in display order (top bar). */
export const PRIMARY_PAGES: readonly PrimaryPage[] =
  ['home', 'today', 'search', 'inbox', 'entity', 'operations']

/** System dropdown in display order. */
export const SYSTEM_PAGES: readonly SystemPage[] = ['activity', 'status', 'config']

/** All pages — kept for callers that iterate every page (e.g. footer). */
export const PAGES: readonly ConsolePage[] = [...PRIMARY_PAGES, ...SYSTEM_PAGES]

/** Human-readable label per page (nav + headings + footer). */
export const PAGE_LABELS: Record<ConsolePage, string> = {
  home: 'Home',
  today: 'Today',
  search: 'Search',
  inbox: 'Inbox',
  entity: 'Entity',
  operations: 'Operations',
  activity: 'Activity',
  status: 'Status',
  config: 'Config',
}

/** Parse `location.hash` into a `ConsolePage`. Defaults to `home`. */
export function parseHash(hash: string = location.hash): ConsolePage {
  const trimmed = hash.replace(/^#\/?/, '').split(/[/?]/)[0]
  if (isConsolePage(trimmed)) {
    return trimmed
  }
  return 'home'
}

/** Type guard. */
function isConsolePage(value: string): value is ConsolePage {
  return (PAGES as readonly string[]).includes(value)
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
 *
 * P1-2 (2026-07-20): on each route change, focus is moved to the
 * `<main id="main-content" tabindex="-1">` element so screen-reader +
 * keyboard users hear the new page content announced. Without this, focus
 * stays on the clicked nav link and the new page is silent.
 */
export function onRouteChange(cb: (page: ConsolePage) => void): () => void {
  const handler = (): void => {
    cb(parseHash())
    // Move focus to the main content region so the new page is announced.
    // setTimeout(0) lets Svelte render the new page before focus moves.
    setTimeout(() => {
      const main = document.getElementById('main-content')
      if (main) main.focus()
    }, 0)
  }
  // Initial fire so the caller renders the current route without waiting
  // for the first hashchange event. No focus move on initial load — the
  // browser already focuses the document.
  cb(parseHash())
  window.addEventListener('hashchange', handler)
  return () => window.removeEventListener('hashchange', handler)
}
