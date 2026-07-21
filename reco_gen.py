#!/usr/bin/env python3
"""Generate approve/reject recommendations for 181 pending inbox proposals."""
import json
import re
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).parent
data = json.load(open(ROOT / 'inbox.json', encoding='utf-8'))

rules = tomllib.load(open(ROOT / 'rules/subject_rules.toml', 'rb'))
deny = tomllib.load(open(ROOT / 'rules/subject_denylist.toml', 'rb'))
allow = tomllib.load(open(ROOT / 'rules/subject_allowlist.toml', 'rb'))
metric_heads = set(deny.get('metric_heads', {}).keys())
metric_single = set(deny.get('metric_single_words', {}).keys())
headings = set(deny.get('headings', {}).keys())
stopwords_l = set(s.lower() for s in deny.get('stopwords', {}).keys())
llm_bleed = set(deny.get('llm_bleed', {}).keys()) | set(s.lower() for s in deny.get('llm_bleed', {}).keys())
corp_suf_l = set(s.lower() for s in allow.get('corporate_suffixes', {}).keys())
verdicts_map = rules.get('verdicts', {})

RE_ACR = re.compile(r'^[A-Z]{2,8}$')
RE_TK = re.compile(r'^[A-Z]{1,6}(\.[A-Z]{1,4})?$')
RE_SL = re.compile(r'^[a-z0-9]+(-[a-z0-9]+){1,}$')
RE_DATE = re.compile(r'^\d{4}-\d{2}(-\d{2})?$|^Q[1-4]\s+\d{4}$')
RE_CUR = re.compile(r'^[¥$€£฿]\s*[\d,.]+')
RE_NUM = re.compile(r'^[\d,.]+\s*[BMK]?\b')


def classify(s):
    if RE_ACR.match(s):
        return 'Acronym'
    if RE_TK.match(s):
        return 'Ticker'
    if RE_DATE.match(s):
        return 'Date'
    if RE_SL.match(s):
        return 'Slug'
    if RE_CUR.match(s):
        return 'CurrencyLed'
    if RE_NUM.match(s):
        return 'NumberLed'
    if ',' in s:
        return 'MultiEntity'
    if "'s" in s:
        return 'Possessive'
    if all((not c.isalpha()) or c.islower() for c in s) and any(c.isalpha() for c in s):
        return 'LowercaseNoun'
    return 'TitleCase' if all(w[:1].isupper() for w in s.split() if w) else 'Plain'


def validate(s):
    shape = classify(s)
    if s in headings:
        return ('reject', 'heading', shape)
    if s.lower() in stopwords_l:
        return ('reject', 'stopword', shape)
    if s in llm_bleed:
        return ('reject', 'llm_bleed', shape)
    last = s.split()[-1].lower() if s.split() else ''
    if last in corp_suf_l:
        return ('accept', 'corp_suffix', shape)
    if shape in ('TitleCase', 'LowercaseNoun', 'Plain'):
        if s.lower() in metric_single:
            return ('reject', 'metric_single', shape)
        paren = s.split('(')[0].strip()
        lt = paren.split()[-1].lower() if paren.split() else ''
        if lt in metric_heads:
            return ('reject', 'metric_head', shape)
    v = verdicts_map.get(shape, 'accept_info')
    if v == 'reject_critical':
        return ('reject', f'shape={shape}', shape)
    if v == 'soft_flag':
        return ('soft_flag', f'shape={shape}', shape)
    return ('accept', f'shape={shape}', shape)


METADATA_SUBJECTS = {
    'Research report',
    'Q1 2026 performance',
    'Q2 2026 earnings report',
    'Peer set',
    'Analyst recommendations',
    'New H-shares',
    'CATL 2025 Annual Report',
    'CATL Q1 2026 results',
}

# Industry-level entities (TAM, market, sector) — accepted as "industry entities"
INDUSTRY_ENTITIES = {
    'TAM EV battery',
    'TAM ESS',
    'TAM Datacenter ESS',
    'Total TAM',
    'Chinese battery industry',
}

