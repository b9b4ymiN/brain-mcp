import { expect, test } from '@playwright/test'

// Proves the E2E harness works end-to-end against the production build:
// the static index loads (title) and the Svelte bundle mounts (heading).
test('console shell loads and mounts', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveTitle('Brain Console')
  await expect(
    page.getByRole('heading', { level: 1, name: 'Brain Console' }),
  ).toBeVisible()
})
