<script lang="ts">
  /**
   * EntityDestructivePanel — entity-level destructive controls (Task E3.3 Part D).
   *
   * Lives inside `Entity.svelte` when an entity_id is present on the loaded
   * claims. Hosts the three entity-level destructive flows:
   *   - Merge: rewrite this entity's claims onto another entity (entity_merge).
   *   - Split: move per-predicate claims onto other entities (entity_split).
   *   - Retract (per claim): drop a single claim from current view
   *     (`claim_retract`) — REVERSIBLE via supersede, so it uses a plain
   *     confirm, NOT the destructive dialog.
   *
   * Merge + Split both go through `<DestructiveDialog>` because the server's
   * `DestructiveWarning::for_action(EntityMerge|EntitySplit)` is non-
   * irreversible + non-reauth + non-nonce — but we still SHOW the warning +
   * structured preview before commit, mirroring the hard-purge flow's
   * "warning visible before any confirm" invariant.
   *
   * The dialog is the SINGLE chokepoint: no path calls `entityMerge` /
   * `entitySplit` without first rendering the dialog. `claimRetract` has its
   * own tiny confirm (it's reversible per E3.2 design).
   *
   * Anti-XSS (Part F): every dynamic value (target id input, predicate,
   * warning message, preview items) is bound via Svelte text binding — no
   * `{@html}`. Server-supplied warning text renders as literal text.
   */
  import {
    destructiveWarning as apiDestructiveWarning,
    entityMerge as apiEntityMerge,
    entitySplit as apiEntitySplit,
    claimRetract as claimRetractLocal,
    ApiError,
    type DestructiveWarning,
    type DestructivePreviewItem,
  } from '../lib/api'
  import { tick } from 'svelte'
  import type { SessionStore } from '../lib/session.svelte'
  import type { ToastStore } from '../lib/toast.svelte'
  import DestructiveDialog from './DestructiveDialog.svelte'
  import { formatValue } from '../lib/format'

  /**
   * Minimal claim projection the panel needs from the parent. The full
   * `SubjectClaim` / `ClaimView` shapes carry far more than retract + split
   * care about; this projection keeps the panel decoupled from the page's
   * fetch shape AND lets us avoid fabricating fields we don't have.
   */
  export interface PanelClaim {
    claim_id: string
    predicate: string
    value: unknown
  }

  interface Props {
    /** Active session (used for flash banners on success/error). */
    session: SessionStore
    /** Toast store — for success/error toasts. */
    toasts: ToastStore
    /** The current entity's UUID — `source` for merge/split. */
    entityId: string
    /**
     * The current claims (used to enumerate predicates for split + retract).
     * The retract endpoint takes the claim's CONFIRM operation_id (NOT the
     * claim UUID) — the public timeline surface does not expose it, so the
     * retract dialog asks the user to paste it (the panel only displays the
     * claim_id as a hint).
     */
    claims: PanelClaim[]
    /** Fired after a successful merge/split/retract so the parent can refetch. */
    onMutated: () => void
  }

  let { session, toasts, entityId, claims, onMutated }: Props = $props()

  // ── Dialog state ─────────────────────────────────────────────────────────
  type Dialog =
    | {
        kind: 'merge'
        warning: DestructiveWarning
        preview: DestructivePreviewItem[]
        target: string
      }
    | {
        kind: 'split'
        warning: DestructiveWarning
        preview: DestructivePreviewItem[]
        assignments: { predicate: string; target_entity_id: string }[]
      }
    | null

  let dialog = $state<Dialog>(null)
  let acting = $state(false)

  // ── Retract confirm (separate, simpler — reversible) ─────────────────────
  // The API takes the claim's CONFIRM operation_id (the literal string the
  // proposer passed to `confirm`), which the public `/get` + `/timeline`
  // surfaces do NOT expose. The retract dialog therefore asks the user to
  // paste it (displaying the claim_id as a hint). This is a known Console
  // gap; the destructive-action test exercises the dialog UI rather than a
  // blind fire from claim_id.
  let retractTarget = $state<{ claim: PanelClaim; confirmOp: string } | null>(
    null,
  )
  let retracting = $state(false)
  let retractOpInput = $state<HTMLInputElement | null>(null)

  // When the retract dialog opens, focus the confirm-op input so keyboard
  // users land inside the dialog (and Escape — bound on the dialog root —
  // fires from the active element). Mirrors the focus-on-open pattern from
  // Inbox.svelte + DestructiveDialog.svelte.
  $effect(() => {
    if (retractTarget) void focusRetractInput()
  })

  async function focusRetractInput(): Promise<void> {
    await tick()
    retractOpInput?.focus()
  }

  // ── Merge form state ─────────────────────────────────────────────────────
  let mergeTarget = $state('')
  let mergeLoading = $state(false)

  // ── Split form state ─────────────────────────────────────────────────────
  let splitRows = $state<{ predicate: string; target_entity_id: string }[]>(
    [],
  )
  let splitLoading = $state(false)

  function distinctPredicates(): string[] {
    const seen = new Set<string>()
    const out: string[] = []
    for (const c of claims) {
      if (!seen.has(c.predicate)) {
        seen.add(c.predicate)
        out.push(c.predicate)
      }
    }
    return out
  }

  function addSplitRow(): void {
    splitRows = [...splitRows, { predicate: '', target_entity_id: '' }]
  }

  function removeSplitRow(idx: number): void {
    splitRows = splitRows.filter((_, i) => i !== idx)
  }

  // ── Merge flow ───────────────────────────────────────────────────────────
  async function startMerge(): Promise<void> {
    const target = mergeTarget.trim()
    if (!target) {
      toasts.push('error', 'Target required', 'Enter a target entity UUID.')
      return
    }
    mergeLoading = true
    try {
      const warning = await apiDestructiveWarning('entity_merge')
      const preview: DestructivePreviewItem[] = [
        {
          target_kind: 'entity',
          target_id: entityId,
          effect: `Claims rewritten onto target ${target}`,
        },
        {
          target_kind: 'entity',
          target_id: target,
          effect: 'Receives all claims from source',
        },
      ]
      dialog = { kind: 'merge', warning, preview, target }
    } catch (cause) {
      onDestructiveFetchError(cause)
    } finally {
      mergeLoading = false
    }
  }

  // ── Split flow ───────────────────────────────────────────────────────────
  async function startSplit(): Promise<void> {
    const cleaned = splitRows
      .map((r) => ({
        predicate: r.predicate.trim(),
        target_entity_id: r.target_entity_id.trim(),
      }))
      .filter((r) => r.predicate && r.target_entity_id)
    if (cleaned.length === 0) {
      toasts.push(
        'error',
        'Assignment rows required',
        'Add at least one predicate → target row before splitting.',
      )
      return
    }
    // Defensive: reject duplicate predicates client-side (server would 409).
    const predicates = new Set<string>()
    for (const row of cleaned) {
      if (predicates.has(row.predicate)) {
        toasts.push(
          'error',
          'Duplicate predicate',
          `Predicate "${row.predicate}" appears more than once — pick distinct predicates.`,
        )
        return
      }
      predicates.add(row.predicate)
    }
    splitLoading = true
    try {
      const warning = await apiDestructiveWarning('entity_split')
      const preview: DestructivePreviewItem[] = cleaned.map((r) => ({
        target_kind: 'claims',
        target_id: r.predicate,
        effect: `Moved onto entity ${r.target_entity_id}`,
      }))
      dialog = { kind: 'split', warning, preview, assignments: cleaned }
    } catch (cause) {
      onDestructiveFetchError(cause)
    } finally {
      splitLoading = false
    }
  }

  // ── Confirm — fires the actual mutation ──────────────────────────────────
  async function onConfirm(): Promise<void> {
    const d = dialog
    if (!d || acting) return
    acting = true
    try {
      if (d.kind === 'merge') {
        const result = await apiEntityMerge(entityId, d.target)
        toasts.push(
          'success',
          'Entity merged',
          `target ${d.target.slice(0, 8)}… · event_seq=${result.event_seq}`,
        )
      } else if (d.kind === 'split') {
        const result = await apiEntitySplit(entityId, d.assignments)
        toasts.push(
          'success',
          'Entity split',
          `moved ${result.moved_claim_count} claim(s), ${result.source_remaining_claim_count} remain · event_seq=${result.event_seq}`,
        )
      }
      dialog = null
      // Reset form state for the next round.
      mergeTarget = ''
      splitRows = []
      onMutated()
    } catch (cause) {
      onMutationError(cause)
    } finally {
      acting = false
    }
  }

  function cancelDialog(): void {
    if (acting) return
    dialog = null
  }

  // ── Retract flow (reversible — plain confirm) ────────────────────────────
  function startRetract(claim: PanelClaim): void {
    retractTarget = { claim, confirmOp: '' }
  }

  function cancelRetract(): void {
    if (retracting) return
    retractTarget = null
  }

  async function confirmRetract(): Promise<void> {
    const t = retractTarget
    if (!t || retracting) return
    const op = t.confirmOp.trim()
    if (!op) {
      toasts.push('error', 'Operation id required', 'Enter the claim confirm operation id.')
      return
    }
    retracting = true
    try {
      const result = await claimRetractLocal(op)
      toasts.push(
        'success',
        'Claim retracted',
        `claim ${t.claim.claim_id.slice(0, 8)}… · event_seq=${result.event_seq} · undo: supersede`,
      )
      retractTarget = null
      onMutated()
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) {
        session.clear()
        toasts.push('error', 'Session expired', 'Please sign in again.')
        retractTarget = null
        return
      }
      if (cause instanceof ApiError && cause.status === 404) {
        toasts.push(
          'error',
          'Operation id not found',
          'Confirm operation id not found — check it matches the claim.',
        )
      } else if (cause instanceof ApiError) {
        toasts.push('error', 'Retract failed', `Server returned: ${cause.code}`)
      } else {
        toasts.push('error', 'Retract failed', 'Is the backend running on :8080?')
      }
    } finally {
      retracting = false
    }
  }

  // ── Error helpers ────────────────────────────────────────────────────────
  function onDestructiveFetchError(cause: unknown): void {
    if (cause instanceof ApiError && cause.status === 401) {
      session.clear()
      toasts.push('error', 'Session expired', 'Please sign in again.')
      return
    }
    toasts.push(
      'error',
      'Warning unavailable',
      cause instanceof ApiError
        ? `Server returned: ${cause.code}`
        : 'Could not load destructive warning — is the backend running on :8080?',
    )
  }

  function onMutationError(cause: unknown): void {
    if (cause instanceof ApiError) {
      if (cause.status === 401) {
        session.clear()
        toasts.push('error', 'Session expired', 'Please sign in again.')
        dialog = null
        return
      }
      if (cause.status === 403) {
        session.clear()
        toasts.push(
          'error',
          'Session expired',
          'Session or CSRF token rejected. Please sign in again.',
        )
        dialog = null
        return
      }
      if (cause.status === 404) {
        toasts.push('error', 'Entity not found')
        dialog = null
        return
      }
      if (cause.status === 409) {
        toasts.push(
          'error',
          'Conflict',
          'Another client already mutated this entity concurrently.',
        )
        dialog = null
        return
      }
      toasts.push('error', 'Action failed', `Server returned: ${cause.code}`)
      return
    }
    toasts.push('error', 'Action failed', 'Is the backend running on :8080?')
  }
