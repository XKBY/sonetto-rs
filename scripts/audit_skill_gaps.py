#!/usr/bin/env python3
"""Skill ID parity gap inventory: LIVE fixtures vs OURS replays.

Walks every `tests/battle{N}/begin_round_{R}.json` (LIVE) and the
matching `tests/runs/battle{N}/my_begin_round_{R}.json` (OURS),
recursively counting `actId` emissions through nested
`actEffect[].fightStep` children. Sorts skills by absolute
parity gap and annotates each with the in-game description from
`data/excel2json/language_en.json`.

Usage:
  python scripts/audit_skill_gaps.py                  # full report
  python scripts/audit_skill_gaps.py --top 30          # top N by |delta|
  python scripts/audit_skill_gaps.py --battle battle2  # filter battle
  python scripts/audit_skill_gaps.py --over-fires      # only OURS > LIVE
  python scripts/audit_skill_gaps.py --under-fires     # only OURS < LIVE
"""

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
TESTS = REPO / "tests"


def parse_lang(path):
    """Parse a language_*.json file (handles trailing padding bytes)."""
    text = path.read_text(encoding='utf-8')
    decoder = json.JSONDecoder()
    obj, _ = decoder.raw_decode(text.lstrip())
    return dict(obj[1])


def load_skill_descriptions():
    """Build skill_id -> (kind, name, desc_text) for every known skill.

    Resolves directly from `skill_effect.json::desc` for hero, psychube,
    and battle-rule signatures; chains through `skill.json::skillEffect`
    for boss skills (signature `40120xxx`).
    """
    client = parse_lang(DATA / "language_en.json")

    with (DATA / "skill_effect.json").open(encoding='utf-8') as f:
        effects = {r['id']: r for r in json.load(f)[1] if isinstance(r, dict)}
    with (DATA / "skill.json").open(encoding='utf-8') as f:
        skill_meta = {r['id']: r for r in json.load(f)[1] if isinstance(r, dict)}

    out = {}

    # Direct: skill_effect.json carries desc for hero/psychube/battle-rule
    for sid, row in effects.items():
        desc_key = row.get('desc', '')
        text = client.get(desc_key, '') if desc_key else ''
        if text:
            out[sid] = ('skill_effect', '', text)

    # Boss: skill.json::skillEffect -> skill_effect.json::desc
    for sid, row in skill_meta.items():
        if sid in out:
            continue
        name_key = row.get('name', '')
        name = client.get(name_key, '')
        eff_id = row.get('skillEffect')
        if eff_id and eff_id in effects:
            desc_key = effects[eff_id].get('desc', '')
            text = client.get(desc_key, '') if desc_key else ''
            if text or name:
                out[sid] = ('boss', name, text)

    return out


def signature_owner(sid):
    """Best-effort hero/psychube/rule classification by ID prefix."""
    s = str(sid)
    if s.startswith('5300'):
        return 'rule'
    if s.startswith('40'):
        return 'boss'
    if 30000000 <= sid < 32000000:
        # Hero signature: take first 4 digits
        return f'hero {s[:4]}'
    if 30000 <= sid < 40000 or 40000 <= sid < 50000 or 4348 <= sid // 1000 <= 4360:
        return 'psychube'
    if sid < 100000:
        return 'effect-marker'
    return '?'


def walk_steps(steps):
    """Recursively yield every FightStep, including nested children."""
    if not steps:
        return
    for s in steps:
        yield s
        for eff in s.get('actEffect', []) or []:
            child = eff.get('fightStep')
            if child:
                yield from walk_steps([child])


def count_act_ids(round_json):
    """Count actId occurrences in a round's full nested fightStep tree."""
    counts = defaultdict(int)
    for step in walk_steps(round_json.get('fightStep', [])):
        aid = step.get('actId')
        if aid and aid != 0:
            counts[aid] += 1
    return counts


