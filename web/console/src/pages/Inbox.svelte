<script lang="ts">
  /**
   * Inbox — pending proposal review workflow (Task E1.3).
   *
   * Lifecycle of a proposal:
   *   Pending ──Approve──▶ Approved   (POST /inbox/{id}/approve)
   *   Pending ──Reject───▶ Rejected   (POST /inbox/{id}/reject)
   *   Pending ──Supersede▶ Approved   (POST /inbox/{id}/supersede —
   *                                    commits a new claim that replaces the
   *                                    selected prior claims)
   *   Pending ──Edit─────▶ Approved   (supersede with a user-edited value)
   *
   * Once decided, the row is locked (the local `review.transition()` guard
   * returns `already_decided`). The Rust `transition` in `src/console.rs`
   * only models Approve/Reject; the TS adapter in `lib/review.ts` adds
   * `supersede` resolving to `approved` (the proposal is no longer pending
   * once a new claim is committed).
   *
   * Diff-before-commit invariant (DoD #8): every mutation passes through
   * `openDialog()`, and the dialog renders `<DiffPreview>` BEFORE the user
   * clicks Yes. The mutation API call only fires from `confirmDialog()`.
   * No blind applies.
   *
   * 4-state coverage via `<StateBox>` for the list:
   *   - loading    → `listLoading=true` while `inbox()` is in flight.
   *   - error      → any non-401/403/404 thrown; rendered as state-error.
   *   - empty      → inbox returned `[]` OR every visible proposal has been
   *                  locally decided (rare; the refetch after an action
   *                  usually clears the row server-side too).
   *   - permission → 401 from any call → `session.clear()` + banner;
   *                  App.svelte re-renders the login form.
   *
   * Distinct error handling:
   *   - 401 → session-expired (session.clear + flash).
   *   - 403 → CSRF failure (treated as session-expired: the cookie is dead
   *     even if the page looks logged-in; clear session and prompt re-login).
   *   - 404 → proposal already decided / not found (flash error, refetch
   *     inbox so the row disappears — do NOT clear session).
   *
   * Anti-XSS (DoD #4): every value from the API (proposal subject /
   * predicate / value, evidence excerpt, claim values, flash text, error
   * codes) is bound as TEXT via Svelte's `{value}` syntax, which
   * auto-escapes. NO raw-HTML bindings anywhere in this file.
   * `formatValue` returns plain string and callers bind it as text — no
   * double-escape.
   */
  import { onMount, tick } from 'svelte'
  import {
    inbox as apiInbox,
    evidence as apiEvidence,
    approve as apiApprove,
    reject as apiReject,
    supersede as apiSupersede,
    timeline as apiTimeline,
    ApiError,
    type ProposalSummary,
    type EvidenceSummary,
    type ClaimView,
    type Uuid,
  } from '../lib/api'
  import type { SessionStore } from '../lib/session.svelte'
  import {
    transition,
    buildDiff,
    type ReviewState,
    type ReviewAction,
    type DiffPreview,
  } from '../lib/review'
  import StateBox from '../components/StateBox.svelte'
  import DiffPreviewCmp from '../components/DiffPreview.svelte'
  import { formatValue, formatDate } from '../lib/format'

  interface Props {
    session: SessionStore
  }

  let { session }: Props = $props()

  // ── Inbox list state ───────────────────────────────────────────────────
  let proposals = $state<ProposalSummary[]>([])
  let listLoading = $state(false)
  let listError = $state<string | null>(null)
  let sessionExpired = $state(false)
  let hasLoaded = $state(false)

  /**
   * Local review state per proposal_id. The server returns only `proposed`
   * proposals in `/inbox`, so this map starts at `pending` for every row
   * we render. It flips to `approved` / `rejected` after a successful
   * action so the UI can immediately reflect the decision without waiting
   * for the inbox refetch (and so a second click is a no-op).
   */
  let reviewStates = $state<Record<Uuid, ReviewState>>({})

  // ── Per-proposal detail panel state ────────────────────────────────────
  type DetailState =
    | { kind: 'idle' }
    | { kind: 'loading' }
    | { kind: 'error'; message: string }
    | {
        kind: 'ready'
        evidence: EvidenceSummary
        currentValue: unknown
        currentClaims: ClaimView[]
      }

  let openId = $state<Uuid | null>(null)
  let details = $state<Record<Uuid, DetailState>>({})

  // Monotonic request-id guard for the inbox refetch + the detail-panel
  // fetch. The inbox one prevents a stale list refresh from clobbering a
  // newer one; the detail one prevents a stale detail fetch (from a
  // collapsed-then-reopened proposal) from overwriting the current panel.
  let listSeq = 0
  let detailSeq = 0

  // ── Confirm dialog state ───────────────────────────────────────────────
  /**
   * The dialog is the single chokepoint for every mutation. While open,
   * the action button click is the ONLY thing that opens it, and the Yes
   * button is the ONLY thing that calls the mutation API. No path calls
   * approve/reject/supersede without first rendering the dialog (DoD #8).
   */
  type Dialog =
    | {
        kind: 'approve' | 'reject'
        proposalId: Uuid
        subject: string
        predicate: string
        diffs: DiffPreview[]
      }
    | {
        kind: 'supersede'
        proposalId: Uuid
        subject: string
        predicate: string
        diffs: DiffPreview[]
        // All current confirmed claims in scope, with the user's checkbox
        // selection state. Supersede is multi-select: reviewer picks which
        // priors to replace.
        claims: { claim_id: Uuid; value: unknown; selected: boolean }[]
      }
    | {
        kind: 'edit'
        proposalId: Uuid
        subject: string
        predicate: string
        // The user-edited value. Diff is computed live from this.
        editedValue: string
        currentValue: unknown
        // All current confirmed claims in scope — Edit = supersede-with-
        // edited-value, so every confirmed prior in scope is superseded.
        claimIds: Uuid[]
      }

  let dialog = $state<Dialog | null>(null)
  // `true` while a mutation API call is in flight — disables Yes/No so the
  // user can't double-submit. Distinct from list/detail loading.
  let acting = $state(false)
  // Ref to the Yes button so we can focus it when the dialog opens.
  let yesBtn = $state<HTMLButtonElement | null>(null)

  onMount(() => {
    void refreshList()
  })

  // ── Inbox list ─────────────────────────────────────────────────────────
  async function refreshList(): Promise<void> {
    const seq = ++listSeq
    listLoading = true
    listError = null
    sessionExpired = false
    hasLoaded = true
    try {
      const result = await apiInbox()
      if (seq !== listSeq) return
      proposals = result
      // Seed local review state at `pending` for any new proposal id; leave
      // already-decided ones untouched (they shouldn't reappear, but if they
      // do, we keep the prior decision rather than resurrecting the row).
      const next: Record<Uuid, ReviewState> = {}
      for (const p of result) {
        next[p.proposal_id] = reviewStates[p.proposal_id] ?? 'pending'
      }
      reviewStates = next
    } catch (cause) {
      if (seq !== listSeq) return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      proposals = []
      listError =
        cause instanceof ApiError
          ? `Failed to load inbox (${cause.code}).`
          : 'Failed to load inbox — is the backend running on :8080?'
    } finally {
      if (seq === listSeq) listLoading = false
    }
  }

  // ── Detail panel ───────────────────────────────────────────────────────
  async function openDetail(proposal: ProposalSummary): Promise<void> {
    // Toggle behaviour: clicking the open proposal again closes it.
    if (openId === proposal.proposal_id) {
      openId = null
      return
    }
    openId = proposal.proposal_id
    // Already-loaded detail (e.g. user collapsed then re-opened) — reuse it.
    const existing = details[proposal.proposal_id]
    if (existing && (existing.kind === 'ready' || existing.kind === 'error')) {
      return
    }
    details = { ...details, [proposal.proposal_id]: { kind: 'loading' } }
    const seq = ++detailSeq
    try {
      // Fire both reads in parallel — they're independent. The detail panel
      // needs the evidence excerpt AND the current confirmed claim(s) in
      // scope so we can render before → after.
      const [evidenceResult, timelineResult] = await Promise.all([
        apiEvidence(proposal.proposal_id),
        apiTimeline({
          domain: proposal.domain,
          subject: proposal.subject,
          predicate: proposal.predicate,
        }),
      ])
      if (seq !== detailSeq) return
      // `claim_timeline` returns every claim that has ever been confirmed
      // in this scope (including later-superseded ones). For the diff's
      // "before" we want the most recent STILL-CONFIRMED claim (status
      // === 'confirmed'), breaking ties by the highest confirmed seq. If
      // there are none, the proposal introduces a brand-new claim and
      // `before` is null.
      const confirmed = timelineResult
        .filter((c) => c.status === 'confirmed')
        .sort((a, b) => b.confirmed_event_seq - a.confirmed_event_seq)
      const currentValue = confirmed.length > 0 ? confirmed[0].value : null
      details = {
        ...details,
        [proposal.proposal_id]: {
          kind: 'ready',
          evidence: evidenceResult,
          currentValue,
          currentClaims: confirmed,
        },
      }
    } catch (cause) {
      if (seq !== detailSeq) return
      if (cause instanceof ApiError && cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        return
      }
      const message =
        cause instanceof ApiError
          ? `Failed to load evidence (${cause.code}).`
          : 'Failed to load evidence — is the backend running on :8080?'
      details = { ...details, [proposal.proposal_id]: { kind: 'error', message } }
    }
  }

  function onRowKeydown(event: KeyboardEvent, proposal: ProposalSummary): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      void openDetail(proposal)
    }
  }

  // ── Diff construction ──────────────────────────────────────────────────
  function diffFor(proposal: ProposalSummary, before: unknown): DiffPreview[] {
    return buildDiff(proposal.predicate, before, proposal.value)
  }

  // ── Dialog openers (one per action) ────────────────────────────────────
  function startApprove(proposal: ProposalSummary, before: unknown): void {
    dialog = {
      kind: 'approve',
      proposalId: proposal.proposal_id,
      subject: proposal.subject,
      predicate: proposal.predicate,
      diffs: diffFor(proposal, before),
    }
    void focusYes()
  }

  function startReject(proposal: ProposalSummary, before: unknown): void {
    dialog = {
      kind: 'reject',
      proposalId: proposal.proposal_id,
      subject: proposal.subject,
      predicate: proposal.predicate,
      diffs: diffFor(proposal, before),
    }
    void focusYes()
  }

  function startSupersede(
    proposal: ProposalSummary,
    before: unknown,
    claims: ClaimView[],
  ): void {
    dialog = {
      kind: 'supersede',
      proposalId: proposal.proposal_id,
      subject: proposal.subject,
      predicate: proposal.predicate,
      diffs: diffFor(proposal, before),
      // Default: every confirmed prior in scope is selected. Reviewer can
      // uncheck individual ones before confirming.
      claims: claims.map((c) => ({
        claim_id: c.claim_id,
        value: c.value,
        selected: true,
      })),
    }
    void focusYes()
  }

  function startEdit(
    proposal: ProposalSummary,
    before: unknown,
    claims: ClaimView[],
  ): void {
    dialog = {
      kind: 'edit',
      proposalId: proposal.proposal_id,
      subject: proposal.subject,
      predicate: proposal.predicate,
      editedValue:
        typeof proposal.value === 'string' ? proposal.value : JSON.stringify(proposal.value),
      currentValue: before,
      claimIds: claims.map((c) => c.claim_id),
    }
    void focusYes()
  }

  async function focusYes(): Promise<void> {
    // Wait for the DOM to render the Yes button, then focus it so keyboard
    // users can confirm with Enter immediately (DoD: dialog traps focus).
    await tick()
    yesBtn?.focus()
  }

  function cancelDialog(): void {
    if (acting) return
    dialog = null
  }

  // ── Live diff for the Edit dialog as the user types ───────────────────
  let editDiffs = $derived(
    dialog && dialog.kind === 'edit'
      ? buildDiff(dialog.predicate, dialog.currentValue, parseEditedValue(dialog.editedValue))
      : [],
  )

  /**
   * Parse the user-typed Edit value. Strings stay strings; anything that
   * looks like JSON ({...}, [...], number, true/false/null) is parsed so
   * the diff reflects the actual JSON value that would be committed. A
   * parse failure falls back to the raw string — the reviewer typed it,
   * we honour it.
   */
  function parseEditedValue(raw: string): unknown {
    const trimmed = raw.trim()
    if (trimmed === '') return null
    // Plain string with no JSON-ish first char → keep as string (the common
    // case: most claim values are short text).
    const c0 = trimmed[0]
    if (c0 !== '{' && c0 !== '[' && c0 !== '"' && c0 !== 't' && c0 !== 'f' && c0 !== 'n') {
      // Numeric? parse as number; else treat as literal string.
      const num = Number(trimmed)
      return trimmed !== '' && !Number.isNaN(num) && /^-?\d/.test(trimmed) ? num : raw
    }
    try {
      return JSON.parse(trimmed)
    } catch {
      return raw
    }
  }

  // ── Mutation execution (only reachable from the dialog's Yes button) ───
  async function confirmDialog(): Promise<void> {
    const d = dialog
    if (!d || acting) return
    // Snapshot the request plan BEFORE any `await`. TypeScript narrowing on
    // a `$state` proxy is invalidated by `await` (the proxy can be mutated
    // externally), so we extract every field we need up front into a
    // discriminated `plan` and only then start the request. The narrowing
    // below uses an explicit `kind === 'edit'` arm (not a bare `else`) so
    // the union collapses correctly to the Edit variant.
    type Plan =
      | { action: 'approve'; proposalId: Uuid }
      | { action: 'reject'; proposalId: Uuid }
      | { action: 'supersede'; proposalId: Uuid; ids: Uuid[]; label: 'Superseded' }
      | { action: 'supersede'; proposalId: Uuid; ids: Uuid[]; label: 'Edited' }
    let plan: Plan
    if (d.kind === 'approve') {
      plan = { action: 'approve', proposalId: d.proposalId }
    } else if (d.kind === 'reject') {
      plan = { action: 'reject', proposalId: d.proposalId }
    } else if (d.kind === 'supersede') {
      plan = {
        action: 'supersede',
        proposalId: d.proposalId,
        ids: d.claims.filter((c) => c.selected).map((c) => c.claim_id),
        label: 'Superseded',
      }
    } else if (d.kind === 'edit') {
      // Edit — supersede with the (proposal's) value, replacing every
      // current confirmed claim in scope. The server commits the proposal's
      // value via the proposal pipeline; Edit here is the user choosing to
      // replace priors using this proposal.
      plan = {
        action: 'supersede',
        proposalId: d.proposalId,
        ids: d.claimIds,
        label: 'Edited',
      }
    } else {
      // Unreachable — exhaustive over the Dialog union. Defensive: refuse
      // to mutate if the dialog shape is unknown.
      acting = false
      dialog = null
      return
    }
    acting = true
    try {
      let eventSeq: number
      if (plan.action === 'approve') {
        eventSeq = (await apiApprove(plan.proposalId)).event_seq
        finalize(plan.proposalId, 'approve', `Approved proposal ${short(plan.proposalId)} (event_seq=${eventSeq}).`)
      } else if (plan.action === 'reject') {
        eventSeq = (await apiReject(plan.proposalId)).event_seq
        finalize(plan.proposalId, 'reject', `Rejected proposal ${short(plan.proposalId)} (event_seq=${eventSeq}).`)
      } else {
        eventSeq = (await apiSupersede(plan.proposalId, plan.ids)).event_seq
        finalize(plan.proposalId, 'supersede', `${plan.label} proposal ${short(plan.proposalId)} (event_seq=${eventSeq}).`)
      }
    } catch (cause) {
      onMutationError(cause, plan.proposalId)
    } finally {
      acting = false
    }
  }

  /**
   * Apply the post-success common path: flip local review state via the
   * state machine (defensive — should never return an error since the row
   * was pending), push a success flash, refetch the inbox, close the
   * dialog, and collapse the detail panel.
   */
  function finalize(
    proposalId: Uuid,
    action: ReviewAction,
    message: string,
  ): void {
    const prior = reviewStates[proposalId] ?? 'pending'
    const next = transition(prior, action)
    // `transition` returns `ReviewState | TransitionError`. Treat the
    // terminal-state case explicitly so TS narrows correctly. (Reject →
    // 'rejected'; approve/supersede → 'approved'.) Defensive: the server
    // already confirmed success, so even an unexpected local-state drift
    // gets locked here.
    const resolved: ReviewState =
      next === 'approved' || next === 'rejected'
        ? next
        : action === 'reject'
          ? 'rejected'
          : 'approved'
    reviewStates = { ...reviewStates, [proposalId]: resolved }
    session.pushFlash('success', message)
    dialog = null
    openId = null
    void refreshList()
  }

  /**
   * Map mutation errors to the distinct UX branches (DoD #10):
   *   401 → session expired (clear session).
   *   403 → CSRF failure: the session cookie is no longer valid even
   *         though the CSRF token may be cached. Treat as session-expired
   *         so the user is dropped back to login.
   *   404 → proposal already decided or not found. The inbox refetch will
   *         drop the row from the list; we just flash the user.
   *   other → generic failure flash; keep the dialog open so the user can
   *           retry.
   */
  function onMutationError(cause: unknown, proposalId: Uuid): void {
    void proposalId
    if (cause instanceof ApiError) {
      if (cause.status === 401) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired — sign in again.')
        dialog = null
        return
      }
      if (cause.status === 403) {
        sessionExpired = true
        session.clear()
        session.pushFlash('error', 'Session expired or CSRF failed — please sign in again.')
        dialog = null
        return
      }
      if (cause.status === 404) {
        session.pushFlash('error', 'Proposal not found (already decided?).')
        dialog = null
        void refreshList()
        return
      }
      session.pushFlash('error', `Action failed (${cause.code}).`)
      return
    }
    session.pushFlash('error', 'Action failed — is the backend running on :8080?')
  }

  function short(id: Uuid): string {
    // First 8 chars of the UUID — enough to identify the row in a flash
    // without crowding the banner. Bound as text.
    return id.slice(0, 8)
  }

  // Visible proposals: keep decided rows out of the list once the refetch
  // catches up (defensive — the server should already have removed them).
  let visibleProposals = $derived(
    proposals.filter((p) => (reviewStates[p.proposal_id] ?? 'pending') === 'pending'),
  )
