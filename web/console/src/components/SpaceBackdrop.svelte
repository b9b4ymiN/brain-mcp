<script lang="ts">
  /**
   * SpaceBackdrop — the global cosmic atmosphere mounted once in App.svelte.
   *
   * The Observer's Deck: every page is the same console looking out at a
   * different sector. Four layers (bottom → top):
   *
   *   1. `.space-void`     — base cool-void background (token).
   *   2. `.space-nebula`   — three radial gradients (deep indigo, plasma
   *                          amber, holo-cyan/teal aurora). Slow breathe.
   *   3. `.stars--far`     — small dim stars (320 desktop / hidden mobile).
   *   4. `.stars--near`    — larger brighter stars (80, twinkle).
   *
   * Two interaction layers (desktop, pointer:fine only, disabled on reduced
   * motion):
   *
   *   5. `.space-parallax` — wrapper on the nebula + stars that translates
   *                          by a few px on mousemove (depth illusion).
   *                          Lerp'd via rAF so it lags slightly — that's
   *                          what sells the parallax.
   *   6. cursor trail      — stardust particles left behind the cursor.
   *                          Rendered to a single canvas via rAF; capped at
   *                          ~40 particles; dies after 1s.
   *
   * Performance: the box-shadow star trick paints once (deterministic via
   * Mulberry32 PRNG — no bundle hash churn). Parallax uses `transform`
   * only (GPU-composited). The cursor canvas is 1 layer, draw call per
   * particle, cheap. Mobile drops the far-star layer and disables the
   * parallax + cursor entirely.
   *
   * Reduced motion: parallax, cursor trail, nebula breathe, and star
   * twinkle all collapse. Stars stay visible (static), nebula becomes a
   * still gradient field. WCAG 2.3.3 compliant.
   */
  import { onMount, onDestroy } from 'svelte'

  // ── Star generation (deterministic via seeded PRNG) ────────────────────
  function makeStars(count: number, maxSize: number, seed: number): string {
    let s = seed >>> 0
    const rand = (): number => {
      s = (s + 0x6d2b79f5) >>> 0
      let t = s
      t = Math.imul(t ^ (t >>> 15), t | 1)
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296
    }
    const shadows: string[] = []
    for (let i = 0; i < count; i++) {
      const x = Math.round(rand() * 1000) / 10 // 0–100vw
      const y = Math.round(rand() * 1000) / 10 // 0–100vh
      const opacity = (0.45 + rand() * 0.55).toFixed(2)
      const spread = (rand() * maxSize).toFixed(2)
      shadows.push(`${x}vw ${y}vh 0 ${spread}px rgba(255, 255, 255, ${opacity})`)
    }
    return shadows.join(', ')
  }

  let farStars = makeStars(320, 1.0, 0x5f3759df)
  let nearStars = makeStars(80, 2.4, 0x82a5b34c)

  // ── Parallax + cursor trail (desktop only, reduced-motion off) ────────
  let parallaxEl: HTMLElement | null = null
  let cursorCanvas: HTMLCanvasElement | null = null
  let cursorCtx: CanvasRenderingContext2D | null = null
  let parallaxEnabled = false
  let cursorEnabled = false

  // Target + current offsets (lerp makes the parallax feel weighty).
  let targetX = 0
  let targetY = 0
  let currentX = 0
  let currentY = 0
  let rafId = 0

  // Cursor particle pool (fixed-size, recycled).
  interface Particle {
    x: number
    y: number
    vx: number
    vy: number
    life: number // 0..1, 1 = just spawned
    size: number
  }
  const MAX_PARTICLES = 40
  const particles: Particle[] = []
  let lastSpawn = 0

  function onMouseMove(e: MouseEvent): void {
    if (!parallaxEnabled) return
    // Normalize to -1..1 around viewport center.
    targetX = (e.clientX / window.innerWidth - 0.5) * 2
    targetY = (e.clientY / window.innerHeight - 0.5) * 2
    if (cursorEnabled) spawnParticle(e.clientX, e.clientY)
  }

  function spawnParticle(x: number, y: number): void {
    // Throttle spawn so a stationary cursor doesn't dump particles.
    const now = performance.now()
    if (now - lastSpawn < 28) return
    lastSpawn = now
    // Recycle the oldest dead particle, or push if under cap.
    const p: Particle = {
      x,
      y,
      vx: (Math.random() - 0.5) * 0.4,
      vy: (Math.random() - 0.5) * 0.4 - 0.2, // slight upward drift
      life: 1,
      size: 1 + Math.random() * 1.5,
    }
    if (particles.length >= MAX_PARTICLES) {
      particles.shift()
    }
    particles.push(p)
  }

  function loop(): void {
    // Lerp parallax (≈0.06 factor → smooth, weighty).
    currentX += (targetX - currentX) * 0.06
    currentY += (targetY - currentY) * 0.06
    if (parallaxEl) {
      // ±14px translate on the nebula; ±7px on stars (separate element
      // would be cleaner but a single transform reads close enough at
      // this scale, and keeps it to one composited layer).
      parallaxEl.style.transform = `translate3d(${(-currentX * 14).toFixed(2)}px, ${(-currentY * 14).toFixed(2)}px, 0)`
    }

    // Cursor particle render.
    if (cursorCtx && cursorCanvas) {
      cursorCtx.clearRect(0, 0, cursorCanvas.width, cursorCanvas.height)
      for (let i = particles.length - 1; i >= 0; i--) {
        const p = particles[i]
        p.life -= 0.018 // ~1s lifespan at 60fps
        if (p.life <= 0) {
          particles.splice(i, 1)
          continue
        }
        p.x += p.vx
        p.y += p.vy
        // Amber-tinted stardust; alpha tied to life.
        const alpha = p.life * 0.7
        cursorCtx.beginPath()
        cursorCtx.arc(p.x, p.y, p.size * p.life, 0, Math.PI * 2)
        cursorCtx.fillStyle = `oklch(0.82 0.14 75 / ${alpha})`
        cursorCtx.shadowColor = 'oklch(0.82 0.14 75 / 0.6)'
        cursorCtx.shadowBlur = 6
        cursorCtx.fill()
      }
    }

    rafId = requestAnimationFrame(loop)
  }

  function onResize(): void {
    if (!cursorCanvas) return
    cursorCanvas.width = window.innerWidth
    cursorCanvas.height = window.innerHeight
  }

  function onVisibilityChange(): void {
    // Pause the rAF loop when the tab is hidden; resume on return. Saves
    // battery + CPU on background tabs (the cursor trail would otherwise
    // keep churning particles that no one is looking at).
    if (document.hidden) {
      if (rafId) {
        cancelAnimationFrame(rafId)
        rafId = 0
      }
    } else if ((parallaxEnabled || cursorEnabled) && !rafId) {
      rafId = requestAnimationFrame(loop)
    }
  }

  onMount(() => {
    // Capability gate: pointer:fine + no reduced motion.
    const finePointer =
      typeof window !== 'undefined' &&
      window.matchMedia('(pointer: fine)').matches
    const reducedMotion =
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    parallaxEnabled = finePointer && !reducedMotion
    cursorEnabled = finePointer && !reducedMotion

    if (parallaxEnabled || cursorEnabled) {
      window.addEventListener('mousemove', onMouseMove, { passive: true })
      window.addEventListener('resize', onResize)
      document.addEventListener('visibilitychange', onVisibilityChange)
      if (cursorCanvas) {
        cursorCtx = cursorCanvas.getContext('2d')
        onResize()
      }
      rafId = requestAnimationFrame(loop)
    }
  })

  onDestroy(() => {
    window.removeEventListener('mousemove', onMouseMove)
    window.removeEventListener('resize', onResize)
    document.removeEventListener('visibilitychange', onVisibilityChange)
    if (rafId) cancelAnimationFrame(rafId)
  })
