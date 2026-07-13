# ADR-0001: Semantic Authority, Identity, Time, Privacy, Purge, and Authorization

- Status: **Accepted for Phase 0**
- Date: 2026-07-13
- Decision owners: repository owner and Brain Application Core
- Scope: contracts only; no runtime or data migration is authorized by this ADR
- Baseline: `brain-mcp` v0.4.15 at `c49e7b30705f0055402dea2b2ec3b1471c1e29b3`

## Context

The upstream system has a useful Markdown/Git wiki, MCP transports, Tantivy search, Petgraph projections, and a large test suite. Its current write path writes Markdown directly, commits Git separately, and refreshes derived indexes independently. The HTTP server can bind to all interfaces and has no application authorization layer. Those properties are acceptable as a read/write wiki baseline but cannot be treated as the authority for bitemporal claims, concurrent semantic writes, or irreversible privacy purge.

This ADR freezes the boundary that Task 0.3 must spike. It does not authorize production exposure, user-data migration, a live identity provider, or a new write path.

## Decision 1: authority and no dual truth

**DECISION: Event Ledger is the sole semantic transition authority.**

**No dual source of truth** means each layer has one explicit kind of authority:

| Layer | Authority | Mutable? | Recovery rule |
|---|---|---:|---|
| Raw object store | Canonical bytes/evidence addressed by SHA-256 | No, except irreversible purge | Verify hash; never reconstruct bytes from a claim |
| Human-authored Markdown | Canonical authored document content with Git history | New revisions only | Git restores authored text, not claim state |
| Event ledger | Sole order and authority for semantic transitions | Append-only | Replay through a selected ledger head |
| Purge registry | Sole deny/decryption authority for purged IDs and key epochs | Monotonic append-only | Sync before any read/decrypt; stale/unavailable means fail closed |
| Claim snapshot | Deterministic semantic read model | Projector-only replacement | Rebuild from ledger head minus denied IDs at purge epoch |
| Generated wiki | Readable materialized view | Replaceable | Regenerate from claim snapshot and citations |
| Tantivy/Petgraph/vector | Search/graph projections | Replaceable | Delete and rebuild from canonical layers |
| AI output | Proposal material only | Ephemeral or proposal event | Never becomes confirmed memory without policy event |

The purge registry does not create or supersede a semantic claim. The event ledger does not override a deny or make destroyed content decryptable. A readable semantic state is:

```text
snapshot = project(replay(events[1..ledger_head]), schema_version)
readable = snapshot - purge_registry.denied_ids(purge_epoch)
checksum = sha256(canonical(snapshot) || ledger_head || purge_epoch || schema_version)
```

Only the Brain Application Service may append a semantic event. MCP, HTTP, Console, workers, importers, Git hooks, and projectors call that service; none writes a claim snapshot, ledger row, or purge state directly.

## Decision 2: atomic write topology

There is one serialized semantic writer process per owner ledger. Multiple clients remain supported because commands enter the same service and SQLite `BEGIN IMMEDIATE` transaction.

Normal write flow:

1. Validate authenticated authority, capability, schema, valid time, provenance, and policy.
2. Normalize request and calculate a canonical request SHA-256.
3. Resolve `(owner_id, client_id, operation_id)` inside the transaction.
4. Stage object bytes outside the canonical path; calculate SHA-256, `fsync`, then atomic rename.
5. Append exactly one ordered semantic event and transactional-outbox work in SQLite WAL.
6. Commit, then let idempotent consumers update Git/Markdown, Tantivy, Petgraph, and optional projections.
7. Return the stored outcome. A replayed identical operation returns the same outcome; a changed tool or payload returns `IDEMPOTENCY_CONFLICT` without mutation.

SQLite, filesystem objects, Git, and indexes are not described as one transaction. Staging cleanup handles objects that lose the transaction race; outbox replay handles committed events whose projections lag or crash.

## Decision 3: stable IDs and ordering

**DECISION: UUIDv7** in lowercase canonical RFC 9562 text form is the server-generated stable identifier for owners, actors, clients registered by the server, events, sources, renditions, evidence spans, entities, claims, proposals, jobs, and purge requests.

Exceptions:

