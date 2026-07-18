#!/usr/bin/env bash
# scripts/docker_build_smoke.sh
#
# Phase F1.1 smoke test: build the image, run it on 127.0.0.1:18080, and
# verify the Console + API surface is reachable + auth gates work.
#
# Usage:
#   bash scripts/docker_build_smoke.sh             # uses brain-smoke:dev
#   bash scripts/docker_build_smoke.sh my-tag:latest
#
# Env:
#   SECRET  (default: dev-smoke-secret) — must match `console_dev_bootstrap_secret`
#            in scripts/docker-smoke-config.toml.
#
# Designed to run under Git Bash on Windows; uses absolute paths for mounts.

set -euo pipefail

# NOTE on Windows + Git Bash: we scope `MSYS_NO_PATHCONV=1` to the `docker run`
# invocations only. Without it, MSYS rewrites the container-side path
# `/data/config.toml` into a Windows path before docker sees it, which silently
# breaks the single-file config bind mount (the container then sees defaults
# and binds 127.0.0.1 only).

# ── Resolve repo root (so the script works from any CWD) ─────────────────────
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

IMAGE_TAG="${1:-brain-smoke:dev}"
SECRET="${SECRET:-dev-smoke-secret}"
SMOKE_PORT="${SMOKE_PORT:-18080}"
CONTAINER_NAME="${CONTAINER_NAME:-brain-smoke}"

CONFIG_SRC="$SCRIPT_DIR/docker-smoke-config.toml"
DATA_DIR="$REPO_ROOT/.docker-smoke-data"

echo "[smoke] repo root : $REPO_ROOT"
echo "[smoke] image tag : $IMAGE_TAG"
echo "[smoke] port      : 127.0.0.1:${SMOKE_PORT} -> 8080"
echo "[smoke] config    : $CONFIG_SRC"

if [[ ! -f "$CONFIG_SRC" ]]; then
    echo "[smoke] FAIL: $CONFIG_SRC missing" >&2
    exit 2
fi

# Pre-clean any stale container from a previous aborted run.
docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
# Pre-clean smoke data dir (idempotent — keep it small + reproducible).
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

# ── Build ────────────────────────────────────────────────────────────────────
echo "[smoke] building image $IMAGE_TAG ..."
# BuildKit is required for the cache mounts in the Dockerfile.
DOCKER_BUILDKIT=1 docker build -t "$IMAGE_TAG" "$REPO_ROOT"

# ── Run ──────────────────────────────────────────────────────────────────────
echo "[smoke] starting container ..."
# The binary derives `state_dir` from `config_path.parent()` and writes the
# semantic store + indexes there, so the config MUST live under a writable
# directory. We mount it inside /data (the writable volume) rather than under
# a read-only /config mount.
#
# `MSYS_NO_PATHCONV=1` is scoped to the `docker run` call so the container-side
# `/data/...` paths are not rewritten by Git Bash into Windows paths (which
# would silently break the single-file config bind mount).
CID=$(MSYS_NO_PATHCONV=1 docker run -d \
    --name "$CONTAINER_NAME" \
    -p "127.0.0.1:${SMOKE_PORT}:8080" \
    -v "$DATA_DIR:/data" \
    -v "$CONFIG_SRC:/data/config.toml:ro" \
    "$IMAGE_TAG" \
    serve --http :8080 --config /data/config.toml)
echo "[smoke] container id: $CID"

