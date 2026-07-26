/**
 * Typed fetch client for the Brain Console HTTP API (mounted at `/api/v1`).
 *
 * Contract mirrors `src/api.rs` EXACTLY — including the field-name
 * divergences between routes:
 *   - `/search` + `/get` use `provenance` + `origin` (NOT `provenance_kind`)
 *   - `/get` uses `kind` (NOT `claim_kind`) and exposes `confidence` (0..1 float)
 *   - `/entity/timeline` returns the full `ClaimView` with `provenance_kind`
 *     (and `claim_kind`, `status`, `confidence_basis_points`, etc.)
 *   - `/inbox` + `/inbox/{id}/evidence` are bare arrays / bare objects
 *
 * CSRF: mutation POSTs (`approve`/`reject`/`supersede`) attach the
 * `X-CSRF-Token` header cached from `login()`. All requests carry
 * `credentials: 'include'` so the `brain_console_session` cookie rides the
 * same-origin proxy (dev) or same-origin prod origin.
 *
 * No external HTTP library, no `any`. Pure `fetch`.
 */

// ── shared primitives ───────────────────────────────────────────────────────

/** A UUID as transmitted on the wire (string). */
export type Uuid = string

/** An RFC 3339 timestamp as transmitted on the wire (string). */
export type IsoTimestamp = string

/** Arbitrary JSON value (matches `serde_json::Value` on the Rust side). */
export type JsonValue = string | number | boolean | null | JsonValue[] | { [k: string]: JsonValue }

/**
 * Uniform error shape: `{ "error": "<code>" }`. Every non-2xx response from
 * the API carries one of the fixed codes below; `ApiError.code` is always
 * populated (falling back to a status-derived hint if the body is missing).
 */
export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string) {
    super(`API ${status}: ${code}`)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

/**
 * Map a numeric HTTP status to the canonical error code, mirroring
 * `ApiError` construction + `map_semantic_error` in `src/api.rs`. Used when
 * the response body cannot be parsed (network blip, empty body).
 */
function codeForStatus(status: number): string {
  switch (status) {
    case 400:
      return 'invalid_request'
    case 401:
      return 'unauthorized'
    case 403:
      return 'forbidden'
    case 404:
      return 'not_found'
    case 409:
      return 'conflict'
    case 422:
      return 'unsupported_inference'
    case 503:
      return 'unavailable'
    case 500:
    default:
      return 'internal_error'
  }
}

// ── CSRF token cache ────────────────────────────────────────────────────────

/** Module-level CSRF token cache; populated by `login()`, cleared by `logout()`. */
let csrfToken: string | null = null

/** Read the cached CSRF token (or `null` if not logged in). Test/debug hook. */
export function getCsrfToken(): string | null {
  return csrfToken
}

/** Overwrite the CSRF token (used by the session store on login/restore). */
export function setCsrfToken(token: string | null): void {
  csrfToken = token
}

// ── request primitive ───────────────────────────────────────────────────────

const API_BASE = '/api/v1'

interface RequestOptions {
  method: 'GET' | 'POST'
  path: string
  query?: Record<string, string | number | undefined>
  body?: unknown
  /** Mutation POSTs set this to attach `X-CSRF-Token`. */
  csrf?: boolean
  /**
   * Optional `AbortSignal` forwarded to `fetch`. Callers that may unmount
   * mid-flight (e.g. `GalaxyGraph`) pass a signal so the in-flight request
   * is aborted on teardown instead of resolving into a destroyed component.
   * The resulting `AbortError` is surfaced as `ApiError`-shaped so callers
   * can detect it by `name === 'AbortError'`.
   */
  signal?: AbortSignal
}

function buildUrl(path: string, query?: RequestOptions['query']): string {
  if (!query) return `${API_BASE}${path}`
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined) params.set(key, String(value))
  }
  const qs = params.toString()
  return qs ? `${API_BASE}${path}?${qs}` : `${API_BASE}${path}`
}

/**
 * Core request routine. Throws `ApiError` on any non-2xx response, parsing
 * the `{error: code}` body when present. Returns the typed JSON payload on
 * success (or `null` for 204 No Content).
 */
