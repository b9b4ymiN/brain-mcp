import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'
import pkg from './package.json' with { type: 'json' }

// Console dev/build config.
//
// In dev, `/api` and `/events` are proxied to the Rust HTTP server on
// `127.0.0.1:8080` (`cargo run -- serve --http :8080`), so the browser's
// same-origin requests carry the `brain_console_session` cookie without
// CORS gymnastics. In production, the Rust server serves the built bundle
// itself (same-origin).
//
// `appType: 'spa'` makes `vite preview` and the dev server fall back to
// `index.html` for unknown paths so hash-routing deep links (e.g.
// `/operations`) don't 404 on a refresh.
const BACKEND = 'http://127.0.0.1:8080'

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  appType: 'spa',
  // Bake the console package version into the bundle so the footer BUILD
  // cell can show a real number instead of the hardcoded 'v0.1' that
  // drifted out of sync. Tree-shaken to the actual string at build time.
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  build: {
    outDir: 'dist',
  },
  server: {
    proxy: {
      '/api': {
        target: BACKEND,
        changeOrigin: true,
        secure: false,
      },
      '/events': {
        target: BACKEND,
        changeOrigin: true,
        secure: false,
      },
    },
  },
})
