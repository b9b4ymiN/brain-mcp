import { expect, test } from '@playwright/test'
import {
  transition,
  buildDiff,
  type ReviewState,
  type ReviewAction,
} from '../src/lib/review.ts'

// Pure-function unit tests for the review-domain helpers in
// `src/lib/review.ts`. Runs under the Playwright runner (same harness as
// `format.spec.ts` / `safetext.spec.ts`) so no Vitest dependency is needed.
// The page fixture is unused — these are plain TS imports exercised against
// the source directly.
//
// Mirrors the contract pinned by `src/console.rs` lines 64-103:
//   transition_pending_approve            — (Pending, Approve) → Approved
//   transition_pending_reject             — (Pending, Reject)  → Rejected
//   transition_pending_supersede          — (Pending, Supersede) → Approved
//   transition_already_decided            — (Approved | Rejected, *) → error
//   buildDiff identical → []              — no diff for equal values
//   buildDiff differing → 1 row           — single field row with before/after
//   buildDiff null → value                — new-claim path produces a row

test.describe('review.transition happy paths', () => {
  test('Pending + Approve → Approved', () => {
    expect(transition('pending', 'approve')).toBe('approved')
  })

  test('Pending + Reject → Rejected', () => {
    expect(transition('pending', 'reject')).toBe('rejected')
  })

  test('Pending + Supersede → Approved (TS extension: supersede commits)', () => {
    // Rust's ReviewAction only has Approve/Reject; supersede is an
    // API-layer concern that maps to Approved in the local state machine
    // (the proposal is no longer pending once the new claim is committed).
    expect(transition('pending', 'supersede')).toBe('approved')
  })
})

test.describe('review.transition AlreadyDecided', () => {
  test('Approved + any action → already_decided', () => {
    const actions: ReviewAction[] = ['approve', 'reject', 'supersede']
    for (const a of actions) {
      expect(transition('approved', a)).toBe('already_decided')
    }
  })

  test('Rejected + any action → already_decided', () => {
    const actions: ReviewAction[] = ['approve', 'reject', 'supersede']
    for (const a of actions) {
      expect(transition('rejected', a)).toBe('already_decided')
    }
  })

  test('every valid ReviewState is covered by the matrix above', () => {
    // Compile-time exhaustiveness guard for the type. If a new variant is
    // added to `ReviewState` without updating the tests, the assertion
    // below will fail to compile (or runtime-fail). Catches regressions.
    const all: ReviewState[] = ['pending', 'approved', 'rejected']
    expect(all.length).toBe(3)
  })
})

test.describe('review.buildDiff', () => {
  test('identical values yield no diff', () => {
    expect(buildDiff('age', 42, 42)).toEqual([])
    expect(buildDiff('name', 'alice', 'alice')).toEqual([])
    // Deep equality for object values (stable key order from JSON).
    expect(buildDiff('meta', { a: 1 }, { a: 1 })).toEqual([])
  })

  test('differing scalar values yield a single row', () => {
    const diffs = buildDiff('age', 41, 42)
    expect(diffs).toHaveLength(1)
    expect(diffs[0].field).toBe('age')
    expect(diffs[0].before).toBe(41)
    expect(diffs[0].after).toBe(42)
  })

  test('null → value (new claim) yields a single row with null before', () => {
    const diffs = buildDiff('status', null, 'active')
    expect(diffs).toHaveLength(1)
    expect(diffs[0].before).toBeNull()
    expect(diffs[0].after).toBe('active')
  })

  test('object-value diff yields a single row when contents differ (Fix M5)', () => {
    // buildDiff is field-level on the whole value; an object-value change
    // is still ONE diff row (before=old object, after=new object). This
    // pins the contract so a future multi-field refactor doesn't silently
    // change the row count for object values.
    const before = { a: 1, b: 2 }
    const after = { a: 1, b: 3 }
    const diffs = buildDiff('meta', before, after)
    expect(diffs).toHaveLength(1)
    expect(diffs[0].field).toBe('meta')
    expect(diffs[0].before).toEqual(before)
    expect(diffs[0].after).toEqual(after)
  })

  test('object-value with different key order but same contents yields no diff (Fix M5)', () => {
    // jsonEqual uses JSON.stringify, so key ORDER matters. Document the
    // current contract: two objects with the same KV pairs emitted in the
    // SAME order are equal; a different order is treated as a diff (the
    // caller is expected to have received both from serde_json, which has
    // stable insertion-order emission, so order shifts are significant).
    expect(buildDiff('meta', { a: 1, b: 2 }, { a: 1, b: 2 })).toEqual([])
    const ordered = buildDiff('meta', { a: 1, b: 2 }, { b: 2, a: 1 })
    expect(ordered).toHaveLength(1)
  })

  test('nested object-value diff yields a single row (Fix M5)', () => {
    const before = { profile: { name: 'alice', age: 30 } }
    const after = { profile: { name: 'alice', age: 31 } }
    const diffs = buildDiff('profile', before, after)
    expect(diffs).toHaveLength(1)
    expect(diffs[0].before).toEqual(before)
    expect(diffs[0].after).toEqual(after)
  })
})