- Raw object identity is `sha256:<64 lowercase hex>` because byte identity is content-derived.
- `operation_id` is a client-supplied opaque 1–128 byte UTF-8 identifier. Clients should use UUIDv7 or UUIDv4, but the server never infers time or identity from it.
- `event_seq` is a positive, server-assigned 64-bit integer monotonically increasing within `owner_id`; it is the only semantic order tie-breaker.
- External/provider IDs are aliases, never primary keys.

UUID embedded time improves locality but is not trusted as `recorded_at`, valid time, authorization proof, or latest-wins order. IDs never contain slug, entity name, provider, filesystem path, email, or secret.

## Decision 4: event envelope and hash chain

`evals/v1/contracts/event-schema-v1.json` is the frozen event envelope. Each append records:

- schema and event identity;
- `owner_id`, authoritative `event_seq`, event type, and server `recorded_at`;
- authenticated `actor_id`, technical `client_id`, optional session/channel inside the encrypted payload, and client `operation_id`;
- canonical request hash and stored outcome through the idempotency record;
- encrypted envelope, opaque object reference, or explicitly non-content payload;
- preceding event hash, event hash, and purge epoch observed at append.

Canonical event hashing uses RFC 8785 JSON Canonicalization Scheme over the schema-valid event with `event_hash` omitted, UTF-8 without BOM, SHA-256, and lowercase hex. `prior_event_hash` is null only for the first owner event. Hash chaining detects alteration but is not a substitute for authenticated storage, backups, or authorization.

The ledger must not contain source text, claim values, quotes, credentials, or other purge-target payload as plaintext. Content is an opaque object reference or an AEAD envelope whose DEK can be destroyed.

## Decision 5: bitemporal semantics

**DECISION: half-open valid-time intervals** `[valid_from, valid_to)` represent when a claim is true in the modeled world.

- Both bounds are optional; null means unbounded in that direction.
- Empty/reversed intervals are invalid.
- External timestamps preserve their source precision and timezone evidence; normalized comparison uses UTC.
- `recorded_at` is a server-generated RFC 3339 UTC timestamp. It is audit/display data; `event_seq` is authoritative when clock values collide or move backward.
- Transaction-time/as-of state is selected by ledger head/event sequence. A timestamp as-of query first selects the maximum committed sequence with `recorded_at <= requested_time`, while exposing clock ambiguity if the server clock was unhealthy.
- Git commit time, file mtime, UUIDv7 embedded time, client time, ingest time, publication time, and market effective time are never substituted for one another.
- A future-valid confirmed claim is historical/known but not current until `valid_from`.
- Correcting a valid-time boundary appends a correction/supersede event; it never edits the old event.

Current at world time `t` means `valid_from <= t` when bounded and `t < valid_to` when bounded, plus confirmed/not denied/not superseded at the selected ledger head.

## Decision 6: claims, provenance, and conflicts

A claim contains stable claim/subject identity, predicate, typed value, kind, status, valid interval, recorded event, non-empty provenance union, confidence, privacy/domain labels, and explicit supersede/retract links.

Valid provenance variants:

- `evidence`: immutable rendition plus half-open UTF-8 byte span and quote hash;
- `user_assertion`: authenticated utterance object/span and event;
- `mechanical`: input hashes, deterministic method/version, and output hash;
- `inference`: method/model/prompt and evidence when available.

An inference with no evidence is `unsupported=true`, may only be proposed/rejected, and cannot be confirmed. User acceptance creates a new user assertion or decision event rather than laundering an unsupported external fact.

### latest-user-wins

`latest-user-wins` applies only to `preference`, `profile`, and `project decision` claims with the same explicit scope key:

```text
(owner_id, domain, subject_id, predicate, normalized_context)
```

The winning claim is the confirmed user-authored candidate with greatest `event_seq`. It does not cross subject, predicate, domain, portfolio/account, or context. The owner and the owner's partner intentionally share one `actor_id=owner`; only client/session/channel can be audited, so the system must not claim which person spoke.

Example:

```json
{"scope":"projects:brain:deployment:production","events":[{"seq":41,"value":"Kubernetes"},{"seq":49,"value":"Docker Compose"}],"expected_active_seq":49,"history":[41,49]}
```

External facts and stock observations never use latest-user-wins. Conflicting credible claims coexist as `disputed`, retain source/valid time, and cause an evidence-rich answer, clarification, or abstention.

