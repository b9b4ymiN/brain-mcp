import { expect, test } from '@playwright/test'
import { loginAsConsole, gotoNav } from './helpers.ts'

// Phase E1.4 — the CRITICAL XSS / CSP test (Phase E1 DoD #4).
//
// The seed plants a proposal whose SUBJECT and VALUE are XSS payloads:
//   subject = `<img src=x onerror=alert(1)>`
//   value  = `"<script>alert('xss')</script>"`
// Every Console binding uses Svelte's `{value}` (auto-escaped TEXT) — there is
// no `{@html}` anywhere (verified by a separate grep in the DoD checklist).
// So the payloads MUST render as literal visible text, never execute. We
// assert three independent properties:
//
//   1. PAYLOAD AS TEXT: the subject + value strings appear verbatim in the
//      rendered DOM (i.e. they were HTML-escaped on the way out). We check
//      the escaped text shows up — `<img ...>` rendered as TEXT contains the
//      literal `<`/`>` characters in `element.textContent`, whereas an
//      actual `<img>` element would NOT match `toHaveText(/<img/)`).
//   2. NO ALERT DIALOG: an `onerror=alert(1)` or `<script>` that DID execute
//      would open a dialog. We attach a `page.on('dialog')` handler that
//      fails the test if any dialog appears during the whole flow.
//   3. CSP HEADER: the HTML response at `/` carries a strict CSP —
//      `script-src 'self'` and NO `unsafe-inline` — so even an escaped-but-
//      somehow-injected `<script>` would be blocked by the browser. We fetch
//      `/` raw and assert the header.

const XSS_SUBJECT = '<img src=x onerror=alert(1)>'
const XSS_VALUE = "<script>alert('xss')</script>"

test.describe('XSS + CSP (real backend)', () => {
  test('CSP header on the HTML response blocks inline scripts', async ({ request }) => {
    // Direct fetch of the SPA shell. The Rust static router stamps the CSP
    // uniformly on every response (success AND 404) — see src/api.rs
    // `static_router` + `CONSOLE_CSP`.
    const resp = await request.get('/')
    // CSP is present and strict.
    const csp = resp.headers()['content-security-policy']
    expect(csp, 'CSP header must be present on /').toBeTruthy()
    expect(csp, "CSP must include script-src 'self'").toContain("script-src 'self'")
    expect(csp, 'CSP must NOT allow unsafe-inline').not.toContain('unsafe-inline')
    expect(csp, 'CSP must block object-src (no <object>/<embed>)').toContain(
      "object-src 'none'",
    )
    expect(csp, 'CSP must pin base-uri (no <base> injection)').toContain("base-uri 'self'")
  })

  test('seeded XSS payloads render as literal text and never execute', async ({ page }) => {
    // FAIL LOUDLY if any alert/confirm dialog opens during this test — that
    // would mean a payload executed. Attach before any navigation.
    let dialogOpened = false
    page.on('dialog', (dialog) => {
      dialogOpened = true
      // Dismiss so the test can continue to its assertions rather than hang.
      void dialog.dismiss()
    })

    await loginAsConsole(page)

    // ── Inbox: the XSS proposal row shows the payload subject as TEXT ──
    await gotoNav(page, 'Inbox')
    const list = page.getByRole('list', { name: 'Pending proposals' })
    await expect(list).toBeVisible()

    // The subject `<img src=x onerror=alert(1)>` must appear as TEXT in the
    // list. If it had been injected as HTML, it would create a broken <img>
    // element whose textContent is EMPTY — `getByText` would not find the
    // literal `<img` characters. So this assertion is only satisfiable if
    // the binding escaped it.
    const subjectText = list.getByText(XSS_SUBJECT)
    await expect(subjectText).toBeVisible()

    // ── Inbox proposal-head: the VALUE payload renders as TEXT too ──
    // The proposal-head binds `formatValue(p.value)` as text (Inbox.svelte).
    // The XSS row's value is `<script>alert('xss')</script>`; if it had been
    // injected as HTML it would create a <script> element (not executed on
    // insert, but its textContent would be the JS body, and getByText would
    // NOT match the literal tag). A literal match proves text binding.
    // (The Entity page can't show this — `getSubject` returns CONFIRMED
    // claims only, and the XSS proposal is pending.)
    await expect(list.getByText(XSS_VALUE)).toBeVisible()

    // ── Final assertion: no dialog ever opened ──
    await expect.poll(() => dialogOpened, { message: 'no alert dialog should open' }).toBe(false)
    // Explicit belt-and-suspenders message if a dialog did slip through.
    expect(dialogOpened, 'an XSS payload executed and opened a dialog').toBe(false)
  })

  test('no dialog opens while reviewing the XSS row (approve/reject path)', async ({ page }) => {
    let dialogOpened = false
    const onDialog = (dialog: { dismiss: () => Promise<void> }): void => {
      dialogOpened = true
      void dialog.dismiss()
    }
    page.on('dialog', onDialog)

    await loginAsConsole(page)
    await gotoNav(page, 'Inbox')

    // Open the XSS row's detail. The evidence excerpt + prior-claims note
    // render the subject as text.
    const xssRow = page.locator('.proposal-head').filter({ hasText: XSS_SUBJECT }).first()
    await xssRow.click()
    await expect(page.getByRole('heading', { name: 'Evidence', level: 3 })).toBeVisible()

    // Reject (distinct from the inbox-review spec which uses GULF/AAPL/PTT).
    await page.getByRole('button', { name: 'Reject' }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm reject' })
    await expect(dialog).toBeVisible()
    // The XSS subject appears in the dialog summary as TEXT too.
    await expect(dialog).toContainText(XSS_SUBJECT)
    await dialog.getByRole('button', { name: 'Yes, confirm' }).click()
    await expect(page.locator('.flash')).toContainText(/Rejected proposal/)

    expect(dialogOpened, 'XSS payload executed during the review flow').toBe(false)
    page.off('dialog', onDialog)
  })
})