async function request<T>(opts: RequestOptions): Promise<T> {
  const headers: Record<string, string> = {
    Accept: 'application/json',
  }
  if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json'
  }
  if (opts.csrf) {
    if (csrfToken === null) {
      // No CSRF token cached → caller hasn't logged in. Mirror the API's own
      // 401 path so UI logic can route back to the login screen uniformly.
      throw new ApiError(401, 'unauthorized')
    }
    headers['X-CSRF-Token'] = csrfToken
  }

  let response: Response
  try {
    response = await fetch(buildUrl(opts.path, opts.query), {
      method: opts.method,
      headers,
      credentials: 'include',
      body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
      signal: opts.signal,
    })
  } catch (cause) {
    // Abort — rethrow verbatim so callers can swallow by `name === 'AbortError'`.
    // We deliberately do NOT wrap aborts in ApiError: the caller already knows
    // it aborted (it held the controller), and the soft 503 mapping below would
    // be a misleading signal.
    if (
      cause instanceof Error &&
      (cause.name === 'AbortError' ||
        (typeof DOMException !== 'undefined' && cause instanceof DOMException && cause.name === 'AbortError'))
    ) {
      throw cause
    }
    // Network failure / proxy down — surface as a 503-style unavailable so
    // callers can show a consistent banner rather than an opaque TypeError.
    throw new ApiError(503, 'unavailable')
  }

  if (response.status === 204) {
    return null as T
  }

  let payload: unknown = null
  const text = await response.text()
  if (text) {
    try {
      payload = JSON.parse(text)
    } catch {
      // Body wasn't JSON — fall through to error mapping if non-2xx.
    }
  }

  if (!response.ok) {
    // Session-expiry recovery: on 401 the cookie session is dead, so drop the
    // cached CSRF token now — otherwise subsequent mutation POSTs keep attaching
    // a stale token and the UI looks "stuck logged in" until reload. The
    // session store separately flips `isLoggedIn` (via its own csrf state) once
    // a page observes the 401 and calls `session.clear()`.
    if (response.status === 401) {
      csrfToken = null
    }
    const code =
      typeof payload === 'object' &&
      payload !== null &&
      'error' in payload &&
      typeof (payload as { error?: unknown }).error === 'string'
        ? ((payload as { error: string }).error)
        : codeForStatus(response.status)
    throw new ApiError(response.status, code)
  }

  return payload as T
}

// ── response shapes (mirror src/api.rs JSON exactly) ────────────────────────

/** `/search` result row. Uses `provenance` (not `provenance_kind`). */
export interface SearchHit {
  claim_id: Uuid
  subject: string
  predicate: string
  value: JsonValue
  domain: string
  origin: string
  provenance: string
  entity_id: Uuid | null
}

/** `GET /search` body. */
export interface SearchResponse {
  query: string
  count: number
  results: SearchHit[]
}

/** `/get` result row. Uses `kind` (not `claim_kind`) + `confidence` (0..1). */
export interface SubjectClaim {
  claim_id: Uuid
  subject: string
  predicate: string
  value: JsonValue
  domain: string
  kind: string
  origin: string
  provenance: string
  entity_id: Uuid | null
  confidence: number
}

/** `GET /get` body. */
export interface GetSubjectResponse {
  subject: string
  count: number
  claims: SubjectClaim[]
}

/**
 * `GET /entity/timeline` row — the full `ClaimView` serialized by
 * `serde_json::to_value(timeline)` in `src/api.rs`. Uses `provenance_kind`
 * + `claim_kind` + `confidence_basis_points`. Bare JSON array on the wire.
 */
export interface ClaimView {
  claim_id: Uuid
  proposal_id: Uuid
  subject: string
  predicate: string
  value: JsonValue
  claim_kind: string
  status: string
  domain: string
  confidence_basis_points: number
  privacy_label: string
  valid_from: IsoTimestamp | null
  valid_to: IsoTimestamp | null
  confirmed_event_seq: number
  provenance_kind: string
  origin: string
  entity_id: Uuid | null
}

/** `GET /inbox` row — a pending proposal. Bare JSON array on the wire. */
export interface ProposalSummary {
  proposal_id: Uuid
  domain: string
  subject: string
  predicate: string
  value: JsonValue
  claim_kind: string
  provenance_kind: string
  submitted_at: IsoTimestamp
  event_seq: number
}

// ── Phase 1.6 — inbox conflict detection ────────────────────────────────────
// Mirrors `src/inbox_conflicts.rs`. A proposal in `/inbox` may participate in
// one or more same-scope conflicts (peer proposals + confirmed claims sharing
// its `(domain, subject, predicate)`). `kind` is the strongest signal found:
// `hard_value` (>0.1% scalar diff) wins over `duplicate` (same value).
//
// NOTE: the Rust endpoint currently emits at most one `ScopeConflict` per
// proposal (one bucket = one conflict), so the Console reads
// `proposal.conflicts[0]` defensively. The array shape keeps the wire format
// forward-compatible if later phases emit multiple kinds per proposal.
/** Conflict kind — `hard_value` (C1, scalar diff > 0.1%) or `duplicate` (C2). */
export type ConflictKind = 'hard_value' | 'duplicate'

/** Peer status — `pending` (proposal) or `confirmed` (claim). */
export type PeerStatus = 'pending' | 'confirmed'

/** One peer (proposal or confirmed claim) that conflicts with the row's proposal. */
export interface ConflictPeer {
  peer_id: Uuid
  peer_status: PeerStatus
  /** Conflicting scalar/value — same JSON shape as `ProposalSummary.value`. */
  value: JsonValue
  submitted_at: IsoTimestamp | null
  /** Relative diff in percent. Only set for `hard_value`; null for `duplicate`. */
  rel_diff_pct: number | null
}

