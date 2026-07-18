# Brain MCP — multi-stage Dockerfile (Phase F1.1)
#
# Stages:
#   1. console-builder  — builds the Svelte 5 + Vite Console SPA into /console/dist
#   2. builder          — compiles the `llm-wiki` Rust binary (release profile)
#   3. runtime          — minimal debian-slim with the binary + Console assets
#
# Reproducibility notes:
#   * `rusqlite = "=0.40.1"` with `features=["bundled"]` → sqlite is statically
#     linked; no system libsqlite3 needed at runtime.
#   * `git2 = { features=["vendored-libgit2"] }` → libgit2 built from source.
#   * `reqwest` uses `rustls-tls` (no openssl); only `ca-certificates` at runtime.
#
# The shipping runtime image MUST NOT contain any production secret. The smoke
# config in `scripts/docker-smoke-config.toml` is bind-mounted at run time and
# never COPYied into the image — see `.dockerignore` + secret-scan DoD.

# ─────────────────────────────────────────────────────────────────────────────
# Stage 1: build the Console SPA (Svelte 5 + Vite)
# ─────────────────────────────────────────────────────────────────────────────
FROM node:20-bookworm AS console-builder
WORKDIR /console
# Layer-cache: install deps before copying source.
COPY web/console/package.json web/console/package-lock.json ./
RUN npm ci --no-audit --no-fund
# Copy the rest of the Console source and build.
COPY web/console/ ./
RUN npm run build
# Output: /console/dist/

# ─────────────────────────────────────────────────────────────────────────────
# Stage 2: build the Rust binary
# ─────────────────────────────────────────────────────────────────────────────
FROM rust:1.95-bookworm AS builder
WORKDIR /build
# Copy manifests first for layer caching.
COPY Cargo.toml Cargo.lock ./
# Source tree. `src/` is the binary + library; `examples/` is opt-in per Cargo
# semantics (not built by `cargo build --bin`, but kept for parity with local
# workflows + `cargo build --examples`). The two `include_str!` roots the binary
# build depends on must also be present:
#   * `schemas/*.json`      — embedded type schemas (`src/default_schemas.rs`)
#   * `web/hugo-cms/**`     — embedded Hugo templates (`src/web.rs`)
COPY src/ ./src/
COPY examples/ ./examples/
COPY schemas/ ./schemas/
COPY web/hugo-cms/ ./web/hugo-cms/
# Build with BuildKit cache mounts for cargo registry + target dir. The cache
# mounts are not baked into the image layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin llm-wiki && \
    cp /build/target/release/llm-wiki /llm-wiki

# ─────────────────────────────────────────────────────────────────────────────
# Stage 3: minimal runtime image
# ─────────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
# Runtime deps:
#   * ca-certificates  — TLS trust roots (reqwest rustls).
#   * git              — git-backed store + history commands.
#   * tini             — PID 1 / signal forwarding / zombie reaping.
#   * curl             — used by HEALTHCHECK (alternatively `llm-wiki --help`).
# No libsqlite3 / libgit2 / openssl needed (all statically linked into the bin).
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        tini \
        curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user (uid 1000 — matches DoD: container runs as non-root).
RUN useradd --uid 1000 --create-home --shell /bin/bash brain

# Copy the binary from the builder stage.
COPY --from=builder /llm-wiki /usr/local/bin/llm-wiki
# Copy the Console SPA from the console-builder stage.
COPY --from=console-builder /console/dist /app/web/console/dist

# Volumes mount points (created with brain ownership so bind mounts work).
RUN mkdir -p /data /backups && chown -R brain:brain /data /backups /app

USER brain
WORKDIR /data

EXPOSE 8080

# tini reaps zombies and forwards SIGTERM; the ENTRYPOINT is the binary so the
# CMD is just the subcommand + flags (compose / `docker run` overrides apply).
ENTRYPOINT ["/usr/bin/tini", "--", "llm-wiki"]
# Default args: `serve --http :8080`. Bind is loopback by default; the smoke
# config / compose sets `http_bind_all_interfaces = true` for in-container 0.0.0.0.
CMD ["serve", "--http", ":8080"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:8080/health || exit 1
