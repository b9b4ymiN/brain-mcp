import { expect, test } from '@playwright/test'
import { loginAsConsole } from './helpers.ts'

// Phase E1.4 — Home page against the REAL Rust server.
//
// The webServer seed (`examples/seed_console_e2e.rs`) plants these pending
// proposals:
//   GULF target_price=58, PTT target_price=62, AAPL sector="tech",
//   <XSS payload> target_price="<script>..."
// Home renders `api.inbox()` (the only query-less read endpoint) as a "Recent
// claims" list capped at 5 rows. We assert the seeded proposals surface there
// after a real login, proving the Home → real-API wiring end to end.
//
// DoD coverage: Phase E1 DoD #1 (real-API wiring for Home) + #5 (these specs
// ride on cargo-test-green Rust routes).

test.describe('Home (real backend)', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsConsole(page)
    // The login handler navigates to #/home; ensure we're there.
    await expect(page).toHaveURL(/#\/home/)
  })

  test('renders the brand and quick-search form', async ({ page }) => {
    // Two "Brain Console" h1s exist (shell header banner + Home page hero).
    // Scope to the main region — the Home page's own h1 — to disambiguate.
    await expect(
      page.getByRole('main').getByRole('heading', { name: 'Brain Console', level: 1 }),
    ).toBeVisible()
    await expect(page.getByLabel('Quick search')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Search' })).toBeVisible()
  })

  test('shows seeded pending proposals under "Recent claims"', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Recent claims' })).toBeVisible()
    // GULF, PTT, AAPL must each appear as a recent-claim row. We assert by
    // subject text (the proposal-row button binds subject as TEXT).
    const gulf = page.getByRole('button', { name: /GULF/i }).first()
    const ptt = page.getByRole('button', { name: /PTT/i }).first()
    const aapl = page.getByRole('button', { name: /AAPL/i }).first()
    await expect(gulf).toBeVisible()
    await expect(ptt).toBeVisible()
    await expect(aapl).toBeVisible()
    // Predicate text is rendered too — pin the predicate binding.
    await expect(gulf).toContainText('target_price')
    await expect(aapl).toContainText('sector')
  })

  test('clicking a recent proposal navigates to the Entity page', async ({ page }) => {
    // The proposal-row onclick stages the subject and navigates to /entity.
    await page.getByRole('button', { name: /GULF/i }).first().click()
    await expect(page).toHaveURL(/#\/entity/)
    // Entity page shows the staged subject in its claims table.
    await expect(page.getByRole('heading', { name: 'Entity', level: 1 })).toBeVisible()
  })
})