/** One conflict group the row's proposal participates in. */
export interface ScopeConflict {
  proposal_id: Uuid
  kind: ConflictKind
  peers: ConflictPeer[]
}

/** Phase 1.6 `/inbox` row: pending proposal + same-scope conflicts. */
export interface InboxProposal extends ProposalSummary {
  conflicts: ScopeConflict[]
}


/** `GET /inbox/{id}/evidence` body. Bare JSON object. */
export interface EvidenceSummary {
  provenance_kind: string
  excerpt: string | null
  source_id: Uuid | null
  quote_hash: string | null
  // Phase 1.6:
  value_located: boolean
  value_offset: number | null
  value_len: number | null
  excerpt_truncated: boolean
  additional_sources: string[]
}

/** Quality tag produced by `QualityChecker` (Rust `src/quality.rs`). */
export type QualityTagKind =
  | 'duplicate_predicate'
  | 'packed_facts'
  | 'vague_predicate'
  | 'taxonomy_drift'
  | 'confidence_too_high'
  | 'double_bracket'
  | 'kind_mismatch'
  | 'source_claim_mismatch' // Phase 3 (AI only)
  | 'semantic_duplicate' // Phase 3 (AI only)
  | 'provenance_loss' // Phase 3 (AI only)
  // Phase 1.5 — Subject Validator:
  | 'bad_subject_empty'
  | 'bad_subject_structural'
  | 'bad_subject_shape'
  | 'bad_subject_length'
  | 'bad_subject_mixed_script'
  | 'bad_subject_adversarial'
  | 'subject_ambiguous_acronym'
  | 'subject_needs_context'

export type QualitySeverity = 'info' | 'warning' | 'critical'

/** One tag on a proposal. `evidence` is an optional offending-text snippet. */
export interface QualityTag {
  kind: QualityTagKind
  severity: QualitySeverity
  message: string
  evidence?: string
}

/** `GET /inbox/{proposal_id}/ai-review` body. */
export interface AiReviewResponse {
  proposal_id: Uuid
  tags: QualityTag[]
  checked_at: IsoTimestamp
  checker_version: string
  /** `true` only when Phase 3's AiQualityChecker ran. Phase 2 always `false`. */
  ai_used: boolean
}

/** `POST /auth/login` body. */
export interface LoginResponse {
  csrf_token: string
}

/** `POST /inbox/{id}/approve` body. */
export interface ApproveResponse {
  status: 'confirmed'
  claim_id: Uuid
  event_seq: number
}

/** `POST /inbox/{id}/reject` body. */
export interface RejectResponse {
  status: 'rejected'
  event_seq: number
}

/** `POST /inbox/{id}/supersede` body. */
export interface SupersedeResponse {
  status: 'superseded'
  claim_id: Uuid
  event_seq: number
}

// ── Galaxy (E2.1/E2.2) ──────────────────────────────────────────────────────

/**
 * Level-of-detail selected by the Galaxy endpoint, mirrors Rust
 * `GraphLod` (snake_case serde rename). The endpoint picks one based on the
 * `zoom` parameter + node_count heuristics; the client must not assume a
 * particular value for a given zoom — render what's returned.
 */
export type GraphLod = 'community_supernodes' | 'visible_nodes' | 'ego_neighborhood'

/**
 * Edge kinds emitted by the Galaxy materializer, mirrors Rust `EdgeKind`
 * (`#[serde(rename_all = "snake_case")]`). `related` is the generic
 * undirected-edge case; `sources` / `supersedes` / `retracts` carry
 * provenance / supersession / retraction semantics for ego neighborhoods.
 */
export type EdgeKind = 'related' | 'sources' | 'supersedes' | 'retracts'

/**
 * Zoom preference — client-side knob that maps to `max_nodes` and influences
 * the server-chosen `GraphLod`. Mirrors Rust `ZoomLevel` (snake_case).
 */
export type ZoomLevel = 'far' | 'mid' | 'close'

/** Galaxy node — a claim (or community supernode at far zoom). */
export interface GalaxyNode {
  id: Uuid
  label: string
  kind: string
  domain: string
}

/** Galaxy edge between two node ids (referenced by `id`). */
export interface GalaxyEdge {
  source: Uuid
  target: Uuid
  kind: EdgeKind
}

/** `GET /galaxy` body — the bounded subgraph materializer result (E2.1). */
export interface GalaxyPayload {
  lod: GraphLod
  max_nodes: number
  node_count: number
  nodes: GalaxyNode[]
  edges: GalaxyEdge[]
}

