/**
 * Galaxy node color assignment — hash-based stellar palette.
 *
 * Replaces the old fixed `KIND_COLOR` lookup (whose keys `source` / `concept` /
 * `entity` almost never matched live `claim_kind` values, leaving every node
 * on the fallback white). Instead, each node's **domain** string is hashed
 * into a curated 12-color palette inspired by real stellar spectral types
 * (O→M), giving the galaxy the multi-hued look of a real starfield while
 * staying within the dark cosmic register.
 *
 * Design constraints honored:
 *   - **One Voice Rule**: amber (`#f5b342`) is reserved for `decision` /
 *     `project_decision` kinds only (rare in live data). Stars use other hues.
 *   - **Cool Void Rule**: every palette color sits at L 0.68–0.86, C 0.02–0.12
 *     so it reads as starlight against the void, not neon.
 *   - **Auto-scaling**: new domains created by future LLM extraction runs
 *     (e.g. "ESG", "macro", "supply_chain") get a deterministic color with
 *     zero code changes — the hash distributes them across the 12 slots.
 *   - **Case-sensitive**: `financial` ≠ `Finance` ≠ `Financial` hash to
 *     different slots, maximizing visible variety from existing variants.
 */

/**
 * 12-color stellar palette spanning the full spectrum (blue → white →
 * yellow → orange → red → violet). Hand-tuned in OKLCH, stored as hex
 * because Three.js `Color` and canvas 2D `fillStyle` don't read CSS custom
 * properties.
 *
 * Index order follows spectral temperature (hot→cool) then wraps to
 * non-thermal accents (rose, violet) for variety beyond real physics.
 */
export const STELLAR_PALETTE = [
  '#a8c0f0', // 0  B-type  — blue-white (Rigel-like)
  '#b8d8f0', // 1  A-type  — pale azure (Sirius-like)
  '#a0d8e8', // 2          — ice cyan
  '#90d0d8', // 3          — teal-cyan
  '#c8d8f0', // 4          — cool white
  '#e0e0e8', // 5  F-type  — neutral white (Procyon-like) — default for empty domain
  '#f0e8c8', // 6          — warm white
  '#f0d898', // 7  G-type  — pale gold (Sun-like)
  '#f0b878', // 8          — warm amber-orange
  '#f0a070', // 9  K-type  — orange (Arcturus-like)
  '#e89098', // 10 M-type  — rose-coral (Betelgeuse-like)
  '#c0a0d8', // 11         — lavender violet (non-thermal accent for variety)
] as const

/**
 * Kind-level overrides that take priority over domain hashing.
 *
 * `decision` / `project_decision` → amber (the One Voice accent) — these
 * nodes represent operator-authored commitments and deserve the brand color.
 * `error` → danger red — contradictions / corrupt claims.
 *
 * All other kinds (`external_fact`, `inference`, `user_assertion`,
 * `financial_metric`, `preference`, …) fall through to domain-based coloring.
 */
const KIND_OVERRIDE: Record<string, string> = {
  decision: '#f5b342', // Stellar Amber — One Voice accent
  project_decision: '#f5b342',
  error: '#e85a5a', // Danger
}

/**
 * djb2 string hash (Daniel J. Bernstein). Fast, well-distributed for short
 * strings like domain tags. Returns a non-negative integer.
 */
function hashStr(s: string): number {
  let h = 5381
  for (let i = 0; i < s.length; i++) {
    h = ((h << 5) + h + s.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

/**
 * The minimum shape a galaxy node payload needs for color assignment.
 * Matches `GalaxyNode.kind` + `GalaxyNode.domain` from the `/api/v1/galaxy`
 * wire response.
 */
export interface StarColorInput {
  kind: string
  domain: string
}

/**
 * Resolve the display color for a galaxy node.
 *
 * Priority:
 *   1. `KIND_OVERRIDE` — semantic kinds (`decision` → amber, `error` → red).
 *   2. Domain hash → `STELLAR_PALETTE[idx]` (12-way, deterministic).
 *   3. Empty / absent domain → neutral white (`STELLAR_PALETTE[5]`).
 */
export function starColorFor(raw: StarColorInput): string {
  // Kind override takes priority (semantic: decision=amber, error=danger).
  if (KIND_OVERRIDE[raw.kind]) return KIND_OVERRIDE[raw.kind]
  // Domain hash → palette index. Empty domain → neutral white (idx 5).
  if (!raw.domain) return STELLAR_PALETTE[5]
  return STELLAR_PALETTE[hashStr(raw.domain) % 12]
}