cleanup() {
    echo "[smoke] cleaning up container $CID ..."
    docker stop "$CID" >/dev/null 2>&1 || true
    docker rm -f "$CID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# ── Wait for /health ─────────────────────────────────────────────────────────
echo "[smoke] waiting for /health (up to 60s) ..."
HEALTHY=""
for i in $(seq 1 60); do
    if curl -sf "http://127.0.0.1:${SMOKE_PORT}/health" >/dev/null 2>&1; then
        echo "[smoke] healthy after ${i}s"
        HEALTHY="1"
        break
    fi
    # Surface container logs on slow boots for debugging.
    if (( i % 10 == 0 )); then
        echo "[smoke]   ...still waiting at ${i}s. Recent logs:"
        docker logs --tail 10 "$CID" 2>&1 | sed 's/^/            /' || true
    fi
    sleep 1
done

if [[ -z "$HEALTHY" ]]; then
    echo "[smoke] FAIL: /health never came up. Full container logs:"
    docker logs "$CID" 2>&1 | sed 's/^/    /'
    exit 1
fi

# ── Check /health response shape (uptime_secs + wikis) ───────────────────────
echo "[smoke] checking /health body ..."
HEALTH_BODY="$(curl -sf "http://127.0.0.1:${SMOKE_PORT}/health")"
echo "[smoke]   /health -> $HEALTH_BODY"
echo "$HEALTH_BODY" | grep -q '"uptime_secs"' || { echo "[smoke] FAIL: /health missing uptime_secs"; exit 1; }
echo "$HEALTH_BODY" | grep -q '"wikis"'       || { echo "[smoke] FAIL: /health missing wikis";       exit 1; }

# ── Auth gate: wrong secret -> 401 ───────────────────────────────────────────
echo "[smoke] checking /api/v1/auth/login with WRONG secret (expect 401) ..."
CODE=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "http://127.0.0.1:${SMOKE_PORT}/api/v1/auth/login" \
    -H 'Content-Type: application/json' \
    -d '{"secret":"wrong"}')
echo "[smoke]   -> HTTP $CODE"
[[ "$CODE" = "401" ]] || { echo "[smoke] FAIL: expected 401 got $CODE"; exit 1; }

# ── Auth gate: correct secret -> 200 + csrf_token ────────────────────────────
echo "[smoke] checking /api/v1/auth/login with CORRECT secret (expect 200) ..."
LOGIN_BODY=$(curl -s -w "\n%{http_code}" \
    -X POST "http://127.0.0.1:${SMOKE_PORT}/api/v1/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"secret\":\"$SECRET\"}")
CODE=$(echo "$LOGIN_BODY" | tail -n1)
BODY=$(echo "$LOGIN_BODY" | sed '$d')
echo "[smoke]   -> HTTP $CODE body=$BODY"
[[ "$CODE" = "200" ]] || { echo "[smoke] FAIL: expected 200 got $CODE"; exit 1; }
echo "$BODY" | grep -q '"csrf_token"' || { echo "[smoke] FAIL: login response missing csrf_token"; exit 1; }

# ── Console static index served at / (fallback) ──────────────────────────────
echo "[smoke] checking Console static index at / ..."
INDEX_BODY="$(curl -sf "http://127.0.0.1:${SMOKE_PORT}/")"
echo "$INDEX_BODY" | grep -q '<title>Brain Console</title>' \
    || { echo "[smoke] FAIL: console index missing <title>Brain Console</title>"; exit 1; }
echo "[smoke]   console index OK ($(echo "$INDEX_BODY" | wc -l) lines)"

# ── Secret-scan gate: the secret must NOT be in image history ────────────────
echo "[smoke] secret-scan: ensuring '$SECRET' is NOT baked into the image ..."
if docker history --no-trunc "$IMAGE_TAG" 2>/dev/null | grep -F -q "$SECRET"; then
    echo "[smoke] FAIL: secret leaked into image history"
    docker history --no-trunc "$IMAGE_TAG" | grep -F "$SECRET" | sed 's/^/    /' || true
    exit 1
fi
echo "[smoke]   image history is clean"

# ── Non-root gate ────────────────────────────────────────────────────────────
echo "[smoke] non-root: checking uid inside container ..."
# Same MSYS_NO_PATHCONV story for the entrypoint override (`/bin/sh`).
ID_OUT="$(MSYS_NO_PATHCONV=1 docker run --rm --entrypoint /bin/sh "$IMAGE_TAG" -c 'id')"
echo "[smoke]   -> $ID_OUT"
echo "$ID_OUT" | grep -Eq 'uid=1000\(brain\)' \
    || { echo "[smoke] FAIL: container not running as brain(uid=1000): $ID_OUT"; exit 1; }

echo ""
echo "[smoke] PASS — all checks green"