// ── E3.2 trust + operations + destructive-action shapes ─────────────────────
//
// These mirror `src/trust.rs` (TrustFlag/RetrievalTrace/DestructiveWarning/
// JobSummary/BackupHealth/ClientActivity/EvalSummary) and `src/semantic.rs`
// (PurgePreview/PurgeReceipt) serde shapes EXACTLY — see tests/
// api_trust_ops_v1.rs for the wire-level fixtures.
//
// Field-name notes:
//   - `TrustFlag` is `#[serde(tag="kind", rename_all="snake_case")]` so each
//     variant carries `"kind": "contradiction"|"stale"|"orphan"`.
//   - `PurgePreview.targets` is `Vec<String>` (content-key strings, NOT
//     structured preview items). The E3.2 task brief described it as a list
//     of items but the shipped Rust serde shape is a bare string list.
//   - `POST /purge/preview` returns `{preview, warning}` where `warning` is a
//     full `DestructiveWarning` OBJECT (not a bare string) — see api.rs
//     `purge_preview` handler.
//   - `PurgeReceipt`'s `registry_epoch`/`new_backup_path`/`composite_checksum`
//     are `Option<>` server-side → nullable on the wire; only `completed`
//     receipts populate them.

/**
 * `GET /trust` flag row — the trust surface. Discriminated by `kind`.
 * Mirrors Rust `TrustFlag` (`#[serde(tag="kind", rename_all="snake_case")]`).
 */
export type TrustFlag =
  | { kind: 'contradiction'; claim_ids: string[] }
  | { kind: 'stale'; claim_id: string; days_since_modified: number }
  | { kind: 'orphan'; claim_id: string }

/** Retrieval-trace body returned optionally by `GET /trust?retrieval_query=`. */
export interface RetrievalTrace {
  included_claim_ids: string[]
  excluded_claim_ids: string[]
  reason: string
}

/** `GET /trust` body. */
export interface TrustResponse {
  contradictions: TrustFlag[]
  stale: TrustFlag[]
  retrieval_trace: RetrievalTrace | null
}

/** `GET /ops/clients` row. Bare JSON array on the wire. */
export interface ClientActivity {
  client_id: string
  label: string
  capabilities: string[]
  last_active_at: string
  mutation_count: number
}

/** `GET /ops/jobs` body. */
export interface JobSummary {
  active: number
  queued: number
  failed: number
}

/** `GET /ops/evals?domain=` body. */
export interface EvalSummary {
  case_count: number
  passed: number
  abstention_passed: boolean
  run_at: string
}

/** `GET /ops/backup-health` body. */
export interface BackupHealth {
  last_backup_at: string
  last_restore_drill_ok: boolean
}

/**
 * Staleness buckets from `GET /status` — counts of pages by last-modified age.
 * Mirrors Rust `StalenessBuckets` (src/ops/stats.rs).
 */
export interface StalenessBuckets {
  fresh: number
  stale_7d: number
  stale_30d: number
}

/**
 * Index health from `GET /status`. `built` is the RFC 3339 timestamp of the
 * last successful rebuild, or `null` when the index has never been built.
 */
export interface IndexHealth {
  stale: boolean
  built: string | null
}

/**
 * Louvain community-detection result. Opaque on the TS side beyond `count`
 * (the Rust struct has more fields but the Status page only uses count).
 * If a future page needs more, expand this interface then.
 */
export interface CommunityStats {
  count: number
}

/**
 * `GET /status` body. Mirrors Rust `WikiStats` (src/ops/stats.rs:35).
 * Optional graph fields are `null` when the wiki has too few connected nodes
 * to compute them (e.g. a fresh seed with one page).
 */
export interface WikiStats {
  wiki: string
  pages: number
  sections: number
  types: Record<string, number>
  status: Record<string, number>
  orphans: number
  avg_connections: number
  graph_density: number
  staleness: StalenessBuckets
  index: IndexHealth
  communities: CommunityStats | null
  diameter: number | null
  radius: number | null
  center: string[]
  structural_note: string | null
}

/**
 * `GET /activity` row. Mirrors Rust `ActivityEvent` (src/ops/activity.rs).
 * `ActivityKind` is `snake_case` serde-tagged on the Rust side; the three
 * variants below are exhaustive for phase 1 (git page events only).
 */
export type ActivityKind = 'page_created' | 'page_edited' | 'page_deleted'

export interface ActivityDetail {
  added: number
  removed: number
  subject: string
}

export interface ActivityEvent {
  kind: ActivityKind
  /** ISO-8601 timestamp (git's %aI author-date format). */
  timestamp: string
  /** Git author name. */
  actor: string
  /** Slug path (wiki-relative) of the page touched. */
  target: string
  detail: ActivityDetail
}

// ── Phase 4 — Config + index management shapes ──────────────────────────────
// These mirror Rust `ConfigView` (src/ops/config_view.rs), `IndexStatus`
// (src/index_manager.rs:47), and `UpdateReport` (src/index_manager.rs:39)
// serde shapes EXACTLY. Field-name note: the on-disk TOML key is `[provider]`
// but the Rust projection renames it to `extraction` on the wire so the Config
// page can show "Extraction" without confusion — the TS shape follows the
// wire (NOT the TOML key).

