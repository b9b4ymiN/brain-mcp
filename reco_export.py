#!/usr/bin/env python3
"""Export recommendations as human-readable Markdown for user review."""
import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).parent
recs = json.load(open(ROOT / 'inbox-recommendations.json', encoding='utf-8'))

# Sort: APPROVE first, then REVIEW, then REJECT
action_order = {'APPROVE': 0, 'REVIEW': 1, 'REJECT': 2}
recs.sort(key=lambda r: (action_order.get(r['action'], 99), r['subject'], r['predicate']))

# Group by action × subject
out = []
out.append('# Inbox Recommendations — 181 Pending Proposals')
out.append('')
out.append(f'> Generated: 2026-07-21 (deterministic heuristic, no LLM)')
out.append(f'> Branch: vnext/phase-0')
out.append(f'> Source: live SemanticStore (181 pending)')
out.append('')
out.append('## Summary')
out.append('')
counts = defaultdict(int)
for r in recs:
    counts[r['action']] += 1
out.append('| Action | Count | % |')
out.append('|--------|-------|---|')
for a in ('APPROVE', 'REVIEW', 'REJECT'):
    c = counts[a]
    pct = 100 * c / len(recs)
    icon = {'APPROVE': '🟢', 'REVIEW': '🟠', 'REJECT': '🔴'}[a]
    out.append(f'| {icon} {a} | {c} | {pct:.1f}% |')
out.append(f'| **TOTAL** | **{len(recs)}** | **100%** |')
out.append('')
out.append('## Heuristic layers (priority order)')
out.append('')
out.append('1. **Phase 1.5 reject** — subject fails validator (metric_head, slug, etc.) → REJECT')
out.append('2. **Soft flag** — MultiEntity + boolean value → REJECT (narrative, not metric)')
out.append('3. **Weak predicate** — `predicate="is"` or len<3 → REJECT')
out.append('4. **Metadata subject** — "Research report", "Peer set", "New H-shares", etc. → REJECT')
out.append('5. **Metric compound** — "CATL market share", "Net cash/share" → REJECT (entity+metric fused)')
out.append('6. **Entity + qualifier** — "CATL (Q1 2026)" + good predicate → APPROVE')
out.append('7. **Industry entity** — "TAM ESS", "Chinese battery industry" → APPROVE')
out.append('8. **Simple acronym/ticker** — CATL, BYD → APPROVE')
out.append('9. **TitleCase + good predicate** — "Hungary overseas plant" + "investment cost" → APPROVE')
out.append('')

for action in ('APPROVE', 'REVIEW', 'REJECT'):
    items = [r for r in recs if r['action'] == action]
    if not items:
        continue
    icon = {'APPROVE': '🟢', 'REVIEW': '🟠', 'REJECT': '🔴'}[action]
    out.append(f'## {icon} {action} ({len(items)})')
    out.append('')

    # Group by subject for readability
    by_subj = defaultdict(list)
    for r in items:
        by_subj[r['subject']].append(r)

    for subj in sorted(by_subj.keys()):
        subj_items = by_subj[subj]
        out.append(f'### {subj!r} ({len(subj_items)} proposal{"s" if len(subj_items) > 1 else ""})')
        out.append('')
        for r in subj_items:
            v_str = str(r['value'])
            if len(v_str) > 60:
                v_str = v_str[:57] + '...'
            pid = r['proposal_id']
            out.append(f'- **`{r["predicate"]}`** = `{v_str}`')
            out.append(f'  - `proposal_id`: `{pid}`')
            out.append(f'  - `domain`: `{r["domain"]}`')
            out.append(f'  - `rationale`: {r["rationale"]}')
        out.append('')

out.append('---')
out.append('')
out.append('## How to execute (after approval)')
out.append('')
out.append('```bash')
out.append('# For each APPROVE proposal:')
out.append('curl -b cookies.txt -X POST http://127.0.0.1:8080/api/v1/inbox/{proposal_id}/approve')
out.append('')
out.append('# For each REJECT proposal:')
out.append('curl -b cookies.txt -X POST http://127.0.0.1:8080/api/v1/inbox/{proposal_id}/reject')
out.append('```')
out.append('')
out.append('Or tell the agent "execute approved" and it will bulk-call the API.')
out.append('')

content = '\n'.join(out)
out_path = ROOT / 'inbox-recommendations.md'
with open(out_path, 'w', encoding='utf-8') as f:
    f.write(content)

print(f'Wrote {len(content)} chars to {out_path}')
print(f'Lines: {len(out)}')
print(f'APPROVE: {counts["APPROVE"]}, REVIEW: {counts["REVIEW"]}, REJECT: {counts["REJECT"]}')
