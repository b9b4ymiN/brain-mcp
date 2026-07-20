/// <reference types="svelte" />
/// <reference types="vite/client" />

/**
 * Build-time constants injected by Vite's `define` option (see vite.config.ts).
 *
 * `__APP_VERSION__` is the console package version (from package.json),
 * baked into the bundle so the footer BUILD readout can show a real number
 * without a runtime fetch or a hardcoded literal that drifts out of sync.
 */
declare const __APP_VERSION__: string

/**
 * Global error trap wired in main.ts and consumed by ErrorBoundary in
 * App.svelte. Set by App on mount; called by the window error/unhandledrejection
 * listeners. Optional because it's only set after the SPA boots.
 */
interface Window {
  __brainTrapError?: (err: unknown) => void
}