/** One wiki space row in `ConfigView.wiki_spaces`. */
export interface WikiSpaceView {
  name: string
  path: string
  /** Optional, skipped on the wire when `None`. */
  description?: string
  /** Optional git remote URL, skipped when `None`. */
  remote?: string
}

/** `ConfigView.server` — transport + access surface. No raw secret values. */
export interface ServerView {
  http_enabled: boolean
  http_port: number
  bind: string
  bind_all_interfaces: boolean
  acp_enabled: boolean
  /** Env var NAME (not the value) holding the dev bootstrap username.
   * Optional — skipped on the wire when not customized. */
  bootstrap_username_env?: string
  /** Env var NAME (not the value) holding the dev bootstrap password. */
  bootstrap_password_env?: string
}

/**
 * `ConfigView.extraction` — AI provider ("Extraction") section. `api_key_env`
 * is the env-var NAME (e.g. `OPENAI_API_KEY`), NOT the resolved secret value,
 * so the wire payload is safe to display verbatim.
 */
export interface ExtractionView {
  enabled: boolean
  base_url: string
  /** Env var NAME — safe, not the value. */
  api_key_env: string
  routine_model: string
  reasoning_model: string
}

/** `ConfigView.logging` — log format + rotation policy. */
export interface LoggingView {
  /** `"text"` or `"json"`. */
  format: string
  /** `"daily"` or `"never"`. */
  rotation: string
}

/** `ConfigView.index` — tokenizer + auto-rebuild policy. */
export interface IndexViewConfig {
  tokenizer: string
  auto_rebuild: boolean
}

/**
 * `GET /config` body. Mirrors Rust `ConfigView` (src/ops/config_view.rs).
 * Read-only projection of `GlobalConfig`; carries env-var NAMES (not values)
 * for every secret-bearing field, so the whole payload is display-safe.
 */
export interface ConfigView {
  wiki_spaces: WikiSpaceView[]
  server: ServerView
  extraction: ExtractionView
  logging: LoggingView
  index: IndexViewConfig
}

/**
 * `GET /index-status?wiki=` body. Mirrors Rust `IndexStatus`
 * (src/index_manager.rs:47). The three `last_*` fields are `Option<>` on the
 * wire — added 2026-07-25, so they may be absent on old `state.toml` files.
 */
export interface IndexStatus {
  wiki: string
  path: string
  /** ISO-8601 timestamp of the last successful build, or `null` if never built. */
  built: string | null
  pages: number
  sections: number
  stale: boolean
  openable: boolean
  queryable: boolean
  /** Optional — may be absent on old `state.toml` files. */
  last_pages_indexed?: number
  last_skipped?: number
  last_duration_ms?: number
}

/** `POST /index/update` body. Mirrors Rust `UpdateReport`. */
export interface UpdateReport {
  updated: number
  deleted: number
}

/**
 * One item a destructive action will affect. Mirrors Rust
 * `DestructivePreviewItem`. The destructive-warning handler returns an empty
 * `preview` array by default; the Console populates it client-side from the
 * context (entity ids, object ids) before showing the dialog.
 */
export interface DestructivePreviewItem {
  target_kind: string
  target_id: string
  effect: string
}

/**
 * `GET /destructive/warning?action=` body. The `message` is human-readable
 * warning text the UI MUST display before confirming (states "no undo" /
 * "cannot be recovered" for hard purge). All text is bound via Svelte text
 * binding — never `{@html}`.
 */
export interface DestructiveWarning {
  action: 'hard_purge' | 'entity_merge' | 'entity_split'
  irreversible: boolean
  requires_recent_reauth: boolean
  requires_two_step_nonce: boolean
  preview: DestructivePreviewItem[]
  message: string
}

/**
 * `POST /purge/preview` result. Mirrors Rust `PurgePreview`:
 *   - `preview_hash` + `nonce` MUST be echoed back to `/purge/execute`.
 *   - `expires_at` is RFC 3339; the nonce is single-use and time-bounded.
 *   - `targets` is the bare string list of object ids (NOT structured items).
 */
export interface PurgePreview {
  preview_hash: string
  nonce: string
  expires_at: string
  targets: string[]
}

/**
 * `POST /purge/execute` / `GET /purge/status` body. The `state` field is the
 * saga stage (`requested` / `registry_denied` / `key_revoked` / `live_deleted`
 * / `projections_cleaned` / `retention_pending` / `completed`). Only a
 * `completed` receipt populates `registry_epoch` / `new_backup_path` /
 * `composite_checksum`; the others are `null` while the saga is in flight.
 */
export interface PurgeReceipt {
  purge_id: string
  state: string
  registry_epoch: number | null
  new_backup_path: string | null
  composite_checksum: string | null
}

/**
 * `POST /purge/preview` body. NOTE: the `warning` is a full
 * `DestructiveWarning` object (not a bare string) — the server ships the
 * warning alongside the preview so the UI can render both atomically.
 */
export interface PurgePreviewResponse {
  preview: PurgePreview
  warning: DestructiveWarning
}

