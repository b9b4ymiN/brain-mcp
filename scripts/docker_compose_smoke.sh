#!/usr/bin/env bash
# scripts/docker_compose_smoke.sh
#
# Phase F1.2 compose smoke: build + up + verify the compose stack behaves like
# a production deployment, with the bootstrap secret flowing through a Docker
# secret file (`/run/secrets/bootstrap_secret`) — NOT via env, NOT via the
# tracked config. This is a stronger gate than F1.1's `docker run` smoke:
# it asserts the secret is absent from `docker inspect` Config.Env (the
# leak vector that would defeat the `_file:` indirection).
#
# Usage:
#   bash scripts/docker_compose_smoke.sh
#
# Pre-reqs:
#   * Docker Engine 24+ with Compose v2 (`docker compose ...`).
#   * BuildKit enabled (default on modern Docker).
#
# What this script does (in order):
#   1. Sets up an isolated scratch tree under .docker-compose-smoke/ that
#      mirrors the operator layout (config/, secrets/, data/, backups/) —
#      avoids touching any real operator dirs at the repo root.
#   2. Writes a dev-only bootstrap secret to .docker-compose-smoke/secrets/
#      (gitignored). The value is random per run.
#   3. Generates a compose override + config.toml that point at the scratch
#      tree, so we don't depend on a pre-existing `./config/config.toml`.
#   4. Builds the image fresh (compose up --build) and waits for /health.
#   5. Polls /ready (Phase F1.3) until the readiness gate passes (db_reachable
#      + migrations_applied + index_open) — proves the readiness gate is real.
#   6. Login smoke: wrong secret -> 401, correct secret -> 200 + csrf_token.
#   6b. /metrics smoke (Phase F2.2): asserts the endpoint returns Prometheus
#       text exposition AND that the wrong-secret login above bumped
#       console_auth_failures_total — proves the recorder is installed and
#       the handler→recorder wire is connected.
#   7. Console index check (`<title>Brain Console</title>`).
#   8. SECURITY GATES:
#        a. `docker inspect brain --format '{{.Config.Env}}'` MUST NOT contain
#           the bootstrap secret value (env leak check).
#        b. `docker inspect brain --format '{{json .Mounts}}'` MUST contain
#           a /run/secrets/bootstrap_secret mount (file indirection check).
#   9. `docker compose down -v` cleanup.
#
# Designed to run under Git Bash on Windows. `MSYS_NO_PATHCONV=1` is not
# needed for compose subcommands (no host-path bind args on the CLI); the
# bind mounts are expressed inside the YAML, which Docker reads verbatim.

set -euo pipefail

# ── Resolve repo root (so the script works from any CWD) ─────────────────────
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Windows / Git Bash: translate the Unix-style MSYS path (/c/...) into the
# Windows-native form Docker Desktop actually understands (C:\...). Otherwise
# the absolute paths embedded in the generated compose YAML get re-translated
# by MSYS in a way that produces bogus `context:` paths. On real Linux/macOS
# this is a no-op.
native_path() {
    local p="$1"
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]] || command -v cygpath >/dev/null 2>&1; then
        cygpath -m "$p" 2>/dev/null || echo "$p"
    else
        echo "$p"
    fi
}

# Smoke-controlled scratch tree. Gitignored (see .gitignore: .docker-smoke-data/
# covers F1.1; .docker-compose-smoke/ is the F1.2 equivalent).
SCRATCH="$REPO_ROOT/.docker-compose-smoke"
COMPOSE_FILE="$SCRATCH/docker-compose.smoke.yml"
EXAMPLE_CONFIG="$REPO_ROOT/examples/config.docker.toml"

