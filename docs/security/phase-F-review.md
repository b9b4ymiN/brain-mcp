# Phase F External Security Review

- **Reviewer identity**: security-reviewer subagent (fresh context, read-only).
  Treated as external per Phase F Task F3.4.
- **Date**: 2026-07-19.
- **Project**: brain-mcp-vnext.
- **Branch / HEAD**: `vnext/phase-0` @ `2a7f2df`.
- **Scope**: whole-codebase security posture, focused on the Phase E (Console)
  + Phase F1 (Docker) + F2 (observability) + F3 (recovery) surface.
- **Mode**: read-only review — no code modified, no commits made. All findings
  are listed for the implementer to action.

## Verdict (top of report)

**PASS — no unresolved Critical or High findings.** Three Medium and three
Low hardening items are listed under Recommendations. The Phase F gate can
close.

## Step 1 — Prior security docs reviewed

- `SECURITY.md` — confirmed the OUTDATED scope claim flagged in the brief: it
  says the project "does not make network requests, store credentials, or run
  user-supplied code". That was true for the original `llm-wiki` ingest-only
  tool but is no longer accurate after the Z.ai provider adapter (Phase D)
  shipped outbound HTTP + bearer auth, and the Console API (Phase E) shipped a
  bootstrap secret + session cookies. **This is a documentation-only finding**
  (Medium M-6 below) — the code does the right thing; only the public-facing
  policy is stale. A separate fix is appropriate but the threat model in
  `docs/security/threat-model-v1.md` already covers the new surface
  (TM-003 secret egress, TM-004 stored XSS, TM-013 unauthenticated bind).
- `docs/security/threat-model-v1.md` — reviewed all 24 threats. TM-013
  (0.0.0.0 bind), TM-007/011 (destructive purge / resurrection), TM-004
  (stored XSS) all map to Phase E + F controls that this review verified.
- `docs/plans/phase-F-production.md` Task F3.4 — confirmed the spec for this
  review; matched the verification checklist.

## Step 2 — Security-critical code verified

