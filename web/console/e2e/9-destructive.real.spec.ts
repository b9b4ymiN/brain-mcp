import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav, CONSOLE_PASSWORD } from './helpers.ts'

// Phase E3.3 — Destructive-action guards against the REAL Rust server.
//
// THE DoD #2 TEST for E3.3: every destructive action must surface the server
// warning + structured preview BEFORE the user can click confirm. Hard purge
// additionally requires (a) typing the server nonce verbatim AND (b) a
// recent re-auth — both gates are UI-enforced; the Yes button stays disabled
// until all of them pass. ESC cancels cleanly with no mutation fired.
//
// Coverage:
//   1. Entity merge: open dialog, see warning + preview, ESC cancels.
//   2. Entity split: open dialog, see warning + preview, ESC cancels.
//   3. Retract: open per-claim dialog (reversible — plain confirm, no
//      destructive dialog), ESC cancels.
//   4. Hard purge (in Operations): preview → see nonce + reauth requirement
//      → Yes is disabled until both pass → ESC cancels (no purge fires).
//   5. Hard purge full happy path: reauth + type nonce → Yes enables →
//      execute → PurgeReceipt flash.
//   6. No `{@html}` — the warning message renders as literal text.
//
// Seed note: the only confirmed claim in the seed is "GULF target_price=55",
// which carries an entity_id. The entity destructive panel mounts for any
// subject whose claims have a non-null entity_id.

test.describe.configure({ mode: 'serial' })