</script>

<section class="page page-inbox">
  <h1>Inbox</h1>
  <p class="tagline">Review pending proposals. Each decision is permanent.</p>

  {#if sessionExpired}
    <p class="state state-error" role="alert">Session expired — sign in again.</p>
  {:else}
    <StateBox
      loading={listLoading}
      error={listError}
      empty={visibleProposals.length === 0}
      emptyText="No pending proposals."
    >
      <ul class="proposal-list" aria-label="Pending proposals">
        {#each visibleProposals as p (p.proposal_id)}
          <li class="proposal-card">
            <div
              class="proposal-head"
              tabindex="0"
              role="button"
              aria-expanded={openId === p.proposal_id}
              aria-label={`Toggle review detail for ${p.subject} ${p.predicate}`}
              onclick={() => void openDetail(p)}
              onkeydown={(e) => onRowKeydown(e, p)}
            >
              <span class="subject">{p.subject}</span>
              <span class="predicate">{p.predicate}</span>
              <span class="value">{formatValue(p.value)}</span>
              <span class="domain">{p.domain}</span>
              <span class="submitted">{formatDate(p.submitted_at)}</span>
            </div>

            {#if openId === p.proposal_id}
              {@const d = details[p.proposal_id] ?? { kind: 'idle' }}
              <div class="proposal-detail">
                {#if d.kind === 'loading'}
                  <p class="state state-loading" role="status">Loading evidence…</p>
                {:else if d.kind === 'error'}
                  <p class="state state-error" role="alert">{d.message}</p>
                {:else if d.kind === 'ready'}
                  {@const before = d.currentValue}
                  {@const diffs = diffFor(p, before)}
                  <DiffPreviewCmp diffs={diffs} />

                  <section class="evidence" aria-label="Evidence excerpt">
                    <h3>Evidence</h3>
                    <dl class="evidence-fields">
                      <div><dt>Provenance</dt><dd>{d.evidence.provenance_kind}</dd></div>
                      <div><dt>Source ID</dt><dd>{d.evidence.source_id ?? '—'}</dd></div>
                      <div><dt>Quote hash</dt><dd>{d.evidence.quote_hash ?? '—'}</dd></div>
                    </dl>
                    {#if d.evidence.excerpt}
                      <blockquote class="excerpt">{d.evidence.excerpt}</blockquote>
                    {:else}
                      <p class="excerpt excerpt-none">No text excerpt for this provenance kind.</p>
                    {/if}
                  </section>

                  {#if d.currentClaims.length > 0}
                    <p class="prior-claims">
                      {d.currentClaims.length} current confirmed claim(s) in scope —
                      Supersede / Edit will replace them.
                    </p>
                  {:else}
                    <p class="prior-claims prior-none">
                      No prior confirmed claim in scope — Approve will create a new claim.
                    </p>
                  {/if}

                  <div class="actions" role="group" aria-label="Review actions">
                    <button
                      type="button"
                      class="action action-approve"
                      onclick={() => startApprove(p, before)}
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      class="action action-reject"
                      onclick={() => startReject(p, before)}
                    >
                      Reject
                    </button>
                    <button
                      type="button"
                      class="action action-supersede"
                      disabled={d.currentClaims.length === 0}
                      onclick={() => startSupersede(p, before, d.currentClaims)}
                    >
                      Supersede
                    </button>
                    <button
                      type="button"
                      class="action action-edit"
                      onclick={() => startEdit(p, before, d.currentClaims)}
                    >
                      Edit
                    </button>
                  </div>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    </StateBox>
  {/if}
</section>

{#if dialog}
  <div
    class="dialog-backdrop"
    role="presentation"
    onclick={() => cancelDialog()}
    onkeydown={(e) => {
      if (e.key === 'Escape') cancelDialog()
    }}
  ></div>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label={`Confirm ${dialog.kind}`}
  >
    <h2>Confirm {dialog.kind}?</h2>
    <p class="dialog-summary">
      Subject <span class="mono">{dialog.subject}</span>
      · Predicate <span class="mono">{dialog.predicate}</span>
    </p>

    {#if dialog.kind === 'approve' || dialog.kind === 'reject'}
      <DiffPreviewCmp diffs={dialog.diffs} />
      <p class="dialog-prompt">
        {dialog.kind === 'approve'
          ? 'Approve this proposal and commit the new claim?'
          : 'Reject this proposal? The decision is permanent.'}
      </p>
    {:else if dialog.kind === 'supersede'}
      <DiffPreviewCmp diffs={dialog.diffs} />
      <fieldset class="supersede-pick">
        <legend>Claims to supersede</legend>
        {#each dialog.claims as c (c.claim_id)}
          <label class="check">
            <input
              type="checkbox"
              bind:checked={c.selected}
              disabled={acting}
            />
            <span class="mono">{c.claim_id}</span>
            <span class="check-value">{formatValue(c.value)}</span>
          </label>
        {/each}
      </fieldset>
      <p class="dialog-prompt">
        Confirm supersede — a new claim will replace the selected priors.
      </p>
    {:else if dialog.kind === 'edit'}
      <!-- Edit: live diff against the user-typed value -->
      <label class="edit-label" for="edit-value">Edited value</label>
      <input
        id="edit-value"
        type="text"
        class="edit-input"
        bind:value={dialog.editedValue}
        disabled={acting}
      />
      <DiffPreviewCmp diffs={editDiffs} />
      <p class="dialog-prompt">
        Confirm edit — a new claim with the edited value will replace the
        {dialog.claimIds.length} current confirmed claim(s) in scope.
      </p>
    {/if}

    <div class="dialog-actions">
      <button
        type="button"
        class="action action-yes"
        bind:this={yesBtn}
        disabled={acting}
        onclick={() => void confirmDialog()}
      >
        {acting ? 'Working…' : 'Yes, confirm'}
      </button>
      <button
        type="button"
        class="action action-no"
        disabled={acting}
        onclick={() => cancelDialog()}
      >
        No, cancel
      </button>
    </div>
  </div>
{/if}

<style>
  .page {
    padding: 1.5rem 0;
  }

  .tagline {
    margin: 0.25rem 0 1.25rem;
    opacity: 0.75;
  }

  .proposal-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }

  .proposal-card {
    border-radius: 0.5rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.06);
    overflow: hidden;
  }

  .proposal-head {
    display: grid;
    grid-template-columns: 1.2fr 1fr 1.4fr 0.8fr 0.9fr;
    gap: 0.5rem;
    padding: 0.6rem 0.75rem;
    cursor: pointer;
    outline: none;
    align-items: center;
  }

  .proposal-head:hover,
  .proposal-head:focus-visible {
    background: rgba(127, 127, 127, 0.15);
  }

  .subject {
    font-weight: 600;
  }

  .predicate {
    opacity: 0.85;
    font-style: italic;
  }

  .value {
    opacity: 0.75;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .domain,
  .submitted {
    opacity: 0.65;
    font-size: 0.85rem;
  }

  .proposal-detail {
    padding: 0.5rem 0.85rem 0.85rem;
    background: rgba(127, 127, 127, 0.04);
    display: grid;
    gap: 0.75rem;
  }

  .evidence h3 {
    margin: 0 0 0.35rem;
    font-size: 0.9rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.75;
  }

  .evidence-fields {
    margin: 0 0 0.4rem;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: 0.2rem 1rem;
    font-size: 0.85rem;
  }

  .evidence-fields div {
    display: flex;
    gap: 0.4rem;
  }

  .evidence-fields dt {
    opacity: 0.6;
    min-width: 6rem;
  }

  .evidence-fields dd {
    margin: 0;
    word-break: break-word;
  }

  .excerpt {
    margin: 0;
    padding: 0.5rem 0.75rem;
    border-left: 3px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.06);
    white-space: pre-wrap;
    word-break: break-word;
    font-style: italic;
  }

  .excerpt-none {
    font-style: italic;
    opacity: 0.65;
    border-left: 3px solid rgba(127, 127, 127, 0.25);
    background: transparent;
  }

  .prior-claims {
    margin: 0;
    font-size: 0.85rem;
    opacity: 0.75;
  }

  .prior-none {
    font-style: italic;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
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

  .action:hover:not(:disabled),
  .action:focus-visible:not(:disabled) {
    background: rgba(127, 127, 127, 0.2);
  }

  .action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .action-approve {
    background: rgba(60, 160, 90, 0.2);
    border-color: rgba(60, 160, 90, 0.55);
  }

  .action-reject {
    background: rgba(190, 70, 70, 0.2);
    border-color: rgba(190, 70, 70, 0.55);
  }

  .action-supersede,
  .action-edit {
    background: rgba(80, 130, 200, 0.18);
    border-color: rgba(80, 130, 200, 0.5);
  }

  .state {
    margin: 0.5rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
  }

  .state-loading {
    opacity: 0.75;
  }

  .state-error {
    background: rgba(190, 70, 70, 0.15);
    border-color: rgba(190, 70, 70, 0.5);
  }

  /* ── Dialog ─────────────────────────────────────────────────────────── */
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
    max-height: 86vh;
    overflow: auto;
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
    text-transform: capitalize;
  }

  .dialog-summary {
    margin: 0 0 0.75rem;
    font-size: 0.9rem;
    opacity: 0.8;
    word-break: break-word;
  }

  .dialog-prompt {
    margin: 0.75rem 0 0.5rem;
    font-size: 0.9rem;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.85em;
  }

  .supersede-pick {
    margin: 0.5rem 0 0;
    border: 1px solid rgba(127, 127, 127, 0.35);
    border-radius: 0.375rem;
    padding: 0.5rem 0.6rem;
    display: grid;
    gap: 0.3rem;
  }

  .supersede-pick legend {
    font-size: 0.8rem;
    opacity: 0.7;
    padding: 0 0.25rem;
  }

  .check {
    display: grid;
    grid-template-columns: auto 1fr 1.5fr;
    gap: 0.5rem;
    align-items: center;
    font-size: 0.85rem;
  }

  .check-value {
    opacity: 0.75;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .edit-label {
    display: block;
    font-size: 0.8rem;
    opacity: 0.7;
    margin: 0.5rem 0 0.2rem;
  }

  .edit-input {
    width: 100%;
    padding: 0.45rem 0.55rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
    box-sizing: border-box;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  .action-yes {
    background: rgba(60, 160, 90, 0.25);
    border-color: rgba(60, 160, 90, 0.6);
    font-weight: 600;
  }

  .action-no {
    background: rgba(127, 127, 127, 0.15);
  }
</style>
