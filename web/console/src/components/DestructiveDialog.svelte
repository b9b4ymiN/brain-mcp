<script lang="ts">
  /**
   * DestructiveDialog — generic destructive-action confirm dialog (Task E3.3 Part C).
   *
   * The single UI chokepoint for hard-purge / entity-merge / entity-split.
   * Renders the server-provided `warning.message` prominently BEFORE any
   * confirm click (Task E3.2 DoD #2), plus the structured `preview` items
   * (target_kind / target_id / effect).
   *
   * Two independent confirmation gates (per `DestructiveWarning`):
   *   - `requires_two_step_nonce`: displays the server nonce + a text input;
   *     the Yes button is disabled until the user types the nonce exactly.
   *     This is a confirmation gate only — the actual server nonce is what
   *     the parent's `onConfirm` echoes to `/purge/execute`.
   *   - `requires_recent_reauth`: embeds `<ReauthForm>`; the Yes button is
   *     disabled until reauth succeeds (track a local `reauthed` flag).
   *
   * Anti-XSS (Task E3.3 Part F): every dynamic string — warning message,
   * preview items, nonce — is bound as text via Svelte's `{value}` (auto-
   * escaped). There is NO `{@html}` anywhere in this file. The server-side
   * XSS payload seeded by the E2E suite MUST render as literal text here.
   *
   * a11y: `role="dialog"`, `aria-modal="true"`, `aria-describedby` on the
   * confirm button pointing at the warning message, focus trap (Tab cycles
   * Yes/No/inputs), Escape cancels, the warning has `role="alert"`.
   */
  import { tick } from 'svelte'
  import type {
    DestructiveWarning,
    DestructivePreviewItem,
  } from '../lib/api'
  import ReauthForm from './ReauthForm.svelte'

  interface Props {
    /** The server-provided warning (from `api.destructiveWarning` or `purgePreview`). */
    warning: DestructiveWarning
    /** The structured preview items to enumerate (caller-built for merge/split). */
    preview: DestructivePreviewItem[]
    /**
     * The server nonce to display + type-match (for hard purge). When provided
     * AND `warning.requires_two_step_nonce` is true, the Yes button stays
     * disabled until the user types this value verbatim.
     */
    nonce?: string
    /**
     * Convenience flag — derived from `warning.requires_recent_reauth` but
     * kept as a separate prop so the parent can force the gate off (e.g. if
     * the parent already reauthed before opening the dialog).
     */
    requiresReauth: boolean
    /** Fired when the user clicks Yes AND every gate has passed. */
    onConfirm: () => Promise<void>
    /** Fired when the user cancels (No button, Escape, or backdrop click). */
    onCancel: () => void
    /** True while the parent's mutation is in flight — disables Yes/No. */
    acting: boolean
  }

  let {
    warning,
    preview,
    nonce,
    requiresReauth,
    onConfirm,
    onCancel,
    acting,
  }: Props = $props()

  // ── Two-step nonce gate ──────────────────────────────────────────────────
  // The user types the server nonce verbatim into this input; the typed value
  // is a CONFIRMATION gate only — `onConfirm` echoes the original `nonce`
  // prop to the server (NOT `typedNonce`). This prevents drive-by confirms.
  let typedNonce = $state('')

  // Whether the typed nonce matches the server nonce exactly. Only consulted
  // when `warning.requires_two_step_nonce` AND a `nonce` was provided.
  let nonceMatched = $derived(
    !warning.requires_two_step_nonce ||
      nonce === undefined ||
      (nonce.length > 0 && typedNonce === nonce),
  )

  // ── Recent-reauth gate ───────────────────────────────────────────────────
  // Flipped to true when the embedded `<ReauthForm>` reports success. Stays
  // false otherwise → the Yes button stays disabled until the user re-auths.
  let reauthed = $state(false)
  let reauthFreshFor = $state<number | null>(null)

  function onReauthSuccess(freshForSeconds: number): void {
    reauthed = true
    reauthFreshFor = freshForSeconds
  }

  // Overall confirmation-enabled predicate. Every active gate must be passed.
  let canConfirm = $derived(
    !acting &&
      nonceMatched &&
      (!requiresReauth || reauthed) &&
      // Defensive: never enable Yes for a hard purge if the server didn't
      // actually hand back a nonce to type (would allow skipping the gate).
      !(warning.requires_two_step_nonce && (nonce === undefined || nonce.length === 0)),
  )

  // ── Focus trap (mirror Inbox.svelte onDialogKeydown pattern) ─────────────
  let dialogRoot = $state<HTMLDivElement | null>(null)
  let yesBtn = $state<HTMLButtonElement | null>(null)

  async function focusYes(): Promise<void> {
    await tick()
    // If Yes is disabled (gates not passed), focus the first focusable
    // descendant instead so screen-reader + keyboard users land somewhere
    // sensible rather than on a disabled button.
    if (yesBtn && !yesBtn.disabled) {
      yesBtn.focus()
      return
    }
    if (dialogRoot) {
      const first = dialogRoot.querySelector<HTMLElement>(
        'input:not([disabled]), button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )
      first?.focus()
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      if (!acting) onCancel()
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

  function onBackdropClick(): void {
    if (!acting) onCancel()
  }

  async function onYesClick(): Promise<void> {
    if (!canConfirm) return
    await onConfirm()
  }

  // Focus the right element on mount. `$effect` runs after the DOM settles.
  $effect(() => {
    void focusYes()
  })
</script>

<div
  class="dialog-backdrop"
  role="presentation"
  onclick={onBackdropClick}
  onkeydown={(e) => {
    if (e.key === 'Escape' && !acting) onCancel()
  }}
></div>

<div
  class="dialog destructive-dialog"
  role="dialog"
  aria-modal="true"
  aria-label={`Confirm ${warning.action.replace('_', ' ')}?`}
  aria-describedby="destructive-warning-message"
  tabindex="-1"
  bind:this={dialogRoot}
  onkeydown={onKeydown}
>
  <h2>Confirm {warning.action.replace('_', ' ')}?</h2>

  <p
    class="warning-message"
    id="destructive-warning-message"
    role="alert"
  >
    {warning.message}
  </p>

  {#if warning.irreversible}
    <p class="warning-flag warning-flag-irreversible" role="note">
      This action is marked IRREVERSIBLE.
    </p>
  {/if}

  {#if preview.length > 0}
    <section class="preview" aria-label="Affected targets">
      <h3>Affected targets ({preview.length})</h3>
      <ul class="preview-list">
        {#each preview as item, i (`${item.target_kind}:${item.target_id}:${i}`)}
          <li class="preview-row">
            <span class="preview-kind">{item.target_kind}</span>
            <span class="preview-id">{item.target_id}</span>
            <span class="preview-effect">{item.effect}</span>
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="preview preview-empty">No structured targets to display.</p>
  {/if}

  {#if warning.requires_two_step_nonce && nonce}
    <fieldset class="nonce-gate">
      <legend>Two-step confirmation</legend>
      <p class="nonce-prompt">
        Type this token to confirm:
        <span class="nonce-value mono">{nonce}</span>
      </p>
      <label for="destructive-nonce-input" class="nonce-label">Confirmation token</label>
      <input
        id="destructive-nonce-input"
        type="text"
        autocomplete="off"
        placeholder="type the token"
        bind:value={typedNonce}
        disabled={acting}
        aria-describedby={nonceMatched ? undefined : 'destructive-nonce-hint'}
      />
      {#if !nonceMatched}
        <p class="nonce-hint" id="destructive-nonce-hint" role="status">
          Token does not match yet — the Yes button stays disabled.
        </p>
      {/if}
    </fieldset>
  {/if}

  {#if requiresReauth}
    <section class="reauth-gate" aria-label="Recent re-authentication required">
      <h3>Re-authentication required</h3>
      {#if reauthed}
        <p class="reauth-done" role="status">
          Re-authenticated — fresh for {reauthFreshFor ?? 0}s.
        </p>
      {:else}
        <p class="reauth-prompt">
          Re-enter your username and password to unlock destructive actions.
        </p>
        <ReauthForm onsuccess={onReauthSuccess} />
      {/if}
    </section>
  {/if}

  <div class="dialog-actions">
    <button
      type="button"
      class="action action-yes"
      bind:this={yesBtn}
      disabled={!canConfirm}
      aria-describedby="destructive-warning-message"
      onclick={onYesClick}
    >
      {acting ? 'Working…' : 'Yes, confirm'}
    </button>
    <button
      type="button"
      class="action action-no"
      disabled={acting}
      onclick={onCancel}
    >
      No, cancel
    </button>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 60;
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 61;
    min-width: 28rem;
    max-width: min(44rem, 92vw);
    max-height: 88vh;
    overflow: auto;
    padding: 1.25rem 1.25rem 1rem;
    border-radius: 0.6rem;
    border: 1px solid rgba(190, 70, 70, 0.6);
    background: var(--console-bg, #fff);
    color: var(--console-fg, #111);
    box-shadow: 0 10px 36px rgba(0, 0, 0, 0.3);
  }

  .dialog h2 {
    margin: 0 0 0.5rem;
    font-size: 1.1rem;
    text-transform: capitalize;
  }

  .warning-message {
    margin: 0 0 0.5rem;
    padding: 0.65rem 0.85rem;
    border-radius: 0.4rem;
    border: 1px solid rgba(190, 70, 70, 0.6);
    background: rgba(190, 70, 70, 0.18);
    font-weight: 600;
    word-break: break-word;
  }

  .warning-flag {
    margin: 0 0 0.6rem;
    padding: 0.4rem 0.6rem;
    font-size: 0.85rem;
    border-radius: 0.3rem;
  }

  .warning-flag-irreversible {
    border: 1px solid rgba(190, 70, 70, 0.55);
    background: rgba(190, 70, 70, 0.12);
  }

  .preview {
    margin: 0.5rem 0 0.6rem;
  }

  .preview h3 {
    margin: 0 0 0.35rem;
    font-size: 0.8rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.75;
  }

  .preview-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
    max-height: 14rem;
    overflow: auto;
  }

  .preview-row {
    display: grid;
    grid-template-columns: minmax(6rem, auto) 1fr 1.5fr;
    gap: 0.5rem;
    padding: 0.35rem 0.5rem;
    border-radius: 0.3rem;
    background: rgba(127, 127, 127, 0.08);
    font-size: 0.85rem;
    align-items: center;
  }

  .preview-kind {
    font-weight: 600;
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.03em;
    opacity: 0.85;
  }

  .preview-id {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.82rem;
    word-break: break-all;
  }

  .preview-effect {
    opacity: 0.8;
    font-style: italic;
  }

  .preview-empty {
    margin: 0.4rem 0;
    font-size: 0.85rem;
    opacity: 0.7;
    font-style: italic;
  }

  .nonce-gate {
    margin: 0.6rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.4rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
  }

  .nonce-gate legend {
    font-size: 0.8rem;
    font-weight: 600;
    opacity: 0.8;
    padding: 0 0.3rem;
  }

  .nonce-prompt {
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
  }

  .nonce-value {
    margin-left: 0.4rem;
    padding: 0.15rem 0.4rem;
    border-radius: 0.3rem;
    background: rgba(127, 127, 127, 0.2);
    font-weight: 700;
    user-select: all;
  }

  .nonce-label {
    display: block;
    font-size: 0.8rem;
    margin-bottom: 0.2rem;
    opacity: 0.8;
  }

  .nonce-gate input {
    width: 100%;
    padding: 0.45rem 0.55rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.5);
    background: inherit;
    color: inherit;
    font: inherit;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .nonce-hint {
    margin: 0.35rem 0 0;
    font-size: 0.82rem;
    opacity: 0.75;
  }

  .reauth-gate {
    margin: 0.6rem 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.4rem;
    border: 1px solid rgba(190, 130, 70, 0.55);
    background: rgba(190, 130, 70, 0.08);
  }

  .reauth-gate h3 {
    margin: 0 0 0.35rem;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    opacity: 0.8;
  }

  .reauth-prompt {
    margin: 0 0 0.3rem;
    font-size: 0.85rem;
    opacity: 0.8;
  }

  .reauth-done {
    margin: 0;
    font-size: 0.85rem;
    color: inherit;
    opacity: 0.85;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.85rem;
  }

  .action {
    padding: 0.5rem 0.9rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.1);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .action:hover:not(:disabled),
  .action:focus-visible:not(:disabled) {
    background: rgba(127, 127, 127, 0.22);
  }

  .action:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .action-yes {
    background: rgba(190, 70, 70, 0.25);
    border-color: rgba(190, 70, 70, 0.6);
    font-weight: 600;
  }

  .action-no {
    background: rgba(127, 127, 127, 0.15);
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
</style>