```json
{"subject":"stock:GULF","predicate":"target_price","claims":[{"value":48.0,"source":"broker-a"},{"value":55.0,"source":"broker-b"}],"expected":"disputed","silent_overwrite":false}
```

Executable variations live in the stocks, projects, and knowledge corpora locked by `evals/v1/manifest.json`.

## Decision 7: retract versus irreversible purge

Retract/archive is the default deletion behavior. It appends a semantic event, removes the claim from current answers, preserves history/evidence subject to normal retention, and can be reversed with a later event.

Hard purge is an irreversible security operation, not a semantic undo. It requires `brain.purge`, recent owner re-authentication no older than five minutes, and a single-use confirmation nonce expiring after 60 seconds. The preview names affected object/claim/source IDs, projections, Git revisions, key epochs, and backup sets; the confirmation must bind to that preview hash.

The hard-purge saga is **deny-first** and idempotent:

1. `requested`: authorize and persist target/composite checksum without sensitive plaintext.
2. `registry_denied`: append denied IDs and next monotonic purge epoch; replicate to at least two independent targets and require quorum acknowledgement.
3. `key_revoked`: destroy target DEKs/wrapped keys; rotate the owner encryption epoch and rewrap surviving keys.
4. `live_deleted`: remove live object/envelope/rendition material and redact content-bearing logs/queues.
5. `git_rewritten/index_rebuilt`: rewrite affected private Git history, rebuild projections, and prove target absence.
6. `retention_pending`: invalidate backups capable of decryption, overriding daily-30/monthly-12 retention, then create and verify a new full backup.
7. `completed`: store only a non-sensitive receipt and post-purge composite checksum.

After `registry_denied`, every read/decrypt/export/restore fails closed for the denied IDs even if later cleanup crashes. Restore first obtains and verifies the newest purge registry from independent targets, applies it, checks epoch monotonicity, and only then exposes/decrypts application data. A stale, unavailable, rolled-back, or checksum-invalid registry keeps the service sealed.

Cryptographic erasure is only claimed when key destruction makes recovery infeasible and all alternate key copies/backups are addressed. This follows the media-sanitization objective in NIST SP 800-88 Rev. 2; it is not a claim that deleting a database row alone sanitizes storage.

## Decision 8: authorization and identity matrix

**DECISION: Keycloak** 26.7.0 is the selected self-hosted Authorization Server candidate for Console and remote MCP because it provides OIDC discovery, authorization code, PKCE S256 enforcement, short-lived tokens, refresh-token/session revocation, client policies, and portable container deployment. Deployment must pin the tested image by immutable digest; `26.7.0` is the decision baseline, not permission to use a floating tag.

There is a known standards gap: Keycloak's current official MCP guide marks MCP 2025-11-25 only partially supported because RFC 8707 Resource Indicators are not yet supported in the documented stable conformance path. A listed `resource-indicators:v1` feature is not accepted as proof by name alone.

**HARD GATE: remote MCP remains disabled** and bound to loopback/private test networks until an exact Keycloak image digest passes all of these tests:

1. MCP client sends the canonical `resource` in authorization and token requests.
2. Authorization Server binds that resource to the resulting token audience.
3. Brain rejects missing/wrong issuer, signature, expiry, not-before, audience, authorized party/client, owner, and capability.
4. Protected Resource Metadata (RFC 9728), Authorization Server/OIDC discovery, PKCE S256, exact redirect URI, refresh rotation/revocation, and incremental scopes interoperate with target clients.
5. No token passthrough occurs.

If Keycloak fails, Task 3.3 must select a compliant replaceable Authorization Server before remote exposure. A scope-to-audience mapper may be tested as defense in depth but does not prove the AS processed RFC 8707 `resource`, and cannot waive the gate.

OAuth 2.1 remains an IETF draft (revision 15 as checked 2026-07-13); implementation follows the exact MCP 2025-11-25 profile and its pinned normative RFCs, not an assumed final RFC.