</script>

<div class="space-backdrop" aria-hidden="true">
  <div class="space-void"></div>
  <div class="space-parallax" bind:this={parallaxEl}>
    <div class="space-nebula"></div>
    <div class="space-stars space-stars--far" style="--star-shadows: {farStars}"></div>
    <div class="space-stars space-stars--near" style="--star-shadows: {nearStars}"></div>
  </div>
  <!-- Cursor stardust trail (desktop only). Sits above the backdrop,
       below page content (z-index: 1). pointer-events: none. -->
  <canvas bind:this={cursorCanvas} class="space-cursor" aria-hidden="true"></canvas>
</div>

<style>
  .space-backdrop {
    position: fixed;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    overflow: hidden;
  }

  .space-void {
    position: absolute;
    inset: 0;
    background: var(--surface-body);
  }

  /* Parallax wrapper — the only element we transform. Will-change so the
   * browser promotes it to its own layer once (avoids per-frame promote). */
  .space-parallax {
    position: absolute;
    inset: -16px; /* overscan so the ±14px translate never reveals an edge */
    will-change: transform;
  }

  /* Three nebula radial gradients, deep-space palette. Composed in one
   * background declaration so the browser paints them as one layer. */
  .space-nebula {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(ellipse 80% 60% at 15% 20%,
        oklch(0.45 0.15 280 / 0.45) 0%,
        transparent 55%),
      radial-gradient(ellipse 60% 50% at 85% 75%,
        oklch(0.65 0.16 75 / 0.28) 0%,
        transparent 50%),
      radial-gradient(ellipse 70% 55% at 50% 50%,
        oklch(0.60 0.12 200 / 0.20) 0%,
        transparent 60%);
    animation: space-breathe 24s var(--ease-breathe) infinite;
    will-change: opacity;
  }

  @keyframes space-breathe {
    0%, 100% { opacity: 0.75; transform: scale(1); }
    50%      { opacity: 1;    transform: scale(1.03); }
  }

  .space-stars {
    position: absolute;
    top: 0;
    left: 0;
    width: 1px;
    height: 1px;
    background: transparent;
    box-shadow: var(--star-shadows);
    border-radius: 50%;
  }

  .space-stars--far {
    opacity: 0.6;
  }

  .space-stars--near {
    animation: space-twinkle 4s var(--ease-breathe) infinite;
  }

  @keyframes space-twinkle {
    0%, 100% { opacity: 0.85; }
    50%      { opacity: 0.4; }
  }

  /* Cursor stardust canvas. Fixed full-viewport, but only painted when
   * particles are alive (cheap otherwise — clearRect on empty pool). */
  .space-cursor {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    z-index: 1;
    pointer-events: none;
  }

  /* Mobile: drop far stars + nebula breathe is slower. Parallax + cursor
   * already disabled via JS capability gate (pointer:fine). */
  @media (max-width: 48rem) {
    .space-stars--far {
      display: none;
    }
    .space-nebula {
      animation-duration: 36s;
    }
  }

  /* Reduced motion: kill breathe, twinkle, parallax. The JS loop already
   * short-circuits, but the CSS animations need this too. Stars stay. */
  @media (prefers-reduced-motion: reduce) {
    .space-nebula,
    .space-stars--near {
      animation: none;
    }
    .space-parallax {
      transform: none !important;
    }
    .space-cursor {
      display: none;
    }
  }
</style>
