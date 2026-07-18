<script lang="ts">
  /**
   * ReauthForm — inline bootstrap-secret re-authentication (Task E3.3 Part C).
   *
   * Used inside `<DestructiveDialog>` to satisfy the
   * `requires_recent_reauth` gate before a hard purge. Calls
   * `api.reauth(secret)`; on success emits `onsuccess` with the freshness
   * window (so the parent can flip a `reauthed` flag and enable the confirm
   * button). On failure renders the error inline (no flash — the dialog owns
   * the surface).
   *
   * Anti-XSS: the only dynamic text is the per-field error string, bound as
   * text via Svelte's `{error}`. No `{@html}`. The `secret` input is
   * `type=password` with an explicit `<label>` for a11y.
   */
  import { reauth as apiReauth, ApiError } from '../lib/api'

  interface Props {
    /** Emitted when reauth succeeds. Carries `fresh_for_seconds` from server. */
    onsuccess: (freshForSeconds: number) => void
  }

  let { onsuccess }: Props = $props()

  let secret = $state('')
  let submitting = $state(false)
  let error = $state<string | null>(null)

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault()
    if (submitting) return
    const trimmed = secret.trim()
    if (!trimmed) {
      error = 'Enter the bootstrap secret.'
      return
    }
    submitting = true
    error = null
    try {
      const result = await apiReauth(trimmed)
      if (!result.reauthenticated) {
        // Defensive — the server returns 401 for a wrong secret rather than
        // `{reauthenticated:false}`, so this branch is unlikely. Treat as
        // failure regardless so the parent confirm button stays disabled.
        error = 'Re-authentication failed.'
        return
      }
      onsuccess(result.fresh_for_seconds)
      // Clear the secret from local state as soon as the server accepts it —
      // minimises the window during which the value is held in memory.
      secret = ''
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) {
        error = 'Wrong secret — try again.'
      } else if (cause instanceof ApiError) {
        error = `Re-auth failed (${cause.code}).`
      } else {
        error = 'Re-auth failed — is the backend running on :8080?'
      }
    } finally {
      submitting = false
    }
  }
</script>

<form class="reauth-form" onsubmit={submit}>
  <label for="reauth-secret" class="reauth-label">Bootstrap secret</label>
  <input
    id="reauth-secret"
    type="password"
    autocomplete="current-password"
    placeholder="secret"
    bind:value={secret}
    disabled={submitting}
    aria-describedby={error ? 'reauth-error' : undefined}
  />
  <button type="submit" disabled={submitting}>
    {submitting ? 'Re-authenticating…' : 'Re-authenticate'}
  </button>
  {#if error}
    <p class="reauth-error" id="reauth-error" role="alert">{error}</p>
  {/if}
</form>

<style>
  .reauth-form {
    display: grid;
    gap: 0.4rem;
    margin: 0.5rem 0 0;
    padding: 0.6rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(190, 130, 70, 0.55);
    background: rgba(190, 130, 70, 0.1);
  }

  .reauth-label {
    font-size: 0.8rem;
    font-weight: 600;
    opacity: 0.85;
  }

  .reauth-form input {
    padding: 0.45rem 0.55rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: inherit;
    color: inherit;
    font: inherit;
  }

  .reauth-form button {
    padding: 0.45rem 0.75rem;
    border-radius: 0.375rem;
    border: 1px solid rgba(127, 127, 127, 0.45);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .reauth-form button:hover:not(:disabled),
  .reauth-form button:focus-visible:not(:disabled) {
    background: rgba(127, 127, 127, 0.25);
  }

  .reauth-form button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .reauth-error {
    margin: 0;
    font-size: 0.85rem;
    color: inherit;
    opacity: 0.9;
  }
</style>