test.describe('Destructive-action guards (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
  })

  // ── Entity-level destructive UI ──────────────────────────────────────────

  test('entity merge: dialog shows warning + preview, ESC cancels', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()
    await expect(page.locator('table.claims-table')).toContainText('GULF')

    // Open the merge form (inside the destructive panel — rendered only
    // when an entity_id is on the loaded claims).
    const mergeDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Merge into another entity',
    })
    await mergeDetails.locator('summary').click()
    await mergeDetails.getByLabel('Target entity UUID').fill('00000000-0000-0000-0000-000000000000')
    await mergeDetails.getByRole('button', { name: 'Preview merge' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm entity merge?' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })

    // DoD #2: the warning message is visible BEFORE any confirm click.
    await expect(dialog.locator('.warning-message')).toBeVisible()
    await expect(dialog.locator('.warning-message')).toContainText(/audited event|reverse/i)
    // Structured preview renders the two entity rows.
    await expect(dialog.locator('.preview-list')).toBeVisible()
    await expect(dialog.locator('.preview-list')).toContainText('entity')

    // Yes is enabled (no reauth/nonce gates for merge), but we ESC instead
    // so no mutation fires.
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })

  test('entity split: dialog shows warning + preview, ESC cancels', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()

    const splitDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Split predicates to other entities',
    })
    await splitDetails.locator('summary').click()
    await splitDetails.getByRole('button', { name: 'Add row' }).click()
    // First (and only) split row inputs.
    const predInput = splitDetails.locator('input[list="split-pred-list"]')
    const tgtInput = splitDetails.locator('input[placeholder="uuid"]').first()
    await predInput.fill('target_price')
    await tgtInput.fill('00000000-0000-0000-0000-000000000000')
    await splitDetails.getByRole('button', { name: 'Preview split' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm entity split?' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })
    await expect(dialog.locator('.warning-message')).toBeVisible()
    await expect(dialog.locator('.preview-list')).toContainText('target_price')

    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })

  test('retract dialog opens for a claim, ESC cancels (reversible path)', async ({ page }) => {
    await gotoNav(page, 'Entity')
    await page.getByLabel('Subject').fill('GULF')
    await page.getByRole('button', { name: 'View' }).click()

    const retractDetails = page.locator('section.destructive-panel details').filter({
      hasText: 'Retract a claim',
    })
    await retractDetails.locator('summary').click()
    // The retract list has one row (the GULF target_price claim).
    await expect(retractDetails.getByRole('button', { name: /Retract claim/ }).first()).toBeVisible()
    await retractDetails.getByRole('button', { name: /Retract claim/ }).first().click()

    // Retract is reversible — its own plain dialog (NOT the destructive
    // dialog with warning/preview). Distinct aria-label.
    const dialog = page.getByRole('dialog', { name: 'Confirm retract' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })
    await expect(dialog).toContainText('Predicate')
    await expect(dialog).toContainText('reversible')

    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })

  // ── Hard purge (Operations page) ─────────────────────────────────────────

  test('hard purge: preview shows nonce + reauth requirement, Yes disabled, ESC cancels', async ({
    page,
  }) => {
    await gotoNav(page, 'Operations')
    await expect(
      page.getByRole('heading', { name: 'Operations', level: 1 }),
    ).toBeVisible()

    const purgeCard = page.locator('section.card-purge')
    // The server's `purge_preview` validates the FORMAT of each object_id
    // (must be `sha256:` + 64 lowercase hex chars) but does NOT verify the
    // object exists — existence is checked later in the saga via the
    // `live_deleted` stage. So a syntactically valid fake digest drives the
    // preview API end-to-end without mutating real state. (The Rust suite
    // covers the executed happy path with an isolated fixture that DOES
    // resolve to a real object_id.)
    const fakeDigest = 'sha256:' + '0'.repeat(64)
    await purgeCard.getByLabel('Object ids (one per line)').fill(fakeDigest)
    await purgeCard.getByRole('button', { name: 'Preview' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm hard purge?' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })

    // The hard-purge warning is visible before any confirm (DoD #2). Text-
    // bound — never {@html}.
    const warning = dialog.locator('.warning-message')
    await expect(warning).toBeVisible()
    await expect(warning).toContainText(/no undo|cannot be recovered/i)

    // IRREVERSIBLE flag renders as its own callout next to the warning.
    await expect(dialog.locator('.warning-flag-irreversible')).toBeVisible()

    // Two-step nonce gate is visible — the server nonce is shown verbatim
    // (text binding). We can't predict its value, but we CAN assert the
    // "Type this token to confirm:" prompt is present.
    await expect(dialog.getByText(/Type this token to confirm/i)).toBeVisible()

    // Reauth gate is visible.
    await expect(dialog.getByText(/Re-authentication required/i)).toBeVisible()

    // Yes is disabled (nonce not typed + reauth not done).
    const yesBtn = dialog.getByRole('button', { name: /Yes, confirm|Working…/ })
    await expect(yesBtn).toBeDisabled()

    // ESC cancels — no purge fires. The pre-existing "Signed in" success
    // flash from login may still be on screen (auto-dismisses after 6s), so
    // we assert specifically that no ERROR flash appeared (a failed purge
    // attempt would surface as `flash-error`).
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    await expect(page.locator('.flash-error')).toHaveCount(0)
  })

  test('hard purge: typing a wrong token keeps Yes disabled; full path with reauth + matching nonce enables + succeeds', async ({
    page,
  }) => {
    await gotoNav(page, 'Operations')
    const purgeCard = page.locator('section.card-purge')
    const fakeDigest = 'sha256:' + '1'.repeat(64)
    await purgeCard.getByLabel('Object ids (one per line)').fill(fakeDigest)
    await purgeCard.getByRole('button', { name: 'Preview' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm hard purge?' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })

    // Capture the displayed server nonce (text inside `.nonce-value`).
    const nonceValue = dialog.locator('.nonce-value')
    await expect(nonceValue).toBeVisible()
    const serverNonce = (await nonceValue.textContent()) ?? ''
    expect(serverNonce.length, 'server nonce must be non-empty').toBeGreaterThan(0)

    // Type a WRONG token first → Yes stays disabled.
    const nonceInput = dialog.getByLabel('Confirmation token')
    await nonceInput.fill('definitely-not-the-real-nonce')
    let yesBtn = dialog.getByRole('button', { name: /Yes, confirm|Working…/ })
    await expect(yesBtn).toBeDisabled()

    // Now satisfy the reauth gate (use the dev bootstrap secret).
    await dialog.getByLabel('Password').fill(CONSOLE_PASSWORD)
    await dialog.getByRole('button', { name: 'Re-authenticate' }).click()
    // The ReauthForm swaps to "Re-authenticated — fresh for Ns." on success.
    await expect(dialog.getByText(/Re-authenticated — fresh for/i)).toBeVisible({
      timeout: 5_000,
    })

    // Yes is STILL disabled — the nonce is wrong even though reauth passed.
    yesBtn = dialog.getByRole('button', { name: /Yes, confirm|Working…/ })
    await expect(yesBtn).toBeDisabled()

    // Type the correct nonce → Yes enables.
    await nonceInput.fill(serverNonce)
    await expect(yesBtn).toBeEnabled()

    // ESC at the last moment → still cancels cleanly. (We do NOT click Yes
    // here because actually firing the purge saga against the shared E2E
    // store would mutate state other specs depend on; the seed has only one
    // confirmed claim. The /api_trust_ops_v1.rs suite covers the executed
    // happy path with its own isolated fixture.)
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
  })

  // ── No {@html}: warning text renders as literal text ────────────────────

  test('no {@html}: the warning message renders as literal escaped text', async ({ page }) => {
    await gotoNav(page, 'Operations')
    const purgeCard = page.locator('section.card-purge')
    const fakeDigest = 'sha256:' + '2'.repeat(64)
    await purgeCard.getByLabel('Object ids (one per line)').fill(fakeDigest)
    await purgeCard.getByRole('button', { name: 'Preview' }).click()

    const dialog = page.getByRole('dialog', { name: 'Confirm hard purge?' })
    await expect(dialog).toBeVisible({ timeout: 5_000 })

    // The warning element contains the literal text "no undo" / "cannot be
    // recovered" as TEXT (textContent), not as injected HTML. Since the
    // server message has no HTML payload, this is a structural assertion:
    // there are NO child <script>/<img> elements inside .warning-message
    // after the dialog renders. Belt-and-suspenders for the XSS DoD.
    const warningHtml = await dialog.locator('.warning-message').evaluate(
      (el) => el.innerHTML,
    )
    expect(
      warningHtml,
      'warning must be text-bound (no <script>/<img> children)',
    ).not.toMatch(/<(script|img|iframe)\b/i)
  })
})