/** `POST /auth/reauth` body. */
export interface ReauthResponse {
  reauthenticated: boolean
  fresh_for_seconds: number
}

/** `POST /entity/merge` body. */
export interface EntityMergeResponse {
  status: 'merged'
  event_seq: number
}

/** `POST /entity/split` body. */
export interface EntitySplitResponse {
  status: 'split'
  event_seq: number
  moved_claim_count: number
  source_remaining_claim_count: number
}

/** `POST /claim/{id}/retract` body. */
export interface ClaimRetractResponse {
  status: 'retracted'
  event_seq: number
}

// ── parameter shapes ────────────────────────────────────────────────────────

export interface SearchParams {
  query: string
  domain?: string
  top_k?: number
}

export interface GetSubjectParams {
  subject: string
  domain?: string
}

export interface TimelineParams {
  domain: string
  subject: string
  predicate: string
}

// ── public API ──────────────────────────────────────────────────────────────

/**
 * `POST /auth/login` body `{username, password}` (Phase G, 2026-07-20) →
 * caches CSRF token, returns it. The backend still accepts the legacy
 * `{secret}` shape on this route, but the SPA always sends the new shape.
 */
export async function login(credentials: {
  username: string
  password: string
}): Promise<LoginResponse> {
  const result = await request<LoginResponse>({
    method: 'POST',
    path: '/auth/login',
    body: { username: credentials.username, password: credentials.password },
  })
  csrfToken = result.csrf_token
  return result
}

/** `POST /auth/logout` → clears CSRF cache. Always 204 on success. */
export async function logout(): Promise<void> {
  await request<null>({
    method: 'POST',
    path: '/auth/logout',
    csrf: true,
  })
  csrfToken = null
}

/** `GET /search?query=&domain=&top_k=`. */
export async function search(params: SearchParams): Promise<SearchResponse> {
  return request<SearchResponse>({
    method: 'GET',
    path: '/search',
    query: { query: params.query, domain: params.domain, top_k: params.top_k },
  })
}

/** `GET /get?subject=&domain=`. */
export async function getSubject(params: GetSubjectParams): Promise<GetSubjectResponse> {
  return request<GetSubjectResponse>({
    method: 'GET',
    path: '/get',
    query: { subject: params.subject, domain: params.domain },
  })
}

/** `GET /entity/timeline?domain=&subject=&predicate=` — bare JSON array. */
export async function timeline(params: TimelineParams): Promise<ClaimView[]> {
  return request<ClaimView[]>({
    method: 'GET',
    path: '/entity/timeline',
    query: { domain: params.domain, subject: params.subject, predicate: params.predicate },
  })
}

/** `GET /inbox` — array of pending proposals with Phase 1.6 conflicts. */
export async function inbox(): Promise<InboxProposal[]> {
  return request<InboxProposal[]>({
    method: 'GET',
    path: '/inbox',
  })
}

/** `GET /inbox/{proposal_id}/evidence` — bare JSON object. */
export async function evidence(proposalId: Uuid): Promise<EvidenceSummary> {
  return request<EvidenceSummary>({
    method: 'GET',
    path: `/inbox/${encodeURIComponent(proposalId)}/evidence`,
  })
}

/** `GET /inbox/{proposal_id}/ai-review` — deterministic quality tags. */
export async function aiReview(proposalId: Uuid): Promise<AiReviewResponse> {
  return request<AiReviewResponse>({
    method: 'GET',
    path: `/inbox/${encodeURIComponent(proposalId)}/ai-review`,
  })
}

/**
 * `GET /galaxy?domain=&zoom=far|mid|close&focus=<uuid>` — bounded subgraph
 * materializer (E2.1). Session required (401 without cookie); GET so no CSRF.
 * The endpoint selects the `lod` server-side; the client renders whatever it
 * returns. `focus` pins an ego-neighborhood around a specific entity.
 *
 * The optional `signal` is forwarded to `fetch` so callers that may unmount
 * mid-flight (e.g. `GalaxyGraph`'s `onDestroy` abort) can cancel the request.
 * On abort the underlying `fetch` rejects with an `AbortError` (an
 * `instanceof Error` whose `name === 'AbortError'`); callers should swallow
 * that case silently rather than surface it as a generic load failure.
 */
export async function galaxy(
  params: {
    domain?: string
    zoom?: ZoomLevel
    focus?: string
    signal?: AbortSignal
  },
): Promise<GalaxyPayload> {
  return request<GalaxyPayload>({
    method: 'GET',
    path: '/galaxy',
    query: { domain: params.domain, zoom: params.zoom, focus: params.focus },
    signal: params.signal,
  })
}

/** `POST /inbox/{proposal_id}/approve` (CSRF required). */
export async function approve(proposalId: Uuid): Promise<ApproveResponse> {
  return request<ApproveResponse>({
    method: 'POST',
    path: `/inbox/${encodeURIComponent(proposalId)}/approve`,
    csrf: true,
  })
}

