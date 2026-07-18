# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✓         |

## Reporting a Vulnerability

Please report security vulnerabilities by email to <jguibert@gmail.com>.

**Do not open a public issue.**

You should receive an acknowledgment within 48 hours. A fix will be
prioritized based on severity and released as a patch version.

## Scope

llm-wiki is a local-first semantic-knowledge engine that, as of v0.5,
optionally exposes three networked surfaces when run via `llm-wiki serve`:

- **MCP/ACP transport** over stdio or HTTP/SSE (`/mcp`) — used by AI clients
- **Console HTTP API** at `/api/v1/*` — Svelte 5 admin UI (search, review,
  destructive ops) authenticated via session cookie + CSRF
- **Outbound HTTPS provider calls** — Z.ai adapter (Phase D) makes
  real network calls when configured

Plus the always-present:

- Malicious Markdown / source files processed by the ingest pipeline
- Semantic-store encryption (AES-GCM at rest + per-object crypto-shred keys)
- Destructive operations (entity merge/split/retract + hard purge) with
  server-enforced guard chain (preview → recent re-auth → nonce → execute)

The HTTP server binds **loopback by default** (`127.0.0.1`); binding
`0.0.0.0` requires explicit opt-in (`serve.http_bind_all_interfaces = true`)
and should sit behind a reverse proxy with TLS + auth in production.

Dependency vulnerabilities are tracked via `cargo audit` (clean as of the
Phase F external review) and Dependabot alerts. See
[`docs/security/phase-F-review.md`](docs/security/phase-F-review.md) for the
most recent external security review.