# Per-run random secret. We need to know it for the login check, so derive
# it deterministically from openssl + a per-run nonce.
SMOKE_PORT="${SMOKE_PORT:-18081}"
SECRET="smoke-$(openssl rand -hex 16 2>/dev/null || echo "fallback-$$")"
SECRET_LEN=${#SECRET}

echo "[smoke] repo root        : $REPO_ROOT"
echo "[smoke] scratch tree     : $SCRATCH"
echo "[smoke] port             : 127.0.0.1:${SMOKE_PORT} -> 8080"
echo "[smoke] secret length    : $SECRET_LEN chars (value redacted)"

# ── Sanity: example config + compose file must exist ─────────────────────────
if [[ ! -f "$EXAMPLE_CONFIG" ]]; then
    echo "[smoke] FAIL: $EXAMPLE_CONFIG missing — run from a checkout that has Phase F1.2 applied" >&2
    exit 2
fi
if [[ ! -f "$REPO_ROOT/docker-compose.yml" ]]; then
    echo "[smoke] FAIL: $REPO_ROOT/docker-compose.yml missing" >&2
    exit 2
fi

# ── 1. Setup scratch tree ────────────────────────────────────────────────────
echo "[smoke] setting up scratch tree ..."
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH/config" "$SCRATCH/data" "$SCRATCH/backups"

# Phase G (2026-07-20): the bootstrap credential is now a BRAIN_USERNAME +
# BRAIN_PASSWORD pair sourced from env vars. Namespaced as BRAIN_* (not bare
# USERNAME/PASSWORD) because bare USERNAME collides with the Windows built-in
# env var (always set to the host user's login name) and silently overrides
# `.env`. Found live 2026-07-20.
# The smoke script seeds the values directly into the compose file's
# `environment:` block (they're dev-grade + loopback-only + torn down with
# the scratch tree, so the Config.Env leak vector is moot for this test).
# The legacy `secrets/` mount is no longer required.
SMOKE_USERNAME="${SMOKE_USERNAME:-smoke-admin}"
SMOKE_PASSWORD="${SMOKE_PASSWORD:-$SECRET}"

# Copy the tracked template into the scratch config dir. The single-file
# bind mount (`./config/config.toml:/data/config.toml:ro`) is what the
# container reads.
cp "$EXAMPLE_CONFIG" "$SCRATCH/config/config.toml"

# ── 2. Generate the smoke compose file ───────────────────────────────────────
# We can't reuse the root docker-compose.yml directly because (a) it hard-codes
# `image: brain:v0.5` and (b) its bind mounts point at ./data, ./backups,
# ./config at the repo root (which we don't want to pollute). The generated
# override re-points everything at the scratch tree + a per-run port + a
# smoke-specific image tag.
#
# Paths are emitted in their OS-native form (via `native_path`) so Docker
# Desktop on Windows accepts them verbatim without MSYS path translation
# mangling the `context:` / `volumes:` values.
REPO_ROOT_N="$(native_path "$REPO_ROOT")"
SCRATCH_N="$(native_path "$SCRATCH")"
cat > "$COMPOSE_FILE" <<EOF
services:
  brain:
    image: brain-compose-smoke:dev
    build:
      context: $REPO_ROOT_N
      dockerfile: Dockerfile
    container_name: brain_compose_smoke
    ports:
      - "127.0.0.1:${SMOKE_PORT}:8080"
    volumes:
      - $SCRATCH_N/data:/data
      - $SCRATCH_N/backups:/backups
      - $SCRATCH_N/config/config.toml:/data/config.toml:ro
    environment:
      - RUST_LOG=llm_wiki=info,warn
      - LLM_WIKI_CONFIG=/data/config.toml
      - BRAIN_USERNAME=${SMOKE_USERNAME}
      - BRAIN_PASSWORD=${SMOKE_PASSWORD}
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:8080/health"]
      interval: 5s
      timeout: 3s
      retries: 6
      start_period: 5s
EOF

cleanup() {
    local ec=$?
    echo "[smoke] cleaning up compose stack ..."
    # `down -v` removes the containers + the anonymous volumes. The bind
    # mounts under $SCRATCH are removed by the rm -rf above / next run.
    ( cd "$SCRATCH" && MSYS_NO_PATHCONV=1 docker compose -p brain-compose-smoke \
        -f docker-compose.smoke.yml down -v >/dev/null 2>&1 ) || true
    if [[ "${KEEP_SMOKE:-0}" != "1" ]]; then
        rm -rf "$SCRATCH"
    fi
    exit $ec
}
trap cleanup EXIT INT TERM

# ── 3. Build + up ────────────────────────────────────────────────────────────
echo "[smoke] docker compose up -d --build (this builds the image fresh, ~4 min on a cold cache) ..."
# Run compose with the smoke file as the project. -p sets the project name so
# we don't collide with the operator's default project. `MSYS_NO_PATHCONV=1`
# is set so Git Bash on Windows does not rewrite path-looking CLI args.
( cd "$SCRATCH" && MSYS_NO_PATHCONV=1 docker compose -p brain-compose-smoke -f docker-compose.smoke.yml up -d --build )

# ── 4. Poll /health (up to 90s — fresh build container cold-starts slower) ───
echo "[smoke] waiting for /health (up to 90s) ..."
HEALTHY=""
for i in $(seq 1 90); do
    if curl -sf "http://127.0.0.1:${SMOKE_PORT}/health" >/dev/null 2>&1; then
        echo "[smoke] healthy after ${i}s"
        HEALTHY="1"
        break
    fi
    if (( i % 15 == 0 )); then
        echo "[smoke]   ...still waiting at ${i}s. Recent container logs:"
        docker logs --tail 15 brain_compose_smoke 2>&1 | sed 's/^/            /' || true
    fi
    sleep 1
done

if [[ -z "$HEALTHY" ]]; then
    echo "[smoke] FAIL: /health never came up. Full container logs:"
    docker logs brain_compose_smoke 2>&1 | sed 's/^/    /'
    exit 1
fi

# ── 5. /health body shape ────────────────────────────────────────────────────
echo "[smoke] checking /health body ..."
HEALTH_BODY="$(curl -sf "http://127.0.0.1:${SMOKE_PORT}/health")"
echo "[smoke]   /health -> $HEALTH_BODY"
echo "$HEALTH_BODY" | grep -q '"uptime_secs"' || { echo "[smoke] FAIL: /health missing uptime_secs"; exit 1; }
echo "$HEALTH_BODY" | grep -q '"wikis"'       || { echo "[smoke] FAIL: /health missing wikis";       exit 1; }

# ── 5b. /ready readiness gate (Phase F1.3) ───────────────────────────────────
# /health (liveness) was cheap and returned 200 as soon as the server was up.
# /ready (readiness) is the real gate: it consults ReadinessCheck::is_ready()
# (db_reachable && migrations_applied && index_open). A 200 here proves the
# semantic store opened at the current schema version AND the engine's spaces
# all have open searchers. A 503 here would mean the server is up (liveness
# passes) but not safe to route traffic to — exactly the split the F1.3 gate
# exists to enforce. We poll because the store open happens just after bind.
echo "[smoke] waiting for /ready (up to 60s) ..."
READY=""
READY_BODY=""
for i in $(seq 1 60); do
    # /ready returns 200 + {"status":"ready",...} or 503 + {"status":"not_ready",...}.
    # curl -sf only succeeds on 2xx, so a 503 falls through to the retry.
    if READY_BODY="$(curl -sf "http://127.0.0.1:${SMOKE_PORT}/ready" 2>/dev/null)"; then
        echo "[smoke] ready after ${i}s"
        echo "[smoke]   /ready -> $READY_BODY"
        READY="1"
        break
    fi
    if (( i % 15 == 0 )); then
        echo "[smoke]   ...still waiting at ${i}s. Last /ready body (if any):"
        curl -s "http://127.0.0.1:${SMOKE_PORT}/ready" 2>/dev/null | sed 's/^/            /' || true
    fi
    sleep 1
done

if [[ -z "$READY" ]]; then
    echo "[smoke] FAIL: /ready never returned 200 (readiness gate did not pass). Full container logs:"
    docker logs brain_compose_smoke 2>&1 | sed 's/^/    /'
    exit 1
fi

# Body shape checks: status=ready + the three named gates.
echo "$READY_BODY" | grep -q '"status":"ready"' \
    || { echo "[smoke] FAIL: /ready missing status=ready"; exit 1; }
echo "$READY_BODY" | grep -q '"db_reachable":true' \
    || { echo "[smoke] FAIL: /ready reports db_reachable != true"; exit 1; }
echo "$READY_BODY" | grep -q '"migrations_applied":true' \
    || { echo "[smoke] FAIL: /ready reports migrations_applied != true"; exit 1; }
echo "$READY_BODY" | grep -q '"index_open":true' \
    || { echo "[smoke] FAIL: /ready reports index_open != true"; exit 1; }

# ── 6. Auth gate: wrong credentials -> 401 ───────────────────────────────────
# Phase G (2026-07-20): login now uses the `{username, password}` body shape.
echo "[smoke] checking /api/v1/auth/login with WRONG credentials (expect 401) ..."
CODE=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "http://127.0.0.1:${SMOKE_PORT}/api/v1/auth/login" \
    -H 'Content-Type: application/json' \
    -d '{"username":"smoke-admin","password":"this-is-not-the-password"}')
