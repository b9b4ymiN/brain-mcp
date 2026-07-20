import { mount } from 'svelte'
import App from './App.svelte'
import './lib/tokens.css'
import './app.css'

/**
 * Global error trap (P0-2 fix, 2026-07-20).
 *
 * Svelte 5 has no built-in error boundary, so any uncaught throw or
 * unhandled rejection in the SPA subtree escapes to the window and blanks
 * the page. App.svelte wraps its entire output in <ErrorBoundary>, which
 * exposes a `trap()` method via `bind:this`. These handlers route uncaught
 * errors into that boundary so the user sees a "Signal lost" panel instead
 * of a white screen.
 */
window.addEventListener('error', (event) => {
  if (event.defaultPrevented) return
  window.__brainTrapError?.(event.error ?? new Error(event.message))
})

window.addEventListener('unhandledrejection', (event) => {
  window.__brainTrapError?.(event.reason)
})

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
