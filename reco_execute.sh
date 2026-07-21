#!/usr/bin/env bash
# Bulk execute approve/reject on 181 pending proposals.
# Uses curl + cookie jar (matches how Phase 1.5/1.6 tests worked).
set -uo pipefail

cd "$(dirname "$0")"

# Source .env WITHOUT echoing secrets
set -a
. ./.env
set +a

if [ -z "${BRAIN_USERNAME:-}" ] || [ -z "${BRAIN_PASSWORD:-}" ]; then
    echo "ERROR: BRAIN_USERNAME or BRAIN_PASSWORD missing from .env" >&2
    exit 1
fi

COOKIE_JAR=/tmp/brain-bulk-cookies.txt
rm -f "$COOKIE_JAR"

# Login
LOGIN_RESP=$(curl -s -c "$COOKIE_JAR" -X POST http://127.0.0.1:8080/api/v1/auth/login \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"$BRAIN_USERNAME\",\"password\":\"$BRAIN_PASSWORD\"}")

if ! echo "$LOGIN_RESP" | grep -q csrf_token; then
    echo "Login failed: $LOGIN_RESP" >&2
    exit 1
fi

CSRF=$(echo "$LOGIN_RESP" | python -c "import json, sys; print(json.load(sys.stdin)['csrf_token'])")
echo "Login OK, CSRF=${CSRF:0:8}..."

# Verify session cookie was set
if ! grep -q brain_console_session "$COOKIE_JAR"; then
    echo "ERROR: session cookie not set" >&2
    cat "$COOKIE_JAR" >&2
    exit 1
fi

# Quick auth test
TEST_RESP=$(curl -s -b "$COOKIE_JAR" http://127.0.0.1:8080/api/v1/inbox?limit=1)
if echo "$TEST_RESP" | grep -q unauthorized; then
    echo "Auth test FAILED on /inbox" >&2
    exit 1
fi
echo "Auth test OK"

# Read recommendations
PYTHON_DATA=$(python -c "
import json
recs = json.load(open('inbox-recommendations.json', encoding='utf-8'))
for r in recs:
    if r['action'] == 'REVIEW':
        r['action'] = 'REJECT'  # user-approved
    print(f\"{r['action']}|{r['proposal_id']}\")
")

# Execute
OK=0
ERR=0
ERR_SAMPLES=""
TOTAL=$(echo "$PYTHON_DATA" | wc -l)
I=0

echo "$PYTHON_DATA" | while IFS='|' read -r action pid; do
    I=$((I+1))
    RESP=$(curl -s -b "$COOKIE_JAR" -X POST "http://127.0.0.1:8080/api/v1/inbox/$pid/${action,,}" \
        -H "Content-Type: application/json" \
        -H "X-CSRF-Token: $CSRF" \
        -d '{}' \
        -w '\n%{http_code}' \
        --max-time 15)
    CODE=$(echo "$RESP" | tail -1)
    BODY=$(echo "$RESP" | head -n -1)
    if [ "$CODE" = "200" ]; then
        OK=$((OK+1))
    else
        ERR=$((ERR+1))
        if [ $ERR -le 5 ]; then
            ERR_SAMPLES="$ERR_SAMPLES\n  $action $pid: HTTP $CODE — ${BODY:0:120}"
        fi
    fi
    if [ $((I % 20)) -eq 0 ] || [ $I -eq $TOTAL ]; then
        echo "  [$I/$TOTAL] last=$action OK=$OK ERR=$ERR"
    fi
done

echo ""
echo "=== RESULTS ==="
echo "  OK=$OK ERR=$ERR"
if [ -n "$ERR_SAMPLES" ]; then
    echo -e "Errors (first 5):$ERR_SAMPLES"
fi

# Verify inbox count
FINAL=$(curl -s -b "$COOKIE_JAR" "http://127.0.0.1:8080/api/v1/inbox" | python -c "import json, sys; d=json.load(sys.stdin); print(len(d))" 2>/dev/null || echo "?")
echo ""
echo "=== Inbox pending after execute: $FINAL ==="