/** `POST /inbox/{proposal_id}/reject` (CSRF required). */
export async function reject(proposalId: Uuid): Promise<RejectResponse> {
  return request<RejectResponse>({
    method: 'POST',
    path: `/inbox/${encodeURIComponent(proposalId)}/reject`,
    csrf: true,
  })
}

/** `POST /inbox/{proposal_id}/supersede` body `{superseded_claim_ids:[uuid]}`. */
export async function supersede(
  proposalId: Uuid,
  supersededClaimIds: Uuid[],
): Promise<SupersedeResponse> {
  return request<SupersedeResponse>({
    method: 'POST',
    path: `/inbox/${encodeURIComponent(proposalId)}/supersede`,
    body: { superseded_claim_ids: supersededClaimIds },
    csrf: true,
  })
}

// ── E3.2 trust + operations + destructive-action routes ─────────────────────
//
// All GETs are session-only (no CSRF). All POSTs are CSRF-required mutations.
// Source/target/object_ids/claim_operation_id are UUID strings on the wire.

/**
 * `GET /trust?staleness_threshold_days=&retrieval_query=`. The threshold
 * defaults to 90 server-side; `retrieval_query`, when non-empty, populates
 * `retrieval_trace`.
 */
export async function trust(params?: {
  staleness_threshold_days?: number
  retrieval_query?: string
}): Promise<TrustResponse> {
  return request<TrustResponse>({
    method: 'GET',
    path: '/trust',
    query: {
      staleness_threshold_days: params?.staleness_threshold_days,
      retrieval_query: params?.retrieval_query,
    },
  })
}

/** `GET /ops/clients` — bare JSON array of registered-client activity. */
export async function opsClients(): Promise<ClientActivity[]> {
  return request<ClientActivity[]>({ method: 'GET', path: '/ops/clients' })
}

/** `GET /ops/jobs` — async-job queue summary. */
export async function opsJobs(): Promise<JobSummary> {
  return request<JobSummary>({ method: 'GET', path: '/ops/jobs' })
}

/**
 * `GET /ops/evals?domain=X`. `domain` is required (server returns 400
 * `invalid_request` for empty/missing) — the eval summary is per-domain.
 */
export async function opsEvals(domain: string): Promise<EvalSummary> {
  return request<EvalSummary>({
    method: 'GET',
    path: '/ops/evals',
    query: { domain },
  })
}

/** `GET /ops/backup-health` — backup + restore-drill health. */
export async function opsBackupHealth(): Promise<BackupHealth> {
  return request<BackupHealth>({ method: 'GET', path: '/ops/backup-health' })
}

/** `GET /status` — wiki health snapshot for the Status page. */
export async function status(): Promise<WikiStats> {
  return request<WikiStats>({ method: 'GET', path: '/status' })
}

/**
 * `GET /activity?since=&limit=` — recent page changes feed. `since` accepts
 * `1d` / `7d` / `30d` (server defaults to `7d`); `limit` is clamped server-
 * side to 1..=200 (defaults to 50). Returns events newest-first.
 */
export async function activity(opts?: {
  since?: '1d' | '7d' | '30d'
  limit?: number
}): Promise<ActivityEvent[]> {
  const query: Record<string, string> = {}
  if (opts?.since) query.since = opts.since
  if (opts?.limit !== undefined) query.limit = String(opts.limit)
  return request<ActivityEvent[]>({ method: 'GET', path: '/activity', query })
}

// ── Phase 4 — Config + index management routes ──────────────────────────────
//
// `GET /config` + `GET /index-status` are session-only (no CSRF).
// `POST /index/update` + `POST /index/rebuild` are CSRF-gated mutations
// (cheap path vs. background job respectively).

/** `GET /config` — masked, read-only projection of `GlobalConfig`. */
export async function config(): Promise<ConfigView> {
  return request<ConfigView>({ method: 'GET', path: '/config' })
}

/**
 * `GET /index-status?wiki=name` — index health for a single wiki. `wiki`
 * omitted → the server's default/first wiki (mirrors the Rust handler, which
 * falls back to the configured default when the param is absent).
 */
export async function indexStatus(wiki?: string): Promise<IndexStatus> {
  return request<IndexStatus>({
    method: 'GET',
    path: '/index-status',
    query: wiki ? { wiki } : {},
  })
}

/**
 * `POST /index/update` body `{wiki}` — incremental update (cheap path,
 * re-indexes only changed pages). CSRF-gated. Returns the per-wiki update
 * tally (`updated` + `deleted` counts).
 */
export async function indexUpdate(wiki: string): Promise<UpdateReport> {
  return request<UpdateReport>({
    method: 'POST',
    path: '/index/update',
    body: { wiki },
    csrf: true,
  })
}

/**
 * `POST /index/rebuild` body `{wiki}` — full rebuild, scheduled as a
 * background job. Returns `{job_id}` immediately so the caller can poll
 * `/ops/jobs` (or just refresh `/index-status` after the job settles).
 * CSRF-gated.
 */