METRIC_COMPOUND_PATTERNS = [
    'market share',
    'net cash',
    'cash/share',
    'invalidation level',
    'cost of',
    'cost comparison',
    'equity risk premium',
    'target price',
    'overseas capex',
    'growth contribution',
    'growth source',
    'revenue share',
]


def is_entity_with_qualifier(s):
    """Has "(...)" or year qualifier with entity-like base."""
    if '(' in s and ')' in s:
        base = s.split('(')[0].strip()
        if base and len(base) <= 15 and base[0].isupper():
            return True
    if re.search(r'\b(202[0-9]|20[3-9][0-9])\b', s):
        base = re.sub(r'\s*\(?\s*\b(202[0-9]|20[3-9][0-9])\w*\)?\s*$', '', s).strip()
        if base and base[0].isupper():
            return True
    return False


def is_metric_compound(s):
    sl = s.lower()
    return any(p in sl for p in METRIC_COMPOUND_PATTERNS)


recommendations = []
for p in data:
    s, pred, val = p['subject'], p['predicate'], p['value']
    v, reason, shape = validate(s)

    action = None
    rationale = []

    if v == 'reject':
        action = 'REJECT'
        rationale.append(f'Phase 1.5 ({reason})')
    elif v == 'soft_flag':
        # MultiEntity + value='True' = narrative claim, not metric → reject
        if isinstance(val, str) and val.lower() in ('true', 'false'):
            action = 'REJECT'
            rationale.append('MultiEntity narrative (boolean value, would need split into N claims)')
        else:
            action = 'REVIEW'
            rationale.append(f'split needed ({reason})')
    elif pred.lower() in ('is',) or len(pred) < 3:
        action = 'REJECT'
        rationale.append(f"predicate '{pred}' vague")
    elif s in METADATA_SUBJECTS:
        action = 'REJECT'
        rationale.append('metadata subject (not entity claim)')
    elif s in INDUSTRY_ENTITIES:
        if len(pred) >= 3:
            action = 'APPROVE'
            rationale.append('industry-level entity + meaningful predicate')
        else:
            action = 'REVIEW'
            rationale.append('industry entity but weak predicate')
    elif is_metric_compound(s):
        action = 'REJECT'
        rationale.append('entity+metric fused subject')
    elif is_entity_with_qualifier(s):
        if len(pred) >= 5 and pred.lower() != 'is':
            action = 'APPROVE'
            rationale.append('entity+qualifier + good predicate')
        else:
            action = 'REVIEW'
            rationale.append('entity+qualifier but weak predicate')
    elif shape in ('Acronym', 'Ticker') and len(s) <= 8:
        action = 'APPROVE'
        rationale.append('clean entity acronym/ticker')
    elif shape in ('TitleCase', 'Plain') and len(pred) >= 5 and pred.lower() != 'is':
        first_word = s.split()[0] if s.split() else ''
        if first_word and first_word[0].isupper() and len(first_word) <= 15:
            action = 'APPROVE'
            rationale.append('capitalized subject + meaningful predicate')
        else:
            action = 'REVIEW'
            rationale.append(f'shape={shape}, ambiguous')
    else:
        action = 'REVIEW'
        rationale.append(f'unclassified shape={shape}')

    recommendations.append({
        'proposal_id': str(p['proposal_id']),
        'subject': s,
        'predicate': pred,
        'value': val if not isinstance(val, (dict, list)) else json.dumps(val),
        'domain': p['domain'],
        'submitted_at': p['submitted_at'],
        'action': action,
        'rationale': '; '.join(rationale),
    })

with open(ROOT / 'inbox-recommendations.json', 'w', encoding='utf-8') as f:
    json.dump(recommendations, f, indent=2, ensure_ascii=False, default=str)

actions = Counter(r['action'] for r in recommendations)
print('=== REFINED DISTRIBUTION ===')
for a, c in actions.most_common():
    print(f'  {a:8} {c:4}')
print(f'  TOTAL   {len(recommendations):4}')

print(f'\n=== Still REVIEW ({actions["REVIEW"]}) ===')
for r in recommendations:
    if r['action'] == 'REVIEW':
        v_str = str(r['value'])[:40]
        print(f'  {r["subject"]!r:38} {r["predicate"]!r:30} {v_str!r}')
