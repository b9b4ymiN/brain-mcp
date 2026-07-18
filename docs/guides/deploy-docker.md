# Deploying Brain MCP with Docker

This guide covers building and running the Brain MCP `llm-wiki` server in a
Docker container on a clean host. It is the **Phase F1.1** baseline — a single
container with a bind-mounted config + data volume. A full Docker Compose
runbook (multi-service, secrets, reverse proxy) lands in Phase F1.2.

## Prerequisites

- Docker Engine 24+ (tested with Docker 28.4.0 + BuildKit)
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

| In-container path | Purpose                                   | Owned by |
|-------------------|-------------------------------------------|----------|
| `/data`           | Wiki repos, indexes, semantic store       | brain    |
| `/backups`        | Optional backup snapshots                 | brain    |
| `/data/config.toml` | Bind-mounted config (read-only)         | brain    |
| `/app/web/console/dist` | Console SPA (baked into image)       | brain    |

> **Why config lives under `/data`:** the binary derives its writable
> `state_dir` from `config_path.parent()` and writes the semantic store +
> indexes there. Mounting config under a read-only `/config/` would make the
> semantic-store create fail. Mount it inside the writable data volume.

## Minimal run

This is the smoke-test shape: a single config file bind-mounted read-only
inside the writable data volume.

```bash
mkdir -p ./data
docker run -d --name brain \
    -p 127.0.0.1:8080:8080 \
    -v "$PWD/data:/data" \
    -v "$PWD/config.toml:/data/config.toml:ro" \
    brain:latest \
    serve --http :8080 --config /data/config.toml
```

### Minimal config

```toml
[serve]
http = true
http_port = 8080
# Bind 0.0.0.0 INSIDE the container so the published port can reach it.
# Keep the host-side publish on 127.0.0.1 unless you have a reverse proxy.
http_bind_address = "0.0.0.0"
http_bind_all_interfaces = true
acp = false
# Dev-grade secret — DO NOT use in production. F1.2 wires `_file:` for
# 1Password / Docker secrets.
console_dev_bootstrap_secret = "CHANGE-ME"
console_static_dir = "/app/web/console/dist"
```

Field names mirror `ServeConfig` in `src/config.rs`.

## Smoke check (post-deploy verification)

`scripts/docker_build_smoke.sh` is the automated version of the manual checks
below. Run it from the repo root:

```bash
bash scripts/docker_build_smoke.sh
```

It builds `brain-smoke:dev`, starts it on `127.0.0.1:18080`, and verifies:

1. `GET /health` returns 200 with `{"uptime_secs":..., "wikis":[...]}`.
2. `POST /api/v1/auth/login` with the wrong secret returns **401**.
3. `POST /api/v1/auth/login` with the right secret returns **200** +
   `{"csrf_token":"..."}`.
4. `GET /` returns the Console SPA (`<title>Brain Console</title>`).
5. `docker history --no-trunc` contains **no** bootstrap secret (it is only
   in the bind-mounted config).
6. `docker run --rm brain-smoke:dev id` reports `uid=1000(brain)`.

Manual equivalents after `docker run`:

```bash
curl -sf http://127.0.0.1:8080/health
curl -i -X POST http://127.0.0.1:8080/api/v1/auth/login \
     -H 'Content-Type: application/json' \
     -d '{"secret":"CHANGE-ME"}'
curl -sf http://127.0.0.1:8080/ | head -n 5
```

## Secrets

**Never bake a production secret into the image.** The Dockerfile does not
COPY any config; the bootstrap secret exists only in the bind-mounted
`/config/config.toml`. Phase F1.2 adds `console_dev_bootstrap_secret_file`
so a secret manager (1Password, Docker secrets, `/run/secrets/...`) can mount
the value without it ever touching disk in plaintext form.

You can confirm a built image is secret-free:

```bash
docker history --no-trunc brain:latest | grep -i secret
# (should print nothing about a real key)
```

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
| `/api/v1/auth/login` always 404                  | `console_dev_bootstrap_secret` not set in config → API router not mounted. |
| `GET /` returns 404                              | `console_static_dir` empty/missing or path wrong.                   |
| Health never ready                               | `docker logs brain`; verify port + bind address in config.          |
| Login page loads but `/api/v1` 403/400           | Cookie/CSRF mismatch — confirm `http_bind_all_interfaces=true` only behind a proxy. |
| `permission denied` writing to `/data`           | Bind-mounted host dir not owned by uid 1000.                        |
| Container exits immediately                      | `docker run` without `--config` → falls back to `~/.llm-wiki/config.toml` which is empty in the image. |

## What's next (Phase F1.2)

- Docker Compose file (`compose.yaml`) with brain + reverse proxy.
- `console_dev_bootstrap_secret_file` for Docker secrets.
- Health + readiness wiring for orchestrators (k8s/Swarm).
- Multi-platform build (`linux/amd64` + `linux/arm64`) via `docker buildx`.
