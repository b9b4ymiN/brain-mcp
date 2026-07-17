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
    })
  } catch (cause) {
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

/** `GET /inbox/{id}/evidence` body. Bare JSON object. */
export interface EvidenceSummary {
  provenance_kind: string
  excerpt: string | null
  source_id: Uuid | null
  quote_hash: string | null
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

/** `POST /auth/login` body `{secret}` → caches CSRF token, returns it. */
export async function login(secret: string): Promise<LoginResponse> {
  const result = await request<LoginResponse>({
    method: 'POST',
    path: '/auth/login',
    body: { secret },
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

/** `GET /inbox` — bare JSON array of pending proposals. */
export async function inbox(): Promise<ProposalSummary[]> {
  return request<ProposalSummary[]>({
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

/**
 * `GET /galaxy?domain=&zoom=far|mid|close&focus=<uuid>` — bounded subgraph
 * materializer (E2.1). Session required (401 without cookie); GET so no CSRF.
 * The endpoint selects the `lod` server-side; the client renders whatever it
 * returns. `focus` pins an ego-neighborhood around a specific entity.
 */
export async function galaxy(params: {
  domain?: string
  zoom?: ZoomLevel
  focus?: string
}): Promise<GalaxyPayload> {
  return request<GalaxyPayload>({
    method: 'GET',
    path: '/galaxy',
    query: { domain: params.domain, zoom: params.zoom, focus: params.focus },
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

/**
 * `GET /events` — opens the SSE stream. Returns the raw `EventSource` so
 * callers can attach their own listeners. Not deeply used yet (E2/E3 will
 * wire this into a live-update store).
 */
export function events(): EventSource {
  return new EventSource(`${API_BASE}/events`, { withCredentials: true })
}