def collect_gaps(battles):
    """Yield (battle, round, actId, live_count, ours_count) for every
    actId that appears with different counts in LIVE vs OURS."""
    for battle in battles:
        live_dir = TESTS / battle
        ours_dir = TESTS / 'runs' / battle
        if not live_dir.is_dir() or not ours_dir.is_dir():
            continue
        for live_file in sorted(live_dir.glob('begin_round_*.json')):
            try:
                round_n = int(live_file.stem.split('_')[-1])
            except ValueError:
                continue
            ours_file = ours_dir / f'my_begin_round_{round_n}.json'
            if not ours_file.exists():
                continue
            try:
                with live_file.open(encoding='utf-8') as f:
                    live = json.load(f).get('round', {})
                with ours_file.open(encoding='utf-8') as f:
                    ours = json.load(f).get('round', {})
            except Exception as e:
                print(f'WARN: failed to load {live_file} or {ours_file}: {e}',
                      file=sys.stderr)
                continue
            live_counts = count_act_ids(live)
            ours_counts = count_act_ids(ours)
            for aid in set(live_counts) | set(ours_counts):
                lc = live_counts.get(aid, 0)
                oc = ours_counts.get(aid, 0)
                if lc != oc:
                    yield (battle, round_n, aid, lc, oc)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--top', type=int, default=0,
                        help='show only top N rows by |Δ| (default: all)')
    parser.add_argument('--battle', default=None,
                        help='filter to one battle (e.g. battle2)')
    parser.add_argument('--over-fires', action='store_true',
                        help='only show OURS > LIVE')
    parser.add_argument('--under-fires', action='store_true',
                        help='only show OURS < LIVE')
    parser.add_argument('--per-battle', action='store_true',
                        help='split aggregation per (battle, round) instead of global')
    args = parser.parse_args()

    descriptions = load_skill_descriptions()
    battles = [args.battle] if args.battle else ['battle1', 'battle2', 'battle3']

    raw_rows = list(collect_gaps(battles))

    # Aggregate by skill_id across all (battle, round)
    by_aid = defaultdict(lambda: {'live': 0, 'ours': 0, 'occurrences': []})
    for battle, round_n, aid, lc, oc in raw_rows:
        agg = by_aid[aid]
        agg['live'] += lc
        agg['ours'] += oc
        agg['occurrences'].append((battle, round_n, lc, oc, oc - lc))

    if args.over_fires:
        by_aid = {k: v for k, v in by_aid.items() if v['ours'] > v['live']}
    elif args.under_fires:
        by_aid = {k: v for k, v in by_aid.items() if v['ours'] < v['live']}

    aggregated = sorted(
        by_aid.items(),
        key=lambda kv: (-abs(kv[1]['ours'] - kv[1]['live']),
                        -(kv[1]['live'] + kv[1]['ours']))
    )

    if args.top > 0:
        aggregated = aggregated[:args.top]

    print('=' * 110)
    print(f'{"actId":>10}  {"owner":<14}  {"LIVE":>4}  {"OURS":>4}  {"Δ":>4}  desc')
    print('-' * 110)
    for aid, info in aggregated:
        delta = info['ours'] - info['live']
        kind, name, text = descriptions.get(aid, ('?', '', ''))
        owner = signature_owner(aid)
        label = f'[{name}] ' if name else ''
        desc_short = (label + text).replace('\n', ' / ')[:75]
        print(f'{aid:>10}  {owner:<14}  {info["live"]:>4}  {info["ours"]:>4}  '
              f'{delta:>+4}  {desc_short}')

    # Summary
    total_gap = sum(abs(info['ours'] - info['live']) for info in by_aid.values())
    n_unique = len(by_aid)
    over = sum(1 for v in by_aid.values() if v['ours'] > v['live'])
    under = sum(1 for v in by_aid.values() if v['ours'] < v['live'])

    print()
    print(f'Unique skill IDs with parity gaps: {n_unique}')
    print(f'  over-fires (OURS > LIVE):  {over}')
    print(f'  under-fires (OURS < LIVE): {under}')
    print(f'Total absolute gap (Σ|Δ| across all skills): {total_gap}')

    # Per-battle/round breakdown for top 5 if --per-battle
    if args.per_battle:
        print()
        print('=== Per-battle/round breakdown (top 5 by |Δ|) ===')
        for aid, info in aggregated[:5]:
            print(f'\n  actId={aid}:')
            for battle, r, lc, oc, d in sorted(info['occurrences']):
                print(f'    {battle} r{r}: LIVE={lc} OURS={oc} Δ={d:+d}')


if __name__ == '__main__':
    main()