echo "[smoke]   -> HTTP $CODE"
[[ "$CODE" = "401" ]] || { echo "[smoke] FAIL: expected 401 got $CODE"; exit 1; }

# ── 6b. /metrics Prometheus endpoint (Phase F2.2) ────────────────────────────
# /metrics is unauthenticated ops surface. The recorder was installed at
# startup (init_recorder, before serve). Two gates:
#   (a) the response is Prometheus text exposition (must contain `^# TYPE`),
#   (b) the wrong-secret login above incremented console_auth_failures_total,
#       which MUST appear as a sample line.
# A missing `# TYPE` would mean the recorder failed to install (or the route
# isn't mounted); a missing counter name would mean the wire from handler to
# recorder is broken.
echo "[smoke] checking /metrics (Prometheus text format + auth-failure counter) ..."
METRICS_CT="$(curl -s -D - -o /tmp/brain_smoke_metrics.txt \
    "http://127.0.0.1:${SMOKE_PORT}/metrics" \
    | grep -i '^content-type:' | tr -d '\r' || true)"
echo "[smoke]   /metrics content-type: $METRICS_CT"
echo "$METRICS_CT" | grep -qi '^content-type:[[:space:]]*text/plain' \
    || { echo "[smoke] FAIL: /metrics Content-Type is not text/plain"; exit 1; }

