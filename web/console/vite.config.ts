import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

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
