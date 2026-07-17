import { expect, test } from '@playwright/test'
import { formatValue, formatDate, truncate } from '../src/lib/format.ts'

// Pure-function unit tests for the display helpers in `src/lib/format.ts`.
// Runs under the Playwright runner (same harness as `safetext.spec.ts`) so
// no Vitest dependency is needed. The page fixture is unused — these are
// plain TS imports exercised against the source directly.
//
// Coverage matrix:
//   formatValue: number, string, boolean, null, undefined, object, array
//   formatDate:  null/undefined/empty, valid ISO, invalid ISO
//   truncate:    no-op (short input), exact-length, truncate-with-ellipsis,
//                edge (max <= 0)

test.describe('format.formatValue', () => {
  test('numbers render verbatim', () => {
    expect(formatValue(42)).toBe('42')
    expect(formatValue(0)).toBe('0')
    expect(formatValue(-3.14)).toBe('-3.14')
  })

  test('strings pass through unescaped (DOM escapes)', () => {
    // XSS payload stays literal here — the Svelte text binding escapes it at
    // render time. formatValue must NOT pre-escape (that would double-escape).
    expect(formatValue('<img src=x onerror=alert(1)>')).toBe('<img src=x onerror=alert(1)>')
    expect(formatValue('hello world')).toBe('hello world')
  })

  test('booleans render as their string form', () => {
    expect(formatValue(true)).toBe('true')
    expect(formatValue(false)).toBe('false')
  })

  test('null / undefined render as the em-dash placeholder', () => {
    expect(formatValue(null)).toBe('—')
    expect(formatValue(undefined)).toBe('—')
  })

  test('objects and arrays JSON-stringify', () => {
    expect(formatValue({ a: 1, b: 'x' })).toBe('{"a":1,"b":"x"}')
    expect(formatValue([1, 2, 3])).toBe('[1,2,3]')
  })
})

test.describe('format.formatDate', () => {
  test('null / undefined / empty render as the em-dash placeholder', () => {
    expect(formatDate(null)).toBe('—')
    expect(formatDate(undefined)).toBe('—')
    expect(formatDate('')).toBe('—')
  })

  test('valid ISO date yields a non-em-dash locale string', () => {
    const out = formatDate('2024-01-15T10:30:00Z')
    // We don't pin the exact locale string (timezone/locale-dependent) but
    // it must (a) not be the placeholder, (b) contain the year 2024.
    expect(out).not.toBe('—')
    expect(out).toContain('2024')
  })

  test('invalid date string falls back to placeholder, never "Invalid Date"', () => {
    expect(formatDate('not-a-date')).toBe('—')
  })
})

test.describe('format.truncate', () => {
  test('short input passes through unchanged (no ellipsis)', () => {
    expect(truncate('hello', 10)).toBe('hello')
    expect(truncate('hello', 5)).toBe('hello')
  })

  test('oversized input truncates with an ellipsis (total length === max)', () => {
    // `…` is a single code point (length 1) — so total length is `keep + 1`.
    // truncate('abcdef', 5) → keep = 4 → 'abcd' + '…' = 'abcd…' (length 5).
    expect(truncate('abcdef', 5)).toBe('abcd…')
    expect(truncate('abcdef', 5).length).toBe(5)
  })

  test('max === 1 yields just the ellipsis; max <= 0 yields empty string', () => {
    expect(truncate('abcdef', 1)).toBe('…')
    expect(truncate('abcdef', 0)).toBe('')
    expect(truncate('abcdef', -3)).toBe('')
  })
})
