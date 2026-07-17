/**
 * Pure display-formatting helpers for the Console.
 *
 * These return plain strings — NO HTML, NO escaping. The DOM boundary
 * (Svelte's `{value}` text binding) is responsible for escaping; pre-escaping
 * here would double-escape at render time (`&amp;lt;` corruption). See
 * `safeText.ts` for the one-time-escape invariant.
 *
 * All functions are total: every input produces a displayable string, never
 * throws. Pure and dependency-free, so they unit-test cleanly in isolation.
 */

/**
 * Render an arbitrary JSON value as a human-readable string.
 *
 *   - `null` / `undefined` → `'—'` (em-dash placeholder, same convention the
 *     Rust Console uses for empty fields).
 *   - numbers / booleans → `String(value)` verbatim (no locale coercion —
 *     callers that want thousands separators can wrap further).
 *   - strings → the string itself, unescaped (escaping is the DOM's job).
 *   - objects / arrays → `JSON.stringify` for a stable round-trippable shape.
 *
 * Returns the placeholder `'—'` for `undefined` (not the literal `"undefined"`)
 * so missing server fields read as "no value" rather than a JS artifact.
 */
export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return '—'
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  // Objects + arrays: stable JSON form. `JSON.stringify` is total for
  // JSON-shaped values (what the wire gives us); worst case it returns
  // `undefined` for functions/symbols, which we fold to the placeholder.
  const json = JSON.stringify(value)
  return json === undefined ? '—' : json
}

/**
 * Render an ISO 8601 timestamp as a locale string, or `'—'` when absent.
 *
 * Accepts the `null`/`undefined`/empty-string shapes the API emits for
 * optional date fields (`valid_from`, `valid_to`, `submitted_at`, etc.).
 * Invalid date strings produce `'—'` too (rather than `'Invalid Date'`) so
 * the UI never shows a confusing JS artifact.
 */
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '—'
  return d.toLocaleString()
}

/**
 * Truncate `text` to `max` visible characters, appending an ellipsis when
 * truncation occurs. `max` is the maximum length of the *returned* string
 * (including the ellipsis), so e.g. `truncate('abcdef', 5)` → `'ab…'`.
 *
 * `max <= 0` returns an empty string. `max >= text.length` returns the input
 * unchanged (no ellipsis). No HTML, no escaping.
 */
export function truncate(text: string, max: number): string {
  if (max <= 0) return ''
  if (text.length <= max) return text
  // Reserve one char for the ellipsis. Guard the slice length to stay >= 0
  // even when `max === 1` (yields just `'…'`).
  const keep = Math.max(0, max - 1)
  return `${text.slice(0, keep)}…`
}
