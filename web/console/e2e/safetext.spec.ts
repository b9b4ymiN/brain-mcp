import { expect, test } from '@playwright/test'
import { escapeHtml, SAFE_TEXT_ENTITIES } from '../src/lib/safeText.ts'

// Ports `tests/console_contract_v1.rs` (lines 80-108) — the four SafeText
// contract cases — to TS. Runs as a Playwright test (zero new deps: Vitest
// is not installed and the task forbids adding it). These are pure-function
// assertions; the page fixture is unused, but Playwright's runner gives us
// the same `expect` + parallelism story the rest of the suite uses.
//
// The reference Rust test asserts:
//   safe_text_escapes_html_entities:    `<script>alert('xss')</script>` must
//     contain `&lt;script&gt;` and must NOT contain `<script>` or `'xss'`.
//   safe_text_escape_is_one_time_only:  `<b>bold</b>` → `&lt;b&gt;bold&lt;/b>`;
//     double-escaping changes the string (visible `&amp;lt;` corruption).
// Plus the two `escape_basics` cases: `<b>` → `&lt;b&gt;`,
// `"hi"` → `&quot;hi&quot;`.

test.describe('SafeText.escapeHtml', () => {
  test('entity table matches the Rust contract verbatim', () => {
    expect(SAFE_TEXT_ENTITIES['&']).toBe('&amp;')
    expect(SAFE_TEXT_ENTITIES['<']).toBe('&lt;')
    expect(SAFE_TEXT_ENTITIES['>']).toBe('&gt;')
    expect(SAFE_TEXT_ENTITIES['"']).toBe('&quot;')
    expect(SAFE_TEXT_ENTITIES["'"]).toBe('&#x27;')
  })

  test('script-tag payload is fully neutralized', () => {
    const raw = `<script>alert('xss')</script>`
    const safe = escapeHtml(raw)
    expect(safe).toContain('&lt;script&gt;')
    expect(safe).not.toContain('<script>')
    expect(safe).not.toContain("'xss'")
  })

  test('basic angle-bracket escape: <b> -> &lt;b&gt;', () => {
    expect(escapeHtml('<b>')).toBe('&lt;b&gt;')
  })

  test('basic double-quote escape: "hi" -> &quot;hi&quot;', () => {
    expect(escapeHtml('"hi"')).toBe('&quot;hi&quot;')
  })

  test('<b>bold</b> escapes to &lt;b&gt;bold&lt;/b&gt;', () => {
    expect(escapeHtml('<b>bold</b>')).toBe('&lt;b&gt;bold&lt;/b&gt;')
  })

  test('escaping is one-time-only: double-escape changes the string', () => {
    const raw = '<b>bold</b>'
    const once = escapeHtml(raw)
    expect(once).toBe('&lt;b&gt;bold&lt;/b&gt;')

    const twice = escapeHtml(once)
    // Double-escaping IS visible corruption — the `&` in `&lt;` gets
    // re-escaped to `&amp;lt;`. Callers must escape exactly once.
    expect(twice).not.toBe(once)
    expect(twice).toContain('&amp;lt;')
  })

  test('plain text passes through unchanged', () => {
    expect(escapeHtml('hello world')).toBe('hello world')
    expect(escapeHtml('')).toBe('')
  })
})
