#!/usr/bin/env bash
# scripts/docker_buildx_multiarch.sh
#
# Phase F1.3 — multi-arch image builder (amd64 ONLY; arm64 DEFERRED).
#
# Closes the §13 Task 6.1 DoD #2 ("images build+test linux/amd64 + arm64") at
# the amd64 level. arm64 is DEFERRED per user direction (2026-07-18): the
# one-time QEMU binfmt setup (`docker run --rm --privileged
# multiarch/qemu-user-static --reset -p yes`) was skipped, and no Oracle ARM
# host or CI runner is wired yet. The arm64 invocation is preserved below as
# a commented block so re-enabling it is a one-line uncomment when an ARM
# target exists.
#
# Usage:
#   bash scripts/docker_buildx_multiarch.sh                 # -> brain:multiarch
#   bash scripts/docker_buildx_multiarch.sh my-tag:latest   # -> my-tag:latest
#
# Pre-reqs:
#   * Docker Engine 24+ with BuildKit (default on modern Docker).
#   * A buildx builder. If the active builder is usable (the default `docker`
#    driver is, for amd64 + `--load`), this script reuses it; only if no
#    usable builder exists does it create `brain-buildx` (docker-container
#    driver) and switch to it.
#
# What this script does:
#   1. Ensures a usable buildx builder is active (reuses default, or creates
#      `brain-buildx` as a fallback).
#   2. Builds + tags the amd64 image with `--load` so it lands in the local
#      image store (runnable with `docker run`).
#   3. Prints the arm64 DEFERRED banner; the arm64 block is commented out.
#
# Why amd64-only for now:
#   * The compose smoke (scripts/docker_compose_smoke.sh) + the build smoke
#     (scripts/docker_build_smoke.sh) both run amd64; amd64 is the gate that
#     closes Phase F1.
#   * arm64 needs QEMU emulation on x86 hosts (slow) OR a native ARM runner.
#     Neither is available in-env today. The deferral is documented in:
#       - this script (commented arm64 block below)
#       - docs/plans/phase-F-production.md (Task F1 DoD #1/#2 amended)
#       - docs/guides/deploy-docker.md (What's next section)

set -euo pipefail

TAG="${1:-brain:multiarch}"

# ── Resolve repo root (so the script works from any CWD) ─────────────────────
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# ── 1. Ensure a buildx builder exists ─────────────────────────────────────────
# For amd64-only with `--load`, the default builder (driver `docker`) is the
# fastest path: it builds natively and loads straight into the local image
# store with no container-driver overhead. The `docker-container` driver is
# only needed for true multi-platform manifest builds (the deferred arm64
# case). So: prefer the default/active builder if it is usable; only spin up a
# `docker-container` builder named `brain-buildx` as a fallback (e.g. on a
# host where the default driver is unavailable).
#
# `docker buildx inspect --bootstrap` on the active builder tells us if it is
# usable. If the active builder errors, create `brain-buildx` and switch to it.
CURRENT_BUILDER="$(docker buildx inspect --bootstrap 2>/dev/null | awk '/^Name:/{print $2; exit}')"
if [[ -z "$CURRENT_BUILDER" ]]; then
    BUILDER_NAME="brain-buildx"
    echo "[buildx] no usable active builder; creating '$BUILDER_NAME' (docker-container driver) ..."
    docker buildx create --name "$BUILDER_NAME" --driver docker-container --use >/dev/null
    docker buildx inspect --bootstrap "$BUILDER_NAME" >/dev/null
    echo "[buildx] using builder '$BUILDER_NAME'"
else
    echo "[buildx] using existing active builder '$CURRENT_BUILDER'"
fi

# ── 2. amd64 build + tag + load into local image store ────────────────────────
echo "[buildx] building linux/amd64 -> $TAG (this builds the Rust release, ~4-15 min) ..."
# BuildKit is required for the cache mounts in the Dockerfile. `--load` makes
# the image available to `docker run` / `docker compose` on this host.
DOCKER_BUILDKIT=1 docker buildx build \
    --platform linux/amd64 \
    --tag "$TAG" \
    --load \
    "$REPO_ROOT"

# ── 3. arm64 — DEFERRED (2026-07-18, user-approved) ──────────────────────────
# Re-enabling requires a one-time QEMU binfmt registration on the host:
#
#   docker run --rm --privileged multiarch/qemu-user-static --reset -p yes
#
# After that, uncomment the block below. On an x86 host the arm64 build runs
# under emulation (slow; useful for smoke only). For production, target a
# native ARM runner (Oracle ARM, GitHub Actions arm64 runner, etc.) — emulated
# builds are not release-quality.
#
# docker buildx build \
#     --platform linux/arm64 \
#     --tag "${TAG}-arm64" \
#     --load \
#     "$REPO_ROOT"

echo ""
echo "[buildx] amd64 image tagged $TAG"
echo "[buildx] arm64 DEFERRED — see comment above + docs/plans/phase-F-production.md (Task F1)"
