# Deploying Brain MCP with Docker

This guide covers building and running the Brain MCP `llm-wiki` server in a
Docker container on a clean host. It is the **Phase F1.2** baseline — a
single container with bind-mounted config + data volumes, Docker-secret
bootstrap auth, and a Compose runbook. The bare `docker run` shape from
F1.1 is preserved below for debugging.

## Prerequisites

- Docker Engine 24+ with Compose v2 (`docker compose ...`; tested with
  Docker 28.4.0 + BuildKit).
- A clean Linux host (amd64). Windows + Docker Desktop also works for local
  testing via Git Bash.
- ~3 GB free disk for the image and the build cache.
- A reachable wiki git repo (or one you will create inside the data volume).

## Build the image

From the repo root:

```bash
DOCKER_BUILDKIT=1 docker build -t brain:latest .
```

First build is slow (10–15 min) because the Rust release profile uses LTO +
`codegen-units=1` for a small, stripped binary. Subsequent builds reuse the
cargo registry + target cache via BuildKit cache mounts.

## What's in the image

| Stage        | Base                | Output                                          |
|--------------|---------------------|-------------------------------------------------|
| console      | `node:20-bookworm`  | `/app/web/console/dist` (Svelte + Vite build)   |
| builder      | `rust:1.95-bookworm`| `/usr/local/bin/llm-wiki` (release, stripped)   |
| runtime      | `debian:bookworm-slim` | binary + Console dist + `tini`/`git`/`curl` |

Runtime dependencies that ship in the image: `ca-certificates`, `git`, `tini`,
`curl`. SQLite (`rusqlite = "0.40.1"` bundled) and libgit2 (`vendored-libgit2`)
are statically compiled into the binary — no system packages required.

The container runs as **non-root user `brain` (uid 1000)**.

## Volume layout

| Host path                       | In-container path             | Mode    | Purpose                                              |
|---------------------------------|-------------------------------|---------|------------------------------------------------------|
| `./data/`                       | `/data`                       | rw      | Wiki repos, indexes, semantic store, state dir      |
| `./backups/`                    | `/backups`                    | rw      | Encrypted backup snapshots (Phase F3.1)              |
| `./config/config.toml`          | `/data/config.toml`           | ro      | Operator's config (single-file bind)                 |
| `./secrets/bootstrap_secret.txt`| `/run/secrets/bootstrap_secret` | ro    | Bootstrap secret (Docker secret, mode 0600 on host)  |
| (baked in)                      | `/app/web/console/dist`       | —       | Console SPA (image layer, not a bind mount)          |

> **Why config lives under `/data`:** the binary derives its writable
> `state_dir` from `config_path.parent()` (`src/engine.rs:102`) and writes the
> semantic store + indexes there. Mounting config under a read-only `/config/`
> would make the semantic-store create fail. Mount the config **file** inside
> the writable data volume (`./config/config.toml:/data/config.toml:ro`),
> keep the host-side config directory read-only at the operator level.

All four host directories (`data/`, `backups/`, `config/`, `secrets/`) are
gitignored — see `.gitignore`. The tracked template is
`examples/config.docker.toml`.

## Quick start — Docker Compose (recommended)

The shipped `docker-compose.yml` is a single-service production-style stack.
The bootstrap secret flows through a Docker secret file
(`/run/secrets/bootstrap_secret`), NOT through env (env leaks via
`docker inspect`).

### 1. Set up the operator tree

```bash
mkdir -p ./data ./backups ./config ./secrets
cp examples/config.docker.toml ./config/config.toml
```

### 2. Generate the bootstrap secret

```bash
# Mode 0600 on the host — only the operator account can read it. Docker
# surfaces it read-only inside the container at /run/secrets/bootstrap_secret.
( umask 077 && \
  printf '%s' "$(openssl rand -hex 32)" > ./secrets/bootstrap_secret.txt )
chmod 600 ./secrets/bootstrap_secret.txt
```

The `examples/config.docker.toml` reads this via
`console_dev_bootstrap_secret_file = "/run/secrets/bootstrap_secret"`. The
config never contains the secret value.

### 3. Bring it up

```bash
docker compose up -d --build    # build fresh on first run
docker compose logs -f brain
```

### 4. Verify

```bash
curl -sf http://127.0.0.1:8080/health
```

Tear down:

```bash
docker compose down             # keep volumes
docker compose down -v          # also remove anonymous volumes (NOT your
                                # bind-mounted ./data — that's a host dir)
```

