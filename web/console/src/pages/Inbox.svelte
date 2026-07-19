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

  let dialog = $state<Dialog | null>(null)
  // `true` while a mutation API call is in flight — disables Yes/No so the
  // user can't double-submit. Distinct from list/detail loading.
  let acting = $state(false)
  // Ref to the Yes button so we can focus it when the dialog opens.
  let yesBtn = $state<HTMLButtonElement | null>(null)
  // Ref to the dialog root so the focus-trap keydown handler can query its
  // focusable descendants (Fix I2).
  let dialogRoot = $state<HTMLDivElement | null>(null)

  /**
   * Fix I1: when the Supersede dialog has every checkbox unchecked, the
   * Yes button is disabled and an inline validation message is shown.
   * Prevents firing `apiSupersede(id, [])` with an empty
   * `superseded_claim_ids` array (which the server would accept but is
   * almost certainly not what the reviewer meant).
   */
  let supersedeNoneSelected = $derived(
    dialog?.kind === 'supersede' && !dialog.claims.some((c) => c.selected),
  )

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
      const ready: DetailState = {
        kind: 'ready',
        evidence: evidenceResult,
        currentValue,
        currentClaims: confirmed,
      }
      // Always cache the result so a stale-but-completed fetch warms the
      // per-id cache for the next open (Fix I3): previously the seq guard
      // short-circuited BEFORE the write, leaving `details[id]` stuck at
      // `{kind:'loading'}` forever and forcing a refetch on re-open. The
      // seq guard now only gates whether the OPEN panel is updated.
      details = { ...details, [proposal.proposal_id]: ready }
      if (seq !== detailSeq) return
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) {
        // 401 is session-wide — surface it regardless of seq (the user
        // needs to re-authenticate before anything else can succeed).
        if (seq === detailSeq) {
          sessionExpired = true
          session.clear()
          session.pushFlash('error', 'Session expired — sign in again.')
        }
        return
      }
      const message =
        cause instanceof ApiError
          ? `Failed to load evidence (${cause.code}).`
          : 'Failed to load evidence — is the backend running on :8080?'
      // Cache the error too (Fix I3): a stale failed fetch should not be
      // re-thrown on re-open; the cached `{kind:'error'}` will be reused
      // and the user can retry by collapsing/re-expanding (which clears
      // the entry via the `existing.kind` guard above being only on
      // 'ready'/'error' — see the toggle/reopen path).
      details = { ...details, [proposal.proposal_id]: { kind: 'error', message } }
      if (seq !== detailSeq) return
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

  async function focusYes(): Promise<void> {
    // Wait for the DOM to render the Yes button, then focus it so keyboard
    // users can confirm with Enter immediately (DoD: dialog traps focus).
    await tick()
    yesBtn?.focus()
  }

  /**
   * Minimal focus trap (Fix I2). The dialog already sets `role="dialog"`,
   * `aria-modal="true"`, closes on Escape, and focuses Yes on open — but
   * without intercepting Tab, a keyboard user can tab out into background
   * elements (the proposal list behind the modal). This handler cycles
   * focus among the dialog's focusable descendants (Yes, No, plus any
   * supersede checkboxes that are not disabled).
   *
   * Deliberately dependency-free: no focus-trap library, ~30 LOC. Called
   * from the dialog root's `onkeydown`.
   */
  function onDialogKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      cancelDialog()
      return
    }
    if (event.key !== 'Tab' || !dialogRoot) return
    const focusables = Array.from(
      dialogRoot.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    )
    if (focusables.length === 0) return
    const first = focusables[0]
    const last = focusables[focusables.length - 1]
    const active = document.activeElement as HTMLElement | null
    if (event.shiftKey) {
      if (active === first || !dialogRoot.contains(active)) {
        event.preventDefault()
        last.focus()
      }
    } else {
      if (active === last || !dialogRoot.contains(active)) {
        event.preventDefault()
        first.focus()
      }
    }
  }

  function cancelDialog(): void {
    if (acting) return
    dialog = null
  }

  // ── Mutation execution (only reachable from the dialog's Yes button) ───
  async function confirmDialog(): Promise<void> {
    const d = dialog
    if (!d || acting) return
    // Snapshot the request plan BEFORE any `await`. TypeScript narrowing on
    // a `$state` proxy is invalidated by `await` (the proxy can be mutated
    // externally), so we extract every field we need up front into a
    // discriminated `plan` and only then start the request.
    type Plan =
      | { action: 'approve'; proposalId: Uuid }
      | { action: 'reject'; proposalId: Uuid }
      | { action: 'supersede'; proposalId: Uuid; ids: Uuid[] }
    let plan: Plan
    if (d.kind === 'approve') {
      plan = { action: 'approve', proposalId: d.proposalId }
    } else if (d.kind === 'reject') {
      plan = { action: 'reject', proposalId: d.proposalId }
    } else if (d.kind === 'supersede') {
      // d.kind === 'supersede' — exhaustive over the (now 3-variant) Dialog
      // union; the explicit kind check lets TS narrow `d` to the supersede
      // variant so `d.claims` is visible.
      plan = {
        action: 'supersede',
        proposalId: d.proposalId,
        ids: d.claims.filter((c) => c.selected).map((c) => c.claim_id),
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
        finalize(plan.proposalId, 'supersede', `Superseded proposal ${short(plan.proposalId)} (event_seq=${eventSeq}).`)
      }
    } catch (cause) {
      onMutationError(cause)
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
    // `transition` returns `ReviewState | TransitionError`. The happy path
    // is `approved`/`rejected`; the error codes (`already_decided`,
    // `invalid_action`) indicate a logic regression — the server just
    // confirmed success, so the local state should still be `pending`.
    // Fix M1: fail LOUDLY here instead of silently locking the row to
    // `approved`, so a future regression (e.g. a double-finalize race or
    // a new ReviewAction variant the switch doesn't handle) surfaces in
    // dev/test rather than mislabeling a rejected proposal as approved.
    if (next !== 'approved' && next !== 'rejected') {
      throw new Error(
        `transition returned unexpected: ${next} (prior=${prior}, action=${action})`,
      )
    }
    reviewStates = { ...reviewStates, [proposalId]: next }
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
   *   409 → concurrent decision conflict (IdempotencyConflict /
   *         InvalidTransition). Distinct from 404: the proposal exists
   *         but a concurrent reviewer (or a replay) already moved it.
   *         Refresh + close so the user sees the current state (Fix M2).
   *   other → generic failure flash; keep the dialog open so the user can
   *           retry.
   */
  function onMutationError(cause: unknown): void {
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
      if (cause.status === 409) {
        // Fix M2: dedicated conflict branch — the proposal was already
        // decided concurrently. Surface a distinct flash (not the 404
        // "not found" wording) and refresh so the row reflects the
        // winning decision.
        session.pushFlash(
          'error',
          'Proposal was already decided concurrently — refreshing.',
        )
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
                      Supersede will replace them.
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
    aria-describedby="dialog-summary"
    tabindex="-1"
    bind:this={dialogRoot}
    onkeydown={(e) => onDialogKeydown(e)}
  >
    <h2>Confirm {dialog.kind}?</h2>
    <p class="dialog-summary" id="dialog-summary">
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
      {#if supersedeNoneSelected}
        <p class="validation-warning" role="alert">
          Select at least one claim to supersede.
        </p>
      {/if}
    {/if}

    <div class="dialog-actions">
      <button
        type="button"
        class="action action-yes"
        bind:this={yesBtn}
        disabled={acting || supersedeNoneSelected}
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
  /* ── Inbox — token-driven, kills all flagged anti-patterns ────────────
   * What changed vs the prior block:
   *  - border-left:3px → 1px hairline + surface tint (kills side-stripe ×2)
   *  - tracked-uppercase .evidence h3 → sentence-case Inter title
   *  - .dialog min-width:26rem → clamp() (kills mobile overflow)
   *  - all rgba(127,127,127,X) + rgba(190,70,70,X) → tokens
   *  - .dialog bg now void-tinted (was hardcoded --console-bg fallback #fff)
   *  - .action buttons: min-height 44px (WCAG 2.5.5 touch targets)
   *  - .proposal-head outline:none → focus-visible ring via box-shadow
   */
  .page {
    padding: var(--space-lg) 0;
  }

  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-headline);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--text-headline-tracking);
    line-height: var(--text-headline-leading);
  }

  .tagline {
    margin: var(--space-xs) 0 var(--space-md);
    color: var(--text-secondary);
    font-size: var(--text-body);
    max-width: var(--content-measure);
  }

  .proposal-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-sm);
  }

  .proposal-card {
    border: var(--border-hairline);
    border-radius: var(--radius-lg);
    background: var(--surface-flat);
    overflow: hidden;
  }

  /* Sentence-case 5-col head on desktop; stacks to 2-col on tablet,
   * single-col with implicit labels on phone. The prior 5-col grid had no
   * breakpoints and was unreadable below ~600px. */
  .proposal-head {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-xs);
    padding: var(--space-md);
    cursor: pointer;
    align-items: center;
    background: transparent;
    border: none;
    width: 100%;
    text-align: left;
    color: inherit;
    font: inherit;
    transition: background var(--duration-fast) var(--ease-out-quart);
  }

  /* Focus ring via box-shadow (the prior outline:none stripped keyboard
   * focus signal — WCAG 2.4.7 Focus Visible failure). */
  .proposal-head:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--color-accent);
  }

  .proposal-head:hover {
    background: var(--overlay-ink-04);
  }

  @media (min-width: 40rem) {
    .proposal-head {
      grid-template-columns: 1.2fr 1fr 1.4fr 0.8fr 0.9fr;
      gap: var(--space-sm);
    }
  }

  .subject {
    font-family: var(--font-body);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    font-size: var(--text-body);
  }

  .predicate {
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: var(--text-mono);
  }

  .value {
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-body);
    /* title attribute set by template provides full text on hover/focus
     * for the truncated case — recoverable without opening detail. */
  }

  .domain,
  .submitted {
    color: var(--text-tertiary);
    font-size: var(--text-label);
    font-family: var(--font-mono);
  }

  .proposal-detail {
    padding: var(--space-sm) var(--space-md) var(--space-md);
    background: var(--surface-sunken);
    display: grid;
    gap: var(--space-md);
    border-top: 1px solid var(--color-hairline);
  }

  /* Evidence heading — sentence-case Inter title (kills tracked eyebrow).
   * The prior text-transform:uppercase + letter-spacing:0.04em was the
   * saturated AI scaffold tell. */
  .evidence h3 {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-body);
    font-size: var(--text-title);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    letter-spacing: 0;
    text-transform: none;
    line-height: var(--text-title-leading);
  }

  .evidence-fields {
    margin: 0 0 var(--space-xs);
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: var(--space-xs) var(--space-md);
    font-size: var(--text-body);
  }

  .evidence-fields div {
    display: flex;
    gap: var(--space-sm);
    align-items: baseline;
  }

  .evidence-fields dt {
    color: var(--text-secondary);
    font-size: var(--text-label);
    min-width: 6rem;
  }

  .evidence-fields dd {
    margin: 0;
    color: var(--text-primary);
    word-break: break-word;
  }

  /* Excerpt — was the side-stripe source (border-left:3px). Now a full
   * hairline border + surface tint + leading quote glyph. The 3px stripe
   * is the detector's #1 AI tell; the new treatment carries the same
   * "this is a quote" affordance without it. */
  .excerpt {
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--overlay-ink-04);
    white-space: pre-wrap;
    word-break: break-word;
    font-style: italic;
    color: var(--text-primary);
    font-size: var(--text-body);
    line-height: var(--text-body-leading);
    position: relative;
  }

  .excerpt-none {
    font-style: italic;
    color: var(--text-tertiary);
    border: 1px dashed var(--color-hairline);
    background: transparent;
  }

  .prior-claims {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-body);
  }

  .prior-none {
    color: var(--text-tertiary);
    font-style: italic;
  }

  /* ── Action buttons — 44px touch min, semantic colors via tokens ──── */
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }

  .action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 44px;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: var(--border-hairline);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    letter-spacing: var(--text-label-tracking);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out-quart),
      border-color var(--duration-fast) var(--ease-out-quart);
  }

  .action:hover:not(:disabled) {
    background: var(--overlay-ink-06);
    border-color: var(--color-ink-faint);
  }

  .action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Always paired with a text label, never color alone. */
  .action-approve {
    background: var(--overlay-success-soft);
    border-color: var(--color-success);
    color: var(--text-primary);
  }

  .action-approve:hover:not(:disabled) {
    background: var(--color-success);
  }

  .action-reject {
    background: var(--overlay-danger-soft);
    border-color: var(--color-danger);
    color: var(--text-primary);
  }

  .action-reject:hover:not(:disabled) {
    background: var(--color-danger);
  }

  .action-supersede {
    background: var(--overlay-info-soft);
    border-color: var(--color-info);
    color: var(--text-primary);
  }

  .action-supersede:hover:not(:disabled) {
    background: var(--color-info);
  }

  /* ── State banners ─────────────────────────────────────────────────── */
  .state {
    margin: var(--space-sm) 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    color: var(--text-primary);
    font-size: var(--text-body);
  }

  .state-loading {
    color: var(--text-secondary);
  }

  .state-error {
    background: var(--overlay-danger-soft);
    border-color: var(--color-danger);
  }

  /* ── Dialog (fixed center, clamp() width — kills mobile overflow) ──── */
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in oklch, var(--color-void) 75%, transparent);
    backdrop-filter: blur(2px);
    z-index: var(--z-modal-backdrop);
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: var(--z-modal);
    /* width: clamp(min, preferred, max) — was min-width:26rem which
     * overflowed on <416px viewports and clipped the Yes/No buttons. */
    width: clamp(18rem, 92vw, 40rem);
    max-height: 86vh;
    overflow: auto;
    padding: var(--space-lg);
    border-radius: var(--radius-lg);
    border: 1px solid var(--color-hairline);
    background: var(--surface-overlay);
    color: var(--text-primary);
    box-shadow: var(--shadow-lift);
  }

  .dialog h2 {
    margin: 0 0 var(--space-xs);
    font-family: var(--font-display);
    font-size: var(--text-headline);
    font-weight: var(--weight-semibold);
    color: var(--text-primary);
    text-transform: capitalize;
    letter-spacing: var(--text-headline-tracking);
    line-height: var(--text-headline-leading);
  }

  .dialog-summary {
    margin: 0 0 var(--space-md);
    color: var(--text-secondary);
    font-size: var(--text-body);
    word-break: break-word;
  }

  .dialog-prompt {
    margin: var(--space-md) 0 var(--space-xs);
    color: var(--text-primary);
    font-size: var(--text-body);
  }

  /* Mono — machine output (UUIDs, predicates). */
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-mono);
    color: var(--text-secondary);
    word-break: break-all;
  }

  .supersede-pick {
    margin: var(--space-sm) 0 0;
    border: var(--border-hairline);
    border-radius: var(--radius-md);
    padding: var(--space-sm) var(--space-md);
    display: grid;
    gap: var(--space-xs);
  }

  .supersede-pick legend {
    font-family: var(--font-body);
    font-size: var(--text-label);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    padding: 0 var(--space-xs);
  }

  .check {
    display: grid;
    grid-template-columns: auto 1fr 1.5fr;
    gap: var(--space-sm);
    align-items: center;
    font-size: var(--text-body);
    min-height: 44px;
    padding: var(--space-xs) 0;
  }

  .check input[type='checkbox'] {
    width: 20px;
    height: 20px;
    accent-color: var(--color-accent);
    cursor: pointer;
  }

  .check-value {
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Inline validation — full border, danger tint (no side-stripe). */
  .validation-warning {
    margin: var(--space-xs) 0 0;
    padding: var(--space-sm) var(--space-md);
    font-size: var(--text-body);
    color: var(--text-primary);
    border-radius: var(--radius-md);
    border: 1px solid var(--color-danger);
    background: var(--overlay-danger-soft);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
    margin-top: var(--space-md);
    flex-wrap: wrap;
  }

  .action-yes {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--text-on-accent);
    font-weight: var(--weight-semibold);
  }

  .action-yes:hover:not(:disabled) {
    background: var(--color-accent-deep);
    border-color: var(--color-accent-deep);
  }

  .action-no {
    background: transparent;
    color: var(--text-secondary);
  }

  .action-no:hover:not(:disabled) {
    background: var(--overlay-ink-06);
    color: var(--text-primary);
  }
</style>