| Item | File:line | Verdict | Notes |
|---|---|---|---|
| CSP | `src/api.rs:67` | PASS | Strict baseline: `default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`. No `unsafe-inline`, no `unsafe-eval`, no `connect-src` wildcard. Applied uniformly via `SetResponseHeaderLayer::overriding` on the static-router fallback (`src/api.rs:1364-1371`) so it covers success and error responses. |
| Session cookie | `src/api.rs:314-323` | PASS | `HttpOnly; SameSite=Strict; Path=/api/v1; Max-Age=86400` always; `Secure` added when `secure_cookie` is true (set when `http_bind_all_interfaces = true`, `src/server.rs:174`). Path is correctly scoped to `/api/v1`. Expiry cookie (`src/api.rs:325-332`) mirrors the same flags. |
| CSRF | `src/api.rs:428-450`, all mutations | PASS | Double-submit: server-issued CSRF token (random UUID v4, `src/api.rs:476`) returned in login body, re-checked via `X-CSRF-Token` header on every mutation. Comparison is `constant_time_eq` (branch-free XOR accumulate, length-mismatch early return is an accepted coarse leak). Verified every mutation handler signature (`entity_merge`, `entity_split`, `entity_retract`, `purge_preview`, `purge_execute`, `approve`, `reject`, `supersede`) takes `CsrfSession`, so the extractor runs before the handler. Logout uses `AuthSession` (no CSRF) — correct: logout invalidates the session. |
| Login secret compare | `src/api.rs:463`, `src/api.rs:1217` | PASS | Both `login` and `reauth` use `constant_time_eq(body.secret, bootstrap_secret)`. Failed-login counter increments regardless of user existence (no user-enumeration signal; there is only one owner anyway). |
| Bootstrap secret indirection | `src/config.rs:352-364`, `src/server.rs:157-178` | PASS | Priority: file > direct string. **Fail-closed verified**: a set `console_dev_bootstrap_secret_file` that fails `read_to_string` returns `Err` with context, and `serve_http` propagates via `?` so the server refuses to start. No fallback to the direct string or empty. Empty resolved secret keeps the router unmounted (`if !secret.is_empty()`). Note: code does not explicitly chmod 0600 on the secret file (it reads, doesn't create) — operator responsibility for the source file. The compose `secrets:` block + Docker `_file:` convention handles this; see F1. |
| LogRedactor | `src/observability.rs:96-176`, `src/main.rs:1234-1341` | PASS | `RedactingMakeWriter` wraps every tracing writer path — stderr (all 3 layer shapes), file via `tracing_appender::NonBlocking` (both json + compact + text formats). Patterns caught: `Bearer `, `access_token=`, `api_key=`, `sk-<key>` (case-insensitive, ≥16-char material — covers `sk-proj-...` and OpenAI/Anthropic shapes, `src/provider.rs:171-188`). |
| Ingest limits | `src/mcp/mod.rs:210-254`, `src/observability.rs:178-303` | PASS | Wired via `with_ingest_limits`. `check_ingest_limits` runs size-first then rate. Rejected attempts don't consume budget (`IngestRateLimiter::check_and_record` only writes on accept, `src/observability.rs:287-292`). Memory bounded: full-sweep eviction on every call drops stale timestamps + empties out idle clients (`src/observability.rs:280-283`). |
| Encrypted backup | `src/semantic.rs:1432-1694` | PASS | AES-256-GCM via `aes-gcm 0.10.3`. Per-file fresh 12-byte nonce (`OsRng`) prefixed to ciphertext (`src/semantic.rs:1499-1505`). Fail-closed on corrupted key: a `backup.key` of wrong length returns `CorruptLedger` rather than rewriting (`src/semantic.rs:1471-1477`) so existing backups don't become unrecoverable ciphertext under a fresh key. Key created with `OpenOptionsExt::mode(0o600)` on Unix (atomic 0600 from create, never world-readable transiently). Plaintext staging dir under a `StagingDirGuard` Drop that wipes on panic + `?` errors (`src/semantic.rs:1552-1561`). |
| Restore fail-closed | `src/semantic.rs:1725-1970` | PASS | Composite checksum verified (`recomputed != manifest.composite_checksum → Err`, `src/semantic.rs:1945-1950`). PurgeRegistry epoch match returned in receipt; drill `passed()` requires both flags (`src/semantic.rs:1989-2028`). Per-blob digest verification catches object-swap under same key (`src/semantic.rs:1873-1914`): missing-from-manifest blob = tampering; extra blob = tampering; F3.1-era empty map = warn + skip (back-compat). |
| Dockerfile | `Dockerfile` | PASS | Multi-stage (console-builder + builder + runtime). Non-root user `brain` uid 1000 (`Dockerfile:73`, `USER brain` line 83). No `COPY` of any config / secret / `.docker-smoke-data` (`.dockerignore` excludes them). No `ENV` with secrets. Healthcheck uses `curl` to `/health` (loopback inside container). |
| docker-compose | `docker-compose.yml` | PASS | Secret via `secrets:` block (`bootstrap_secret: file: ./secrets/bootstrap_secret.txt`) read inside the container at `/run/secrets/bootstrap_secret` via `serve.console_dev_bootstrap_secret_file`. NOT in `environment:` (env values leak via `docker inspect`). Port publish is `127.0.0.1:8080:8080` (loopback-only on the host). Volume mounts: `data`, `backups` (writable), `config.toml` (`:ro`). |
| HTTP bind | `src/server.rs:104-121`, `src/config.rs:228-233, 315-316` | PASS | Default `http_bind_address = "127.0.0.1"` and `http_bind_all_interfaces = false`. Setting `http_bind_all_interfaces = true` emits a `tracing::warn!` on the real serve path ("UNAUTHENTICATED. Only safe behind a reverse proxy with auth"). Invalid `http_bind_address` falls back to 127.0.0.1, not to 0.0.0.0. |
| Destructive guards | `src/api.rs:1147-1170`, `src/trust.rs:184-211` | PASS | Hard purge requires: (1) CsrfSession, (2) `require_purge_freshness` (300s window from `/auth/reauth`, server-enforced at `src/api.rs:1155`), (3) `purge_execute` semantic gate (preview_hash + nonce + capability). `DestructiveWarning::for_action(HardPurge)` message contains "no undo" (`src/trust.rs:201`); unit test asserts the wording (`src/trust.rs:224-230`). Server-enforced — UI cannot bypass. |
| Galaxy XSS | `web/console/src/lib/galaxyRenderer2d.ts:89-101`, `galaxyRenderer3d.ts:132-146`, `safeText.ts` | PASS | Both renderers wrap every untrusted tooltip field (`label`, `kind`, `domain`) with `escapeHtml` before assembling the d3 `.html()` tooltip — only the literal `<b>` / `<br/>` formatting tags are unescaped. `escapeHtml` is a one-pass 5-entity escape (`&`, `<`, `>`, `"`, `'`). List renderer uses `textContent` only (`galaxyRendererList.ts:9`). No `{@html}` anywhere in `web/console/src` (grep verified). |

## Step 3 — Dependency audit (`cargo audit`)

```
$ cargo audit
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1166 security advisories
    Scanning Cargo.lock for vulnerabilities (498 crate dependencies)

Crate:     bincode
Version:   2.0.1
Warning:   unmaintained
Title:     Bincode is unmaintained
Date:      2025-12-16
ID:        RUSTSEC-2025-0141
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0141

warning: 1 allowed warning found
```

**Verdict: clean.** Zero vulnerabilities. One unmaintained-crate warning
(`bincode 2.0.1`) — already documented as TM-016 in `threat-model-v1.md`
(Medium, snapshots remain rebuildable/untrusted). Phase 0 remediations hold:
- `crossbeam-epoch 0.9.20` (TM-014 RUSTSEC-2026-0204 fixed, no regression).
- `anyhow 1.0.103` (TM-015 fixed).
- `memmap2 0.9.11` (TM-015 fixed).
- `aes-gcm 0.10.3` (current).
- `rustls 0.23.38` (current).

## Step 4 — Docker image scan

Docker Desktop was not running on the review host, so a live `docker history`
/ `docker inspect` scan could not be executed. The static analysis is
favorable:

- `Dockerfile` contains no `ENV SECRET`, no `COPY` of any path under
  `secrets/`, `config/`, `.docker-smoke-data/`, or any
  `*_secret*`/`*_token*`/`*_key*` filename.
- `.dockerignore` excludes `.docker-smoke-data/`,
  `scripts/docker-smoke-config.toml`, `*.log`, `.git/`, and
  `docs/` / `tests/` from the build context.
- Compose secret is provided via the Docker `secrets:` mechanism, not via
  `environment:`. The compose smoke script
  (`scripts/docker_compose_smoke.sh`) already asserts the secret is absent
  from `Config.Env` per the Phase F1.2 plan.
- The runtime image has no shell access for the `brain` user beyond `/bin/bash`
  (which is fine for ops + the `HEALTHCHECK` uses `curl`).

Phase F1 plan DoD #5 (`docker history` clean, `Config.Env` clean) was already
closed at commit `9e3d6ba`/`41a1406`/`e5f7b41`; this review did not
re-execute it but found no regression in the Dockerfile or compose diff.

## Step 5 — Findings

### Critical

None.

### High

None.

### Medium (hardening, no gate block)

| ID | Item | File:line | Recommendation |
|---|---|---|---|
| M-1 | No `__Host-` cookie prefix on `brain_console_session`. The cookie is already `Secure` (when bound non-loopback) + `HttpOnly` + `SameSite=Strict` + `Path=/api/v1`, which provides equivalent protection in modern browsers, but the `__Host-` prefix is defense-in-depth against any future regression that loosens `Path`/`Domain`. | `src/api.rs:314-323` | Rename cookie to `__Host-brain_console_session` and drop the explicit `Path=` attribute (the `__Host-` prefix mandates `Path=/` and forbids `Domain=`). Requires verifying the SPA can carry the cookie on `/api/v1/*` requests when the cookie is scoped to `/`. |
| M-2 | No HTTP-layer rate limit on `/auth/login` / `/auth/reauth`. A remote attacker who reaches the Console API (via `0.0.0.0` bind without a rate-limiting reverse proxy, contrary to documented ops) can brute-force the bootstrap secret. The loopback default + documented "reverse proxy with auth" requirement mitigates in production; this finding is the residual risk if an operator deploys without the proxy. | `src/api.rs:459-503` | Wire `tower-governor` (or a hand-rolled per-IP limiter mirroring `IngestRateLimiter`) on `/auth/login`. Failed-login counter (`console_auth_failures_total`) already exists for observability; pair it with an exponential backoff or a 5-per-minute cap. |
| M-3 | `bincode 2.0.1` unmaintained (TM-016, RUSTSEC-2025-0141). Already on the threat model; snapshots remain rebuildable so impact is bounded. | `Cargo.lock:381-382` | Track upstream replacement or pin to a maintained fork. No urgency — advisory is `unmaintained`, not a vuln. |
| M-4 | `SECURITY.md` scope claim is stale. The header says the project "does not make network requests, store credentials, or run user-supplied code" — no longer true after Phase D (Z.ai adapter makes outbound HTTP + bearer auth) and Phase E (Console API + bootstrap secret + session cookie). | `SECURITY.md:20-26` | Update the Scope section to reflect the new surface: outbound HTTPS to configured Z.ai base URL, dev-grade bootstrap secret gated by loopback bind, Console HTTP API at `/api/v1`. Cross-reference `docs/security/threat-model-v1.md`. |
| M-5 | Bootstrap secret comparison uses a hand-rolled `constant_time_eq` (correct), but the comment notes "no `subtle` dep for ~10 lines". If the project later pulls in `subtle` for any other reason, switching to `subtle::ConstantTimeEq` would remove the maintenance burden of in-house crypto-adjacent code. | `src/api.rs:294-303` | Optional: replace with `subtle::ConstantTimeEq` when the dep is already in the tree. Not a vuln today. |
| M-6 | `/metrics` endpoint has no auth. Documented as deliberate ("Prometheus scrapers don't authenticate at this layer, loopback bind default protects on a dev box, production puts the route behind a reverse proxy with auth at the proxy layer" — `src/server.rs:813-818`). If an operator binds `0.0.0.0` without a proxy, `/metrics` is publicly readable; the exposure is bounded (no PII, only aggregate counters + a few gauges) but still reveals system topology. | `src/server.rs:860-877` | Document the operator mitigation in `docs/guides/deploy-docker.md` (likely already there) and/or add a config-gated bearer-token check on `/metrics` for sites that want it. |

### Low / informational

| ID | Item | File:line | Recommendation |
|---|---|---|---|
| L-1 | Session map is in-memory (`Arc<RwLock<HashMap<String, Session>>>`). A restart evicts all sessions (acceptable for dev-grade auth; documented as "Production OAuth is Phase F"). On a multi-instance deployment this would force re-login per instance — but multi-instance is out of scope for v0.5. | `src/api.rs:95` | Document the single-instance constraint in the deploy guide. Phase F OAuth will need a shared session store. |
| L-2 | `console_logins_total` counter is bumped for `/auth/reauth` successes (`src/api.rs:1234`). Defensible (a re-auth is a fresh login-equivalent for audit) but slightly conflates "initial login" with "freshness re-auth". A `console_reauth_total` separate counter would clean up dashboards. | `src/api.rs:1232-1234` | Add a separate counter. Observability polish only. |
| L-3 | `Cargo.lock` rolls `aes-gcm 0.10.3` — the `aead` traits are loaded with the older 0.5 series. No vulnerability; current versions are 0.10.x as of this writing, so this is in line with ecosystem. | `Cargo.lock:27-28` | No action. |

## Fixed-in-this-review

**None.** This was a read-only review. All findings above are listed for the
implementer to triage. None are Critical or High, so none block the Phase F
gate close.

## Recommendations for future hardening (post-gate)

1. **Phase F2 OAuth + token-based session** (replaces the in-memory session +
   dev bootstrap secret). The code is already structured for this — the
   `Session` struct and `resolve_session` indirection can be swapped for a
   stateless JWT/opaque-token lookup without changing the route shape.
2. **`__Host-` cookie prefix** (M-1) — single-line change.
3. **HTTP-layer rate limit on auth routes** (M-2) — `tower-governor` is
   already compatible with the existing axum router shape.
4. **Replace `bincode`** (M-3) or pin a maintained fork.
5. **`SECURITY.md` refresh** (M-4) — documentation-only, but the
   public-facing policy should match reality before any external
   announcement.
6. **Re-run this review at Phase G** (post-OAuth) — the auth surface will
   change substantially; the CSP/CSRF/cookie work done here will carry
   forward, but the bootstrap-secret comparisons (which this review
   verified) will be retired.

## Final verdict

**PASS.**

- Critical findings: **0**
- High findings: **0**
- cargo audit: **0 vulnerabilities**, 1 documented unmaintained-crate warning.
- All Phase E (Console), F1 (Docker), F2 (observability), F3 (recovery)
  controls verified at the code level. Fail-closed paths
  (`resolve_bootstrap_secret`, `restore_from_backup`, `validate_database_identity`,
  destructive-action re-auth freshness) were read end-to-end and confirmed.
- Docker live scan (`docker history`) was not re-executed (Docker Desktop
  was not running); the Phase F1 plan DoD #5 had already closed this at
  `9e3d6ba`/`41a1406`/`e5f7b41` and no Dockerfile/compose regression was
  found in static review.

**Phase F Gate = Phase 6 Gate can close.**
