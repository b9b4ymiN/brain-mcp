#!/usr/bin/env bash
# Phase E1 DoD #1: no mock/TODO path in production build.
#
# Builds the Console SPA (web/console) and greps the built bundle for the
# forbidden markers. Fails the gate (exit non-zero) if any match is found.
#
# Forbidden patterns (case-insensitive):
#   TODO | FIXME | MOCK_DATA | mock_
#
# This is the POSIX sh / bash equivalent of console_grep_gate.ps1 for Linux
# CI. See that file for the full rationale (Phase E1 Gate §13 Task 5.1):
# grepping the BUILT bundle (not the TS source) makes the gate airtight.
#
# Usage:
#   ./scripts/console_grep_gate.sh            # build then grep
#   ./scripts/console_grep_gate.sh --skip-build   # grep existing dist/
# Exit codes:
#   0 = clean
#   1 = found matches (or build failed)

set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONSOLE_DIR="$SCRIPT_ROOT/../web/console"
DIST_ASSETS="$CONSOLE_DIR/dist/assets"

SKIP_BUILD=0
if [[ "${1:-}" == "--skip-build" ]]; then
  SKIP_BUILD=1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "[console_grep_gate] building console in $CONSOLE_DIR"
  ( cd "$CONSOLE_DIR" && npm run build )
fi

if [[ ! -d "$DIST_ASSETS" ]]; then
  echo "[console_grep_gate] FAIL: $DIST_ASSETS does not exist — run without --skip-build first." >&2
  exit 1
fi

# Collect matches across every .js file under dist/assets. grep -rni gives
# file:line:match; we count lines. `-E` for the alternation, `-i` for
# case-insensitivity.
mapfile -t MATCHES < <(grep -rniE 'TODO|FIXME|MOCK_DATA|mock_' "$DIST_ASSETS"/*.js || true)

if [[ ${#MATCHES[@]} -gt 0 ]]; then
  echo "" >&2
  echo "[console_grep_gate] FAIL: ${#MATCHES[@]} forbidden marker(s) in built bundle:" >&2
  printf '  %s\n' "${MATCHES[@]}" >&2
  echo "Phase E1 DoD #1 NOT satisfied — remove the markers and rebuild." >&2
  exit 1
fi

JS_COUNT=$(find "$DIST_ASSETS" -maxdepth 1 -name '*.js' -type f | wc -l | tr -d ' ')
echo "[console_grep_gate] PASS: no TODO/FIXME/MOCK_DATA/mock_ markers in $JS_COUNT built .js file(s)."
exit 0