</script>

<section class="destructive-panel" aria-label="Destructive entity actions">
  <h3>Destructive actions</h3>

  <details class="block">
    <summary>Merge into another entity</summary>
    <p class="hint">
      Rewrites every claim on this entity onto the target. The source becomes a
      backlink. Warning shown before commit.
    </p>
    <div class="row">
      <label for="merge-target">Target entity UUID</label>
      <input
        id="merge-target"
        type="text"
        placeholder="uuid"
        bind:value={mergeTarget}
        disabled={mergeLoading}
      />
      <button type="button" onclick={startMerge} disabled={mergeLoading}>
        {mergeLoading ? 'Loading…' : 'Preview merge'}
      </button>
    </div>
  </details>

  <details class="block">
    <summary>Split predicates to other entities</summary>
    <p class="hint">
      Move each chosen predicate's claims onto a different entity. Predicates
      not listed stay here. Duplicate predicates are rejected client-side.
    </p>
    {#if splitRows.length === 0}
      <p class="empty">No assignment rows yet.</p>
    {:else}
      <ul class="split-rows">
        {#each splitRows as row, idx (idx)}
          <li class="split-row">
            <label for={`split-pred-${idx}`}>Predicate</label>
            <input
              id={`split-pred-${idx}`}
              type="text"
              list="split-pred-list"
              placeholder="predicate"
              bind:value={row.predicate}
              disabled={splitLoading}
            />
            <label for={`split-tgt-${idx}`}>Target entity UUID</label>
            <input
              id={`split-tgt-${idx}`}
              type="text"
              placeholder="uuid"
              bind:value={row.target_entity_id}
              disabled={splitLoading}
            />
            <button
              type="button"
              onclick={() => removeSplitRow(idx)}
              disabled={splitLoading}
              aria-label={`Remove assignment row ${idx + 1}`}
            >
              Remove
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <datalist id="split-pred-list">
      {#each distinctPredicates() as pred (pred)}
        <option value={pred}></option>
      {/each}
    </datalist>
    <div class="row split-actions">
      <button type="button" onclick={addSplitRow} disabled={splitLoading}>
        Add row
      </button>
      <button type="button" onclick={startSplit} disabled={splitLoading}>
        {splitLoading ? 'Loading…' : 'Preview split'}
      </button>
    </div>
  </details>

  <details class="block">
    <summary>Retract a claim (reversible)</summary>
    <p class="hint">
      Retracted claims stop being current. History + evidence are kept. Undo by
      superseding a new claim into scope. No destructive dialog — retract is
      reversible by design.
    </p>
    {#if claims.length === 0}
      <p class="empty">No claims to retract.</p>
    {:else}
      <ul class="retract-list">
        {#each claims as claim (claim.claim_id)}
          <li class="retract-row">
            <span class="retract-pred">{claim.predicate}</span>
            <span class="retract-value">{formatValue(claim.value)}</span>
            <button
              type="button"
              onclick={() => startRetract(claim)}
              disabled={retracting}
              aria-label={`Retract claim ${claim.claim_id}`}
            >
              Retract
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </details>
</section>

{#if dialog}
  <DestructiveDialog
    warning={dialog.warning}
    preview={dialog.preview}
    requiresReauth={dialog.warning.requires_recent_reauth}
    onConfirm={onConfirm}
    onCancel={cancelDialog}
    {acting}
  />
{/if}

{#if retractTarget}
  <div
    class="dialog-backdrop"
    role="presentation"
    onclick={cancelRetract}
    onkeydown={(e) => {
      if (e.key === 'Escape') cancelRetract()
    }}
  ></div>
  <div
    class="dialog retract-dialog"
    role="dialog"
    aria-modal="true"
    aria-label="Confirm retract"
    aria-describedby="retract-summary"
    tabindex="-1"
    onkeydown={(e) => {
      if (e.key === 'Escape' && !retracting) cancelRetract()
    }}
  >
    <h2>Confirm retract?</h2>
    <p class="retract-summary" id="retract-summary">
      Predicate <span class="mono">{retractTarget.claim.predicate}</span>
      · Value <span class="mono">{formatValue(retractTarget.claim.value)}</span>
    </p>
    <p class="retract-summary meta">
      Claim id <span class="mono">{retractTarget.claim.claim_id}</span>
    </p>
    <div class="retract-form">
      <label for="retract-op">Confirm operation id</label>
      <input
        id="retract-op"
        type="text"
        placeholder="e.g. p1-confirm"
        bind:value={retractTarget.confirmOp}
        disabled={retracting}
        aria-describedby="retract-op-hint"
        bind:this={retractOpInput}
      />
      <p class="hint" id="retract-op-hint">
        The API retracts by the claim's confirm operation_id, which the public
        timeline surface does not expose. Paste it here (e.g. from the
        proposer's record).
      </p>
    </div>
    <p class="dialog-prompt">
      Retracting a claim is reversible — supersede a new claim to undo. Continue?
    </p>
    <div class="dialog-actions">
      <button
        type="button"
        class="action action-yes"
        disabled={retracting || retractTarget.confirmOp.trim().length === 0}
        onclick={confirmRetract}
      >
        {retracting ? 'Working…' : 'Yes, retract'}
      </button>
      <button
        type="button"
        class="action action-no"
        disabled={retracting}
        onclick={cancelRetract}
      >
        No, cancel
      </button>
    </div>
  </div>
{/if}

<style>
  .destructive-panel {
    margin: 1rem 0;
    padding: 0.85rem 1rem;
    border-radius: 0.5rem;
    border: 1px solid rgba(190, 70, 70, 0.45);
    background: rgba(190, 70, 70, 0.05);
  }

  .destructive-panel h3 {
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.85;
  }

  .block {
    margin: 0.4rem 0;
    padding: 0.5rem 0.6rem;
    border-radius: 0.4rem;
    background: rgba(127, 127, 127, 0.05);
  }

  .block summary {
    cursor: pointer;
    font-weight: 600;
    font-size: 0.9rem;
  }

  .hint {
    margin: 0.35rem 0 0.6rem;
    font-size: 0.82rem;
    opacity: 0.7;
  }

  .empty {
    margin: 0.4rem 0;
    font-size: 0.85rem;
    opacity: 0.65;
    font-style: italic;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
  }

  .split-actions {
    margin-top: 0.4rem;
  }

  label {
    font-size: 0.8rem;
    opacity: 0.8;
  }

  input {
    padding: 0.4rem 0.55rem;
    border-radius: 0.3rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
    min-width: 12rem;
  }

  button {
    padding: 0.4rem 0.75rem;
    border-radius: 0.3rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  button:hover:not(:disabled),
  button:focus-visible:not(:disabled) {
    background: rgba(127, 127, 127, 0.25);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .split-rows {
    list-style: none;
    margin: 0.4rem 0;
    padding: 0;
    display: grid;
    gap: 0.3rem;
  }

  .split-row {
    display: grid;
    grid-template-columns: auto 1fr auto 1fr auto;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
  }

  .retract-list {
    list-style: none;
    margin: 0.4rem 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }

  .retract-row {
    display: grid;
    grid-template-columns: minmax(8rem, auto) 1fr auto;
    gap: 0.5rem;
    padding: 0.3rem 0.4rem;
    border-radius: 0.3rem;
    background: rgba(127, 127, 127, 0.06);
    font-size: 0.85rem;
    align-items: center;
  }

  .retract-pred {
    font-weight: 600;
  }

  .retract-value {
    opacity: 0.8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Retract dialog (mirrors Inbox.svelte dialog chrome) ─────────────── */
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 50;
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    min-width: 26rem;
    max-width: min(40rem, 92vw);
    padding: 1.25rem 1.25rem 1rem;
    border-radius: 0.6rem;
    border: 1px solid rgba(127, 127, 127, 0.5);
    background: var(--console-bg, #fff);
    color: var(--console-fg, #111);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
  }

  .dialog h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }

  .retract-summary {
    margin: 0 0 0.75rem;
    font-size: 0.9rem;
    opacity: 0.8;
    word-break: break-word;
  }

  .dialog-prompt {
    margin: 0.75rem 0 0.5rem;
    font-size: 0.9rem;
  }

  .retract-form {
    margin: 0.5rem 0;
    display: grid;
    gap: 0.3rem;
  }

  .retract-form input {
    width: 100%;
    padding: 0.45rem 0.55rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.5);
    background: inherit;
    color: inherit;
    font: inherit;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .meta {
    font-size: 0.82rem;
    opacity: 0.7;
  }

  .hint {
    margin: 0.2rem 0 0;
    font-size: 0.8rem;
    opacity: 0.65;
    font-style: italic;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  .action {
    padding: 0.45rem 0.875rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.1);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .action-yes {
    background: rgba(190, 130, 70, 0.25);
    border-color: rgba(190, 130, 70, 0.55);
    font-weight: 600;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.85em;
  }
</style>