export async function indexRebuild(wiki: string): Promise<{ job_id: string }> {
  return request<{ job_id: string }>({
    method: 'POST',
    path: '/index/rebuild',
    body: { wiki },
    csrf: true,
  })
}

/**
 * `GET /purge/status?purge_id=<uuid>` — current PurgeReceipt for a saga.
 * Read-only (session only, no CSRF). Use this after `/purge/execute` returns
 * a `purge_id` to poll the saga state.
 */
export async function purgeStatus(purgeId: string): Promise<PurgeReceipt> {
  return request<PurgeReceipt>({
    method: 'GET',
    path: '/purge/status',
    query: { purge_id: purgeId },
  })
}

/**
 * `GET /destructive/warning?action=hard_purge|entity_merge|entity_split`.
 * Returns the warning the UI MUST display before the user confirms the
 * action (Task E3.2 DoD #2). The `preview` list is empty by default — the
 * caller populates it from the action context before showing the dialog.
 */
export async function destructiveWarning(
  action: 'hard_purge' | 'entity_merge' | 'entity_split',
): Promise<DestructiveWarning> {
  return request<DestructiveWarning>({
    method: 'GET',
    path: '/destructive/warning',
    query: { action },
  })
}

/**
 * `POST /entity/merge` body `{source, target, operation_id?}`. Every claim on
 * the source is rewritten onto the target. CSRF + session required.
 */
export async function entityMerge(
  source: string,
  target: string,
): Promise<EntityMergeResponse> {
  return request<EntityMergeResponse>({
    method: 'POST',
    path: '/entity/merge',
    body: { source, target },
    csrf: true,
  })
}

/**
 * `POST /entity/split` body `{source, assignments, operation_id?}`. Each
 * claim on the source whose predicate matches an assignment is rewritten onto
 * that assignment's target; predicates not listed stay on the source. CSRF +
 * session required.
 */
export async function entitySplit(
  source: string,
  assignments: { predicate: string; target_entity_id: string }[],
): Promise<EntitySplitResponse> {
  return request<EntitySplitResponse>({
    method: 'POST',
    path: '/entity/split',
    body: { source, assignments },
    csrf: true,
  })
}

/**
 * `POST /claim/{claim_operation_id}/retract`. `claim_operation_id` is the
 * proposer's confirm operation_id (NOT the raw claim UUID) — see api.rs
 * `entity_retract`. CSRF + session required. Retract is reversible via
 * supersede, so it does NOT route through the destructive-action dialog.
 */
export async function claimRetract(
  claimOperationId: string,
): Promise<ClaimRetractResponse> {
  return request<ClaimRetractResponse>({
    method: 'POST',
    path: `/claim/${encodeURIComponent(claimOperationId)}/retract`,
    csrf: true,
  })
}

/**
 * `POST /purge/preview` body `{object_ids}`. Phase 1 of the hard-purge flow.
 * Returns the preview (preview_hash + nonce + targets + expiry) AND the
 * `DestructiveWarning` the UI MUST display before the execute step. The
 * `preview_hash` + `nonce` must be echoed back to `purgeExecute`.
 */
export async function purgePreview(
  objectIds: string[],
): Promise<PurgePreviewResponse> {
  return request<PurgePreviewResponse>({
    method: 'POST',
    path: '/purge/preview',
    body: { object_ids: objectIds },
    csrf: true,
  })
}

/**
 * `POST /purge/execute` body `{preview_hash, nonce, operation_id?}`. Phase 2
 * of the hard-purge flow. The session MUST have a recent `/auth/reauth`
 * (else 403 `reauth_required`); the freshness gate lives in the API layer.
 * Returns the `PurgeReceipt` (saga state, eventually `completed`).
 */
export async function purgeExecute(
  previewHash: string,
  nonce: string,
): Promise<PurgeReceipt> {
  return request<PurgeReceipt>({
    method: 'POST',
    path: '/purge/execute',
    body: { preview_hash: previewHash, nonce },
    csrf: true,
  })
}

/**
 * `POST /auth/reauth` body `{username, password}` (Phase G, 2026-07-20).
 * Re-validates credentials against an EXISTING session and bumps its
 * freshness anchor so the next `/purge/execute` passes the recent-reauth
 * gate. Wrong credentials → 401.
 */
export async function reauth(credentials: {
  username: string
  password: string
}): Promise<ReauthResponse> {
  return request<ReauthResponse>({
    method: 'POST',
    path: '/auth/reauth',
    body: { username: credentials.username, password: credentials.password },
  })
}

/**
 * `GET /events` — opens the SSE stream. Returns the raw `EventSource` so
 * callers can attach their own listeners. Not deeply used yet (E2/E3 will
 * wire this into a live-update store).
 */
export function events(): EventSource {
  return new EventSource(`${API_BASE}/events`, { withCredentials: true })
}
