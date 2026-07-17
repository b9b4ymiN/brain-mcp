/**
 * XSS-safe HTML escaping for Console rendering.
 *
 * Mirrors `src/console.rs::SafeText::escape` VERBATIM (see
 * `tests/console_contract_v1.rs`). The Console must use this (or DOM
 * `textContent`) — never raw `innerHTML` with untrusted content.
 *
 * One-time render concern: escape EXACTLY ONCE at render time. Re-escaping
 * an already-escaped string produces visible `&amp;lt;` corruption (the `&`
 * in `&lt;` gets escaped again). Callers must track whether a string is
 * already escaped and escape exactly once.
 */

/**
 * The five-entity escape table. Exported as a documented constant so tests
 * and downstream code can introspect the exact mapping — it MUST match the
 * Rust `SafeText::escape` match arms character-for-character.
 *
 *   &  →  &amp;
 *   <  →  &lt;
 *   >  →  &gt;
 *   "  →  &quot;
 *   '  →  &#x27;
 */
export const SAFE_TEXT_ENTITIES: Readonly<Record<string, string>> = Object.freeze({
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#x27;',
})

/**
 * Escape a raw string for safe DOM insertion. Call exactly once per render;
 * do NOT re-escape an already-escaped string.
 *
 * Implementation mirrors `SafeText::escape` in `src/console.rs`: a single
 * pass over the input, pushing either the entity (for the 5 special chars)
 * or the character verbatim. No sanitization, no DOMPurify — pure escape.
 */
export function escapeHtml(raw: string): string {
  let out = ''
  for (const c of raw) {
    const entity = SAFE_TEXT_ENTITIES[c]
    if (entity !== undefined) {
      out += entity
    } else {
      out += c
    }
  }
  return out
}