The automated end-to-end check is `scripts/docker_compose_smoke.sh` (see
[Smoke checks](#smoke-checks) below).

## Bootstrap secret management

The Console HTTP API (`/api/v1`) is gated by a bootstrap secret compared via
`constant_time_eq` (`src/api.rs`). For production the secret MUST come from a
file, not from the config or the environment.

| Field                                            | Source                  | Use case                                   |
|--------------------------------------------------|-------------------------|--------------------------------------------|
| `console_dev_bootstrap_secret`                   | plaintext in TOML       | local dev only — never in production       |
| `console_dev_bootstrap_secret_file` *(F1.2)*     | file on disk            | Docker secrets, systemd `LoadCredential`   |

Resolution priority is **file > direct string**: if both are set, the file
wins (more secure). Fail-closed: a set `_file` path that cannot be read is a
fatal startup error — the server does NOT fall back to an empty secret or to
the direct string. The file's contents are trimmed (so a trailing newline
from `printf '%s\n'` is dropped).

### Why not env?

Environment values are visible in `docker inspect <container> --format
'{{.Config.Env}}'` to anyone with read access to the Docker socket. A
bind-mounted file with mode 0600 is not. The compose smoke
(`scripts/docker_compose_smoke.sh`) asserts the secret is **absent** from
`Config.Env` as a security gate.

### Confirming an image is secret-free

```bash
# The bootstrap secret value is the thing you DO NOT want in the image.
# Scan history + layers for the operator's actual secret string:
SECRET="$(cat ./secrets/bootstrap_secret.txt)"
docker history --no-trunc brain:latest | grep -F -- "$SECRET" && \
    echo "LEAK: secret baked into image" || echo "OK: image history clean"
```

`grep -F -- "$SECRET"` matches the **literal** secret value (no regex
interpretation, no false positives from the word "secret" appearing in
unrelated layer commands). The compose smoke automates this check.

## Minimal run (bare `docker run`, no Compose)

This is the F1.1 smoke shape: a single config file bind-mounted read-only
inside the writable data volume. Useful for debugging Compose issues.

```bash
mkdir -p ./data
docker run -d --name brain \
    -p 127.0.0.1:8080:8080 \
    -v "$PWD/data:/data" \
    -v "$PWD/config.toml:/data/config.toml:ro" \
    brain:latest \
    serve --http :8080 --config /data/config.toml
```

Minimal config:

```toml
[serve]
http = true
http_port = 8080
# Bind 0.0.0.0 INSIDE the container so the published port can reach it.
# Keep the host-side publish on 127.0.0.1 unless you have a reverse proxy.
http_bind_address = "0.0.0.0"
http_bind_all_interfaces = true
acp = false
# Dev-grade secret — DO NOT use in production. Use
# console_dev_bootstrap_secret_file (see Bootstrap secret management above).
console_dev_bootstrap_secret = "CHANGE-ME"
console_static_dir = "/app/web/console/dist"
```

Field names mirror `ServeConfig` in `src/config.rs`.

## Smoke checks (post-deploy verification)

Two scripts, both safe to run from the repo root:

| Script                                | Shape                   | When to use                                            |
|---------------------------------------|-------------------------|--------------------------------------------------------|
| `scripts/docker_build_smoke.sh`       | bare `docker run`       | F1.1 fast image-build gate (~4 min)                    |
| `scripts/docker_compose_smoke.sh`     | full `docker compose`   | F1.2 production-shape gate (file secret + inspect Env) |

The compose smoke is the stronger gate. It builds fresh, brings the stack up
via `docker-compose.yml` (pointed at a per-run scratch tree under
`.docker-compose-smoke/`), and verifies:

1. `GET /health` returns 200 with `{"uptime_secs":..., "wikis":[...]}`.
2. `POST /api/v1/auth/login` with the wrong secret returns **401**.
3. `POST /api/v1/auth/login` with the right secret (read from
   `/run/secrets/bootstrap_secret`) returns **200** + `{"csrf_token":"..."}`.
4. `GET /` returns the Console SPA (`<title>Brain Console</title>`).
5. **Security gate A:** the secret value is NOT in
   `docker inspect brain --format '{{.Config.Env}}'`.
6. **Security gate B:** `/run/secrets/bootstrap_secret` IS in the container's
   `Mounts` (the file indirection is actually wired).
7. `docker history --no-trunc` contains **no** bootstrap secret.
8. The container reports `uid=1000(brain)`.

Run it:

```bash
bash scripts/docker_compose_smoke.sh
```

Manual equivalents after `docker compose up`:

```bash
curl -sf http://127.0.0.1:8080/health
curl -i -X POST http://127.0.0.1:8080/api/v1/auth/login \
     -H 'Content-Type: application/json' \
     -d "{\"secret\":\"$(cat ./secrets/bootstrap_secret.txt)\"}"
curl -sf http://127.0.0.1:8080/ | head -n 5
```

## Production hardening checklist

The shipped Compose stack is **loopback-only** (`127.0.0.1:8080:8080`). For
any non-local access, put a reverse proxy in front. Recommended baseline:

- [ ] **Reverse proxy with TLS.** Run nginx / caddy / Traefik in a sibling
      Compose service (or on the host) that terminates TLS and forwards to
      `brain:8080` over the Compose network. See the upstream
      [nginx HTTPS guide](https://nginx.org/en/docs/http/configuring_https_servers.html)
      or [caddy automatic HTTPS docs](https://caddyserver.com/docs/automatic-https).
      The Brain container itself does not terminate TLS.
- [ ] **No direct port publish.** Once the proxy is up, drop the
      `ports:` block from `docker-compose.yml` (or keep it loopback-only for
      debugging) and expose the Console only via the proxy.
- [ ] **Bootstrap secret via file.** Never via `environment:` (env leaks via
      `docker inspect`). See [Bootstrap secret management](#bootstrap-secret-management).
- [ ] **Mode 0600 on `./secrets/bootstrap_secret.txt`.** Verified by the
      compose smoke (Docker secret files are mounted read-only inside the
      container regardless of host mode; the host mode restricts who on the
      host can read it).
- [ ] **`RUST_LOG` tuned.** `info` for production; drop to `debug`
      temporarily for diagnosis. Avoid `trace` in production (PII risk from
      request bodies).
- [ ] **Log format JSON.** `examples/config.docker.toml` sets
      `logging.log_format = "json"` for aggregator-friendly parsing. Flip to
      `text` for local dev.
- [ ] **Resource limits.** Add `mem_limit` / `cpus:` to the service once
      Phase F2.2 (`/metrics`) lands so you can right-size from actual usage.
- [ ] **Backups.** `./backups/` is wired as a volume; the F3.1 encrypted
      backup tooling writes here. Verify your backup rotation externally.
- [ ] **Health-based restart.** The compose `healthcheck` + `restart:
      unless-stopped` will restart on `curl /health` failure; verify the
      `start_period` is long enough for your wiki size on cold start.

## Logs

The container logs to stdout/stderr (`tracing_subscriber`) — `docker logs brain`
is the primary log surface. Rolling file logs (if enabled in `[logging]`) land
under `/data/logs/` inside the container (i.e. your bind-mounted data dir).

```bash
docker logs -f brain
docker logs --tail 100 brain
```

## Troubleshooting

| Symptom                                          | Likely cause / fix                                                  |
|--------------------------------------------------|---------------------------------------------------------------------|
| `docker build` fails on `cargo build`            | Check `Cargo.lock` is committed; rerun with `--no-cache` for the builder stage. |
| `/api/v1/auth/login` always 404                  | Bootstrap secret resolution produced `None`/empty → API router not mounted. Check `console_dev_bootstrap_secret_file` is set in the config and the file is readable by uid 1000. |
| `/api/v1/auth/login` always 401                  | Secret in the request doesn't match the resolved secret. The resolved value is trimmed — confirm `printf '%s'` (not `echo`) wrote the file, or that there are no stray bytes. |
| `GET /` returns 404                              | `console_static_dir` empty/missing or path wrong.                   |
| Health never ready                               | `docker logs brain`; verify port + bind address in config.          |
| Login page loads but session cookie rejected     | The `Secure` cookie flag is set (because `http_bind_all_interfaces=true`) but the client reached the API over plain HTTP (not behind a TLS proxy). Either run behind TLS, or for local-only dev set `http_bind_all_interfaces=false` (loopback → no `Secure` flag). |
| Startup error: "failed to read console_dev_bootstrap_secret_file" | The configured secret file path does not exist or is unreadable inside the container. This is fail-closed by design — do not work around it; fix the path/permissions. |
| `permission denied` writing to `/data`           | Bind-mounted host dir not owned by uid 1000.                        |
| Container exits immediately                      | Config not found — verify `LLM_WIKI_CONFIG=/data/config.toml` is set and the bind mount reached the container. |
| Compose: secret leaks into `docker inspect` Env  | Should never happen with the shipped compose (secret is via `secrets:`, not `environment:`). The smoke script asserts this; if it fails, an operator added the secret value to `environment:` by mistake. |

## What's next (Phase F2)

- OAuth-gated Console auth (replaces the bootstrap secret for multi-user).
- `/metrics` Prometheus endpoint for orchestrator-driven autoscaling
  (Phase F2.2).
- Multi-platform build (`linux/amd64` + `linux/arm64`) via `docker buildx`.
- Helm chart for kubernetes deployment.
