/**
 * Pure review-domain helpers for the Console Inbox (Task E1.3).
 *
 * Mirrors `src/console.rs` (lines 36-103):
 *   - `ReviewState`    — Pending | Approved | Rejected (serde snake_case).
 *   - `ReviewAction`   — Approve | Reject on the Rust side. The TS side
 *                        adds a `Supersede` variant that the Console maps
 *                        to the supersede API endpoint; in the state
 *                        machine it resolves to `Approved` (supersede
 *                        confirms a new claim that replaces prior ones).
 *   - `transition`     — the state machine `ReviewState::transition`
 *                        (lines 69-81). Pending→Approve=Approved,
 *                        Pending→Reject=Rejected, Pending→Supersede=
 *                        Approved; AlreadyDecided→anything = error.
 *   - `DiffPreview` + `buildDiff` — before/after capture so the Console
 *                        renders the diff BEFORE commit (§9.1 "แสดง
 *                        evidence/diff ก่อน commit").
 *
 * Pure functions, no Svelte, no DOM, no `any`. Fully unit-testable in
 * isolation. Deep-equality uses JSON.stringify (the wire shape is JSON), so
 * key-order matters for object equality — callers that received both values
 * from the API can rely on serde_json's stable insertion-order emission.
 */

/** A review item's lifecycle state (mirrors Rust `ReviewState`). */
export type ReviewState = 'pending' | 'approved' | 'rejected'

/**
 * A user action on a review item.
 *
 * Rust's `ReviewAction` only has `Approve | Reject`; `supersede` lives at
 * the API layer (its own endpoint). The Console treats `Supersede` as a
 * third review action that resolves to `Approved` in the local state
 * machine (supersede commits a new claim that replaces the listed priors).
 */
export type ReviewAction = 'approve' | 'reject' | 'supersede'

/**
 * State-machine rejection reason (mirrors `ReviewTransitionError`).
 *
 *   - `already_decided` — the item was already Approved or Rejected; a
 *     second decision is forbidden (§4 rule 1: no silent overwrite).
 *   - `invalid_action`  — reserved for future actions the machine does not
 *     recognise. Currently unreachable from `transition` (TS exhaustiveness
 *     would catch a new variant at compile time), but kept for parity with
 *     the Rust enum so the wire shape round-trips.
 */
export type TransitionError = 'already_decided' | 'invalid_action'

/**
 * One before/after field diff (mirrors `src/console.rs::DiffPreview`).
 *
 * `before` / `after` are arbitrary JSON values from the wire. Callers
 * render them with `formatValue` (text binding — DOM escapes).
 */
export interface DiffPreview {
  field: string
  before: unknown
  after: unknown
}

/**
 * Apply a review action, returning the new state or an error code.
 *
 * Mirrors `ReviewState::transition` in `src/console.rs:69-81`:
 *   - (Pending, Approve)   → Approved
 *   - (Pending, Reject)    → Rejected
 *   - (Pending, Supersede) → Approved  (TS extension: supersede commits)
 *   - (Approved | Rejected, *) → AlreadyDecided
 *
 * The Rust enum only carries Approve/Reject; supersede maps to Approved
 * because the supersede API endpoint commits a new claim (its response is
 * `status: "superseded"` server-side, but the proposal's lifecycle from
 * the inbox's perspective is "confirmed → no longer pending").
 */
export function transition(
  from: ReviewState,
  action: ReviewAction,
): ReviewState | TransitionError {
  if (from !== 'pending') return 'already_decided'
  switch (action) {
    case 'approve':
      return 'approved'
    case 'reject':
      return 'rejected'
    case 'supersede':
      return 'approved'
    default: {
      // Exhaustiveness guard — if a new ReviewAction variant is added
      // without updating this switch, TS narrows `action` to `never` here
      // and compile fails. Runtime fallback returns the wire-stable code.
      const _exhaustive: never = action
      void _exhaustive
      return 'invalid_action'
    }
  }
}

/**
 * Stable deep-equality for JSON values. Uses `JSON.stringify` because the
 * wire shape IS JSON — serde_json emits object keys in insertion order, so
 * two server-emitted values that "should be equal" produce identical
 * strings. A throw (circular ref, BigInt) means the value is not a plain
 * JSON value, so we fall back to reference equality (rare; never happens
 * for wire data).
 */
function jsonEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  try {
    return JSON.stringify(a) === JSON.stringify(b)
  } catch {
    return false
  }
}

/**
 * Build a list of field-level diffs between a current value and a proposed
 * value, mirroring the role of `DiffPreview` in `src/console.rs:98-103`.
 *
 * Current scope (E1.3): single-field diff on the claim `value`, with
 * `field` set to the proposal's `predicate` (the natural label for "what
 * about the subject is changing"). If `currentValue` and `proposedValue`
 * are deep-equal, returns an empty array (no diff → nothing to render,
 * caller shows a "no changes" note). Future tasks may extend this to
 * multi-field claim shapes.
 *
 * `before = null` is the "no prior claim" signal from the caller
 * (proposal introduces a brand-new claim) — that is NOT equal to any
 * proposed JSON value, so a non-empty diff is returned showing
 * `null → proposedValue`.
 */
export function buildDiff(
  predicate: string,
  currentValue: unknown,
  proposedValue: unknown,
): DiffPreview[] {
  if (jsonEqual(currentValue, proposedValue)) return []
  return [
    {
      field: predicate,
      before: currentValue,
      after: proposedValue,
    },
  ]
}
