import { expect, test } from '@playwright/test'

// Proves the SPA shell boots against the production build.
//
// The app starts on the login screen (no Rust backend in this task's
// webServer config — only `vite preview` of the static bundle). We assert:
//   1. The static index loads (title).
//   2. The Svelte bundle mounts (brand h1).
//   3. The login form is visible with its own heading + secret input.
// Logged-in navigation (the 5 nav links) is exercised in E1.4 once a real
// backend is wired into the webServer config.
test('console shell loads and shows the login form', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveTitle('Brain Console')
  await expect(
    page.getByRole('heading', { level: 1, name: 'Brain Console' }),
  ).toBeVisible()
  await expect(
    page.getByRole('heading', { level: 2, name: 'Sign in' }),
  ).toBeVisible()
  await expect(page.getByLabel('Bootstrap secret')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible()
})
