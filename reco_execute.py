#!/usr/bin/env python3
"""Bulk execute approve/reject with proper accounting."""
import json
import urllib.request
import urllib.error
import http.cookiejar
import subprocess
from pathlib import Path

ROOT = Path(__file__).parent

# Use curl-style flow but track results in a file (subshell-safe)
# Step 1: login + get CSRF + cookie jar via curl (proven to work)
env = {}
with open(ROOT / '.env', encoding='utf-8') as f:
    for line in f:
        line = line.strip()
        if line and not line.startswith('#') and '=' in line:
            k, v = line.split('=', 1)
            env[k.strip()] = v.strip()

username = env.get('BRAIN_USERNAME', '')
password = env.get('BRAIN_PASSWORD', '')
COOKIE_JAR = '/tmp/brain-bulk-cookies.txt'

# Login via subprocess (matches working curl pattern)
login = subprocess.run(
    ['curl', '-s', '-c', COOKIE_JAR, '-X', 'POST',
     'http://127.0.0.1:8080/api/v1/auth/login',
     '-H', 'Content-Type: application/json',
     '-d', json.dumps({'username': username, 'password': password})],
    capture_output=True, text=True, check=True,
)
csrf = json.loads(login.stdout)['csrf_token']
print(f'Login OK, CSRF={csrf[:8]}...')

# Step 2: load recs, force REVIEW → REJECT
recs = json.load(open(ROOT / 'inbox-recommendations.json', encoding='utf-8'))
for r in recs:
    if r['action'] == 'REVIEW':
        r['action'] = 'REJECT'

# Step 3: check current pending list (some may already be done from earlier script)
pending_resp = subprocess.run(
    ['curl', '-s', '-b', COOKIE_JAR, 'http://127.0.0.1:8080/api/v1/inbox'],
    capture_output=True, text=True, check=True,
)
pending = json.loads(pending_resp.stdout)
pending_ids = {str(p['proposal_id']) for p in pending}
print(f'Currently pending: {len(pending_ids)}')

# Step 4: execute (only on still-pending)
ok = 0
err = 0
errors = []
todo = [r for r in recs if r['proposal_id'] in pending_ids]
print(f'To execute: {len(todo)} (skipping {len(recs) - len(todo)} already done)')

for i, r in enumerate(todo, 1):
    pid = r['proposal_id']
    action = r['action'].lower()
    try:
        result = subprocess.run(
            ['curl', '-s', '-b', COOKIE_JAR, '-X', 'POST',
             f'http://127.0.0.1:8080/api/v1/inbox/{pid}/{action}',
             '-H', 'Content-Type: application/json',
             '-H', f'X-CSRF-Token: {csrf}',
             '-d', '{}',
             '-w', '\n%{http_code}',
             '--max-time', '15'],
            capture_output=True, text=True, check=True,
        )
        lines = result.stdout.rsplit('\n', 1)
        code = lines[-1] if len(lines) > 1 else '???'
        body = lines[0] if len(lines) > 1 else result.stdout
        if code == '200':
            ok += 1
        else:
            err += 1
            if len(errors) < 10:
                errors.append((pid, action, code, body[:200]))
    except Exception as e:
        err += 1
        if len(errors) < 10:
            errors.append((pid, action, 'EXC', str(e)[:200]))

    if i % 20 == 0 or i == len(todo):
        print(f'  [{i}/{len(todo)}] OK={ok} ERR={err}')

print()
print('=== RESULTS ===')
print(f'  OK={ok} ERR={err}')
if errors:
    print('Errors:')
    for pid, action, code, msg in errors:
        print(f'  {action} {pid}: {code} — {msg[:150]}')

# Step 5: verify
final = subprocess.run(
    ['curl', '-s', '-b', COOKIE_JAR, 'http://127.0.0.1:8080/api/v1/inbox'],
    capture_output=True, text=True, check=True,
)
final_pending = json.loads(final.stdout)
print()
print(f'=== Inbox pending AFTER execute: {len(final_pending)} ===')
if final_pending:
    print('Remaining:')
    for p in final_pending[:5]:
        print(f"  {p['subject']!r:30} {p['predicate']!r:30} {p['proposal_id']}")