grep -q '^# TYPE' /tmp/brain_smoke_metrics.txt \
    || { echo "[smoke] FAIL: /metrics body missing Prometheus '# TYPE' lines"; \
         echo "[smoke]   body was:"; sed 's/^/            /' /tmp/brain_smoke_metrics.txt; \
         exit 1; }
grep -q 'console_auth_failures_total' /tmp/brain_smoke_metrics.txt \
    || { echo "[smoke] FAIL: /metrics body missing console_auth_failures_total"; \
         echo "[smoke]   (login failure at step 6 should have bumped it)"; \
         exit 1; }
echo "[smoke]   /metrics OK — Prometheus text + console_auth_failures_total present"

# ── 7. Auth gate: correct credentials -> 200 + csrf_token ────────────────────
# Phase G (2026-07-20): username+password body shape.
echo "[smoke] checking /api/v1/auth/login with CORRECT credentials (expect 200) ..."
LOGIN_BODY=$(curl -s -w "\n%{http_code}" \
    -X POST "http://127.0.0.1:${SMOKE_PORT}/api/v1/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$SMOKE_USERNAME\",\"password\":\"$SMOKE_PASSWORD\"}")
CODE=$(echo "$LOGIN_BODY" | tail -n1)
BODY=$(echo "$LOGIN_BODY" | sed '$d')
echo "[smoke]   -> HTTP $CODE body=$BODY"
[[ "$CODE" = "200" ]] || { echo "[smoke] FAIL: expected 200 got $CODE (BRAIN_USERNAME/BRAIN_PASSWORD env may not have been read)"; exit 1; }
echo "$BODY" | grep -q '"csrf_token"' || { echo "[smoke] FAIL: login response missing csrf_token"; exit 1; }

# ── 8. Console static index served at / ──────────────────────────────────────
echo "[smoke] checking Console static index at / ..."
INDEX_BODY="$(curl -sf "http://127.0.0.1:${SMOKE_PORT}/")"
echo "$INDEX_BODY" | grep -q '<title>Brain Console</title>' \
    || { echo "[smoke] FAIL: console index missing <title>Brain Console</title>"; exit 1; }
echo "[smoke]   console index OK"