| Channel/principal | Authentication | Authority/capabilities | Lifecycle and controls |
|---|---|---|---|
| Console owner | OIDC authorization code + PKCE S256 | Per-route `brain.*` allow-list | Backend-for-frontend secure HttpOnly/Secure/SameSite=Lax session; CSRF token + Origin check; idle/absolute expiry; logout and AS revocation |
| Remote interactive MCP | OAuth profile from MCP 2025-11-25 | Token audience plus per-tool capability | Short access token (target 5 min), rotating/revocable refresh token, exact redirects, no wildcard; pre-registered clients first |
| Local stdio | OS user plus local capability credential retrieved from OS secure store | Explicit local allow-list; annotations are not auth | No network listener; logs only stderr; credential handle never printed or stored in wiki/config |
| AI worker | Internal workload identity | `brain.propose` only | No `confirm`, `purge`, admin, Console session, or arbitrary outbound side-effect credential |
| Projection worker | Internal workload identity | Read ledger/outbox and write named projection only | Cannot append semantic events or decrypt unrelated content |
| Backup/restore operator | Owner admin + recent re-auth + two-step operation | Narrow backup/restore capability | Restore stays sealed until purge registry sync/verification succeeds |
| Purge operator | Owner + `brain.purge` + recent re-auth + bound single-use nonce | Exact preview hash only | Nonce 60 seconds; re-auth 5 minutes; immutable receipt; no undo |

Minimum capabilities are `brain.read`, `brain.capture`, `brain.propose`, `brain.confirm`, `brain.purge`, and `brain.admin`. Each tool/route has an allow-list. There is no broad `brain.write` capability. Dynamic Client Registration is disabled by default. Experimental Client ID Metadata Document fetching is disabled until SSRF and trust-policy tests pass.

The Application Core derives `owner_id`, `actor_id`, and `client_id` from validated security context or registered local identity. Request fields cannot override them. Tool annotations remain UX hints only.

## Decision 9: privacy and outbound policy

Objects and claims carry privacy labels: `local_only`, `private_external_allowed`, or `publishable`. The default is `local_only` until an authenticated policy event authorizes a destination and data class.

Secrets, credentials, detected tokens, and unapproved destinations are denied before serialization into provider requests, retry/dead-letter payloads, telemetry, or logs. AI providers receive redacted bounded renditions, never content keys or authority tokens. Provider/model/base URL are configuration and audit metadata, not durable claim fields.

## Threat and eval contract

`docs/security/threat-model-v1.md` owns threats, severity, controls, and test IDs. `evals/v1/manifest.json` hash-locks self-contained corpora and metric definitions. Hard invariants—temporal correctness, provenance, idempotency, auth, purge denial, and no-egress—require 100% pass and cannot be averaged away by retrieval quality.

## Consequences

Positive:

- Semantic order is deterministic and independent of clocks, filenames, Git commits, or AI providers.
- Raw evidence remains portable while projections can be replaced.
- Retry/concurrency and post-restore purge behavior have explicit authorities.
- Auth limitations block exposure instead of becoming undocumented exceptions.

Costs:

- The application needs an event store, outbox, encrypted object envelope, purge registry, and projector interfaces.
- Git/Markdown becomes asynchronously consistent with semantic state.
- Hard purge is operationally expensive and may truncate backup retention.
- Keycloak remains conditional until exact MCP conformance is proven.

## Rejected alternatives

- Markdown/Git as semantic truth: cannot provide transactional ordering, bitemporal conflict state, or deny-first purge.
- Claim snapshot as writable truth: creates dual writers and irreproducible history.
- Client timestamps/UUID sort for latest-wins: spoofable and clock-dependent.
- Newest external claim wins: unsafe for investment facts and destroys disagreement.
- Soft delete for privacy purge: permits restore/decryption resurrection.
- Embedded custom OAuth server: expands high-risk security scope unnecessarily.
- Keycloak scope mapper as full RFC 8707 conformance: official documentation says the gap remains.

## Sources checked 2026-07-13

- MCP 2025-11-25 Authorization: https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization
- RFC 9562 UUIDs: https://www.rfc-editor.org/rfc/rfc9562.html
- RFC 8707 Resource Indicators: https://www.rfc-editor.org/rfc/rfc8707.html
- Keycloak MCP integration/conformance: https://www.keycloak.org/securing-apps/mcp-authz-server
- Keycloak administration guide: https://www.keycloak.org/docs/latest/server_admin/index.html
- OAuth 2.1 draft history: https://datatracker.ietf.org/doc/draft-ietf-oauth-v2-1/history/
- NIST SP 800-88 Rev. 2: https://csrc.nist.gov/pubs/sp/800/88/r2/final
