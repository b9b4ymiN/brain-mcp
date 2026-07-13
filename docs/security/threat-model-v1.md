# Threat Model v1

- Status: Phase 0 contract
- Date: 2026-07-13
- System: single-owner Brain vNext; multiple clients/workers; local plus future remote access
- Trust boundaries: untrusted source → quarantine; client → application core; core → canonical stores; core → projections; worker → provider; backup → restore; resource server → Authorization Server

Severity is impact-first: Critical permits authority bypass, plaintext secret/PII exfiltration, purge resurrection, or irreversible canonical corruption; High can corrupt/deny important state or expose scoped private data; Medium has bounded impact or requires strong preconditions; Low is operational hardening.

| ID | Threat | Severity | Owner | Control | Eval/Test IDs | Gate |
|---|---|---|---|---|---|---|
| TM-001 | Indirect prompt injection causes commit/admin/network side effect | Critical | Phase 4 AI worker | Quarantine; source-as-data prompts; typed proposal-only workload identity | `adversarial-001..005` | Phase 4 |
| TM-002 | Hallucinated/poisoned memory becomes confirmed | Critical | Phase 1 claim policy | Non-empty provenance union; proposal gate; unsupported inference cannot confirm | `knowledge-011..015`, `adversarial-001..005` | Phase 1 |
| TM-003 | Secret or `local_only` content reaches provider/log/retry/DLQ | Critical | Phase 4 outbound policy | Deny before serialization; interception tests; redaction preview | `adversarial-006..010` | Phase 4 |
| TM-004 | Stored XSS executes in Console/graph labels | High | Phase 5 Console | DOM text output, sanitizer for authored HTML, CSP, no raw `innerHTML` | `adversarial-011..015` | Phase 5 |
| TM-005 | SSRF, private-IP pivot, or path traversal during ingest/export | Critical | Phase 4 ingestion | URL policy, DNS/IP recheck, private range deny, safe-root paths, size/MIME limits | `adversarial-016..020` | Phase 4 |
| TM-006 | Token for another audience/client is accepted | Critical | Phase 3 auth | RFC 8707 resource binding, issuer/JWKS/aud/azp/capability validation, no passthrough | `adversarial-021`, `adversarial-022` | Phase 3 hard gate |
| TM-007 | Destructive purge/restore occurs without fresh owner intent | Critical | Phase 3/6 auth | `brain.purge`, 5-minute re-auth, 60-second nonce bound to preview hash | `adversarial-023..025` | Phase 3/6 |
| TM-008 | Retry/concurrency creates duplicate or lost semantic transitions | Critical | Phase 1 event store | Serialized transaction; unique `(owner,client,operation)`; canonical request hash | `adversarial-026..030`, `projects-021..025` | Phase 1 |
| TM-009 | Future-valid claim appears current or as-of history is rewritten | High | Phase 1 claim projector | Half-open valid interval; event-sequence as-of; append-only correction | `stocks-001..005`, `knowledge-021..023` | Phase 1 |
| TM-010 | External stock contradiction is silently overwritten | High | Phase 1 claim policy | Disputed coexistence; evidence/time surfaced; abstain/clarify | `stocks-006..010` | Phase 1 |
| TM-011 | Purged content resurrects after crash/backup restore | Critical | Phase 1 purge + Phase 6 restore | Deny-first registry, replicated epoch, key destruction, sealed restore | `knowledge-029`, `adversarial-031`, `adversarial-032`, `adversarial-034`, `adversarial-035`, `adversarial-036` | Phase 1/6 |
| TM-012 | Snapshot/index/Git becomes unauthorized semantic writer | Critical | Phase 1 application core | Ledger-only transition API; projector credentials; replay checksum | `projects-026` through `projects-030` | Phase 1 |
| TM-013 | Public unauthenticated HTTP exposure from current `0.0.0.0` bind | Critical | Phase 3 transport | Loopback/private-test bind until auth suite; TLS and Origin enforcement | `adversarial-033`, `adversarial-021` through `adversarial-025` | Phase 3 hard gate |
| TM-014 | RUSTSEC-2026-0204 invalid pointer dereference in `crossbeam-epoch 0.9.18` | High pending reachability | Phase 0 dependency remediation | Upgrade to patched `>=0.9.20` through compatible Tantivy/Rayon resolution; rerun full suite/audit | `cargo audit`; dependency tree evidence | Before Task 0.3 implementation |
| TM-015 | `anyhow 1.0.102` / `memmap2 0.9.10` unsoundness | High pending reachability | Phase 0 dependency remediation | Upgrade to `anyhow >=1.0.103`, `memmap2 >=0.9.11`; regression/audit | `cargo audit`; affected-function review | Before Task 0.3 implementation |
| TM-016 | Unmaintained `bincode 2.0.1` remains in graph snapshot path | Medium | Phase 0/2 projection owner | Assess replacement/containment; snapshots remain rebuildable/untrusted | `cargo audit`; corrupted snapshot tests | Phase 0 decision |
| TM-017 | Mutable GitHub Action tags execute changed supply-chain code | High | CI owner | Pin actions by full commit SHA; Dependabot-reviewed updates | workflow policy test, `actionlint` | Before protected release CI |
| TM-018 | Ignored/untracked integration `uv.lock` permits dependency drift | Medium | Phase 0 test owner | Track a lock or use a documented reproducible alternative | lock hash/clean-clone integration test | Phase 0 gate |
| TM-019 | Rust tests write generated JSON into tracked fixture tree | Medium | Phase 0 test owner | Redirect generated output to temp; assert clean tree after tests | clean-tree regression | Before protected CI |
| TM-020 | Parallel Windows build hits pagefile error 1455 and hides real regressions | Low | Build owner | MSVC pinned; serialized low-memory command documented; CI remains Linux gate | baseline build command | Operational |
| TM-021 | Keycloak is treated as fully MCP 2025-11-25 compliant without RFC 8707 proof | Critical | Phase 3 auth | Remote-disabled hard gate; exact-image conformance; replace AS if it fails | `adversarial-021..022`, MCP Inspector contract | Phase 3 hard gate |
| TM-022 | Client ID metadata/DCR becomes AS-side SSRF or rogue registration | High | Phase 3 auth | Pre-registration first; CIMD/DCR off by default; host allow-list and trust policy | `adversarial-016`, auth registration negative suite | Phase 3 |
| TM-023 | Git/backup retains deleted keys or plaintext payload | Critical | Phase 1/6 purge | Envelope encryption, epoch rotation, Git rewrite, retention override, verified new backup | `adversarial-031`, `adversarial-032`, `adversarial-034`, `adversarial-035`, `adversarial-036` | Phase 1/6 |
| TM-024 | Owner and partner are falsely attributed as distinct people | Medium integrity/privacy | Application Core | One owner actor by explicit policy; expose client/session only; no person inference | `projects-011`, `knowledge-026` | Phase 1/UI |

## Mandatory risk rules

- A Critical threat cannot be accepted silently. Its gate must pass or the affected feature remains disabled.
- Hard invariants have 100% pass thresholds; retrieval averages cannot offset an auth, purge, provenance, temporal, idempotency, or no-egress failure.
- Task 0.1 open risks TM-014 through TM-020 remain open. This documentation task does not claim remediation.
- Security tests use synthetic values only; no real token, personal document, hostname, provider key, or user data belongs in the corpus.

## Abuse-case review checklist

For each mutation and release gate, review: unauthenticated, wrong owner, wrong audience, missing capability, replay, same key/different payload, concurrent writers, crash at each durable boundary, stale purge registry, poisoned index/snapshot, hostile UTF-8/HTML, private IP/DNS rebinding, oversized archive/decompression, secret in error/log/queue, and rollback to older backup.