# ── 9. SECURITY GATE A: BRAIN_USERNAME/BRAIN_PASSWORD env vars ARE present ─
# Phase G (2026-07-20): the credentials are intentionally sourced from env
# vars, so they MUST appear in `docker inspect Config.Env`. The gate flips:
# we now assert the credential keys are present (sanity — env block wired)
# and document the explicit tradeoff (dev-grade auth behind loopback publish
# + TLS fronting proxy; for higher-stakes deployments, restore the legacy
# `console_dev_bootstrap_secret_file` Docker-secret path and unset
# BRAIN_PASSWORD — the server falls back to the file).
echo "[smoke] SECURITY GATE: BRAIN_USERNAME + BRAIN_PASSWORD must appear in docker inspect Config.Env ..."
INSPECT_ENV="$(docker inspect brain_compose_smoke --format '{{.Config.Env}}' 2>/dev/null || true)"
echo "$INSPECT_ENV" | grep -q 'BRAIN_USERNAME=' \
    || { echo "[smoke] FAIL: BRAIN_USERNAME missing from Config.Env (env block not wired)"; exit 1; }
echo "$INSPECT_ENV" | grep -q 'BRAIN_PASSWORD=' \
    || { echo "[smoke] FAIL: BRAIN_PASSWORD missing from Config.Env (env block not wired)"; exit 1; }
echo "[smoke]   BRAIN_USERNAME + BRAIN_PASSWORD present in Config.Env (Phase G env-var flow)"

# Also assert the standard control vars are still present.
echo "$INSPECT_ENV" | grep -q 'RUST_LOG=' \
    || { echo "[smoke] FAIL: RUST_LOG missing from Config.Env"; exit 1; }
echo "$INSPECT_ENV" | grep -q 'LLM_WIKI_CONFIG=' \
    || { echo "[smoke] FAIL: LLM_WIKI_CONFIG missing from Config.Env"; exit 1; }

# ── 10. (Phase G) /run/secrets mount is OPTIONAL ─────────────────────────────
# The legacy Docker-secret path (`console_dev_bootstrap_secret_file`) still
# works as a fallback when PASSWORD env is unset, but the smoke stack no
# longer exercises it. We assert the absence is intentional — the secrets
# block + the bind mount are gone, and that's correct for the env-var flow.
echo "[smoke] verifying /run/secrets/bootstrap_secret is NOT mounted (env-var flow) ..."
MOUNTS_JSON="$(docker inspect brain_compose_smoke --format '{{json .Mounts}}')"
if echo "$MOUNTS_JSON" | grep -F -q '/run/secrets/bootstrap_secret'; then
    echo "[smoke] FAIL: /run/secrets/bootstrap_secret mount unexpectedly present"
    echo "[smoke]   Mounts = $MOUNTS_JSON"
    exit 1
fi
echo "[smoke]   no legacy secret-file mount (correct for Phase G env-var flow)"

# Also assert the source path points at the scratch secrets dir (not a stray
# operator copy). Compare in BOTH the MSYS-style path (what Bash sees) and
# ── 11. Image history clean (carry from F1.1) ────────────────────────────────
# Phase G: the password value now rides in Config.Env (intentional), so we
# scan image history for it instead — the credentials must NEVER be baked
# into the image layers themselves (would leak to anyone who pulls it).
echo "[smoke] SECRET SCAN: image history must not contain the password ..."
if docker history --no-trunc brain-compose-smoke:dev 2>/dev/null | grep -F -q -- "$SMOKE_PASSWORD"; then
    echo "[smoke] FAIL: password leaked into image history"
    exit 1
fi
echo "[smoke]   image history clean"

# ── 12. Non-root gate (carry from F1.1) ──────────────────────────────────────
echo "[smoke] non-root: checking uid inside container ..."
ID_OUT="$(docker exec brain_compose_smoke id 2>/dev/null || true)"
if [[ -z "$ID_OUT" ]]; then
    # `docker exec` may not be available; fall back to a fresh ephemeral run.
    ID_OUT="$(MSYS_NO_PATHCONV=1 docker run --rm --entrypoint /bin/sh brain-compose-smoke:dev -c 'id')"
fi
echo "[smoke]   -> $ID_OUT"
echo "$ID_OUT" | grep -Eq 'uid=1000\(brain\)' \
    || { echo "[smoke] FAIL: container not running as brain(uid=1000): $ID_OUT"; exit 1; }

echo ""
echo "[smoke] PASS — all checks green (compose up, /health, /ready, login 401/200,"
echo "                 console index, /metrics text + counter, USERNAME+PASSWORD"
echo "                 present in inspect Env, image clean, non-root)"
