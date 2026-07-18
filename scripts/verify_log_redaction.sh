#!/usr/bin/env bash
# Task F2.1 — verify LogRedactor is wired into the tracing writers.
#
# Builds and runs the `verify_log_redaction` example, which emits a benign
# marker plus two fake secrets (Bearer + api_key=) through the same
# `RedactingMakeWriter` shape that `init_logging` installs. Greps the program's
# stderr for:
#   - the benign markers (must be present — proves the writer actually ran)
#   - the `[REDACTED]` marker (must be present — proves the redactor fired)
#   - the raw secret values (must NOT be present — proves the redaction worked)
#
# Exit codes:
#   0 = redaction confirmed (benign untouched + secret stripped)
#   1 = redaction failed (any of the above checks missed)
#
# Usage:
#   ./scripts/verify_log_redaction.sh
#
# No docker required. This is the offline/local equivalent of the container
# smoke described in Task F2.1 Part C. The container equivalent is:
#   docker compose up -d && \
#     docker compose logs wiki | grep -E 'sk-|Bearer [a-zA-Z0-9]' && exit 1 || exit 0

set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_ROOT/.." && pwd)"

cd "$REPO_ROOT"

echo "[verify_log_redaction] building example (this also type-checks the wire shape)"
OUTPUT="$(cargo run --quiet --example verify_log_redaction 2>&1)"

echo "$OUTPUT"
echo "---"

# 1. Benign markers must be present (proves the subscriber actually ran).
if ! echo "$OUTPUT" | grep -q "SMOKE-START benign marker: hello world"; then
  echo "[verify_log_redaction] FAIL: benign START marker missing — subscriber did not emit" >&2
  exit 1
fi
if ! echo "$OUTPUT" | grep -q "SMOKE-END benign marker: goodbye world"; then
  echo "[verify_log_redaction] FAIL: benign END marker missing — subscriber did not emit" >&2
  exit 1
fi

# 2. [REDACTED] marker must appear (proves the redactor fired).
if ! echo "$OUTPUT" | grep -q "\[REDACTED\]"; then
  echo "[verify_log_redaction] FAIL: no [REDACTED] marker in output — redactor not wired" >&2
  exit 1
fi

# 3. Raw secret values must NOT appear.
if echo "$OUTPUT" | grep -q "sk-proj-fakeSecretABCD1234567890XYZ"; then
  echo "[verify_log_redaction] FAIL: raw Bearer secret leaked into stderr" >&2
  exit 1
fi
if echo "$OUTPUT" | grep -q "sk-leaked-MULTI1234567890abcd"; then
  echo "[verify_log_redaction] FAIL: raw api_key= secret leaked into stderr" >&2
  exit 1
fi

# 4. The "Bearer " prefix should still be visible (kept for debuggability).
if ! echo "$OUTPUT" | grep -q "Bearer \[REDACTED\]"; then
  echo "[verify_log_redaction] FAIL: expected 'Bearer [REDACTED]' marker pattern" >&2
  exit 1
fi

echo "[verify_log_redaction] OK — benign untouched, secrets stripped"
