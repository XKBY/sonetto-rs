#!/usr/bin/env python3
"""Discovery: empirically map "None"-typed skill_behavior_condition IDs
to their actual firing scope based on the skills that use them.

Hypothesis (per memory `condition_id_semantics.md`): the engine
distinguishes condition IDs even when their `type` field is identical.
Multiple None-typed IDs (`c100`, `c101`, `c201`, `c203`, `c208`,
`c210`, `c591`, `c55`, etc.) likely each map to different fire
contexts (round-start, on-heal-received, inline-active-skill, etc.).

This script gathers the evidence per ID:
- Sample skills using it (limited to those in our fixtures)
- Their descriptions (where the rule's intent is)
- LIVE actId emission contexts (round-level vs nested-skill-level)

Output is a markdown table per ID. Read the descriptions, look at
where LIVE emits the actId, and classify the scope. Once we have a
stable ID → scope map, fold it into the parser.

Usage:
  python scripts/condition_scope_discovery.py                 # all None-typed IDs
  python scripts/condition_scope_discovery.py --top 12        # first 12 by usage
  python scripts/condition_scope_discovery.py --id 101        # focus on one
"""

import argparse
import json
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
TESTS = REPO / "tests"


def load_table(name):
    text = (DATA / f"{name}.json").read_text(encoding="utf-8")
    data = json.loads(text)
    return data[1] if isinstance(data, list) and len(data) >= 2 else data


def load_lang():
    text = (DATA / "language_en.json").read_text(encoding="utf-8")
    obj, _ = json.JSONDecoder().raw_decode(text.lstrip())
    return dict(obj[1])


def parse_first_id(raw):
    if not raw:
        return None
    head = raw.split("#")[0].split("&")[0].split("|")[0].strip()
    try:
        return int(head)
    except ValueError:
        return None


def collect_fixture_actids():
    seen = set()

    def walk(steps):
        for s in steps:
            aid = s.get("actId", 0)
            if aid:
                seen.add(aid)
            for eff in s.get("actEffect", []) or []:
                child = eff.get("fightStep")
                if child:
                    walk([child])

    for which in ["", "runs/"]:
        for battle in ["battle1", "battle2", "battle3"]:
            d = TESTS / battle if which == "" else TESTS / "runs" / battle
            if not d.is_dir():
                continue
            pat = "begin_round_*.json" if which == "" else "my_begin_round_*.json"
            for f in d.glob(pat):
                if "request" in f.name:
                    continue
                with f.open(encoding="utf-8") as fp:
                    walk(json.load(fp).get("round", {}).get("fightStep", []))
    return seen


def collect_emission_context(fixture_actids):
    """For each fixture actId, find LIVE's nesting context: depth + parent
    skill ids, top-level vs nested. This hints at the firing scope."""
    contexts = defaultdict(list)

    def walk(steps, parent_aid=0, depth=0):
        for s in steps:
            aid = s.get("actId", 0)
            if aid and aid in fixture_actids:
                contexts[aid].append({
                    "depth": depth,
                    "parent_aid": parent_aid,
                    "from_id": s.get("fromId", 0),
                    "to_id": s.get("toId", 0),
                })
            for eff in s.get("actEffect", []) or []:
                child = eff.get("fightStep")
                if child:
                    walk([child], aid if aid else parent_aid, depth + 1)

    for battle in ["battle1", "battle2", "battle3"]:
        d = TESTS / battle
        if not d.is_dir():
            continue
        for f in d.glob("begin_round_*.json"):
            if "request" in f.name:
                continue
            with f.open(encoding="utf-8") as fp:
                walk(json.load(fp).get("round", {}).get("fightStep", []))
    return contexts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--id", type=int, help="focus on a single condition id")
    parser.add_argument("--top", type=int, default=20,
                        help="show top N None-typed condition ids by fixture usage")
    args = parser.parse_args()

    print("Loading config + fixtures...")
    lang = load_lang()
    cond_types = {r["id"]: r.get("type", "") for r in load_table("skill_behavior_condition") if isinstance(r, dict)}
    none_typed_ids = {cid for cid, ct in cond_types.items() if ct == "None"}

    rows = load_table("skill_effect")
    fixture_actids = collect_fixture_actids()

    # For each None-typed condition id, find skills using it
    cid_to_skills = defaultdict(list)
    for r in rows:
        if not isinstance(r, dict):
            continue
        sid = r["id"]
        for i in range(1, 21):
            c_raw = r.get(f"condition{i}", "") or ""
            b_raw = r.get(f"behavior{i}", "") or ""
            if not c_raw:
                continue
            cid = parse_first_id(c_raw)
            if cid is None or cid not in none_typed_ids:
                continue
            in_fix = sid in fixture_actids
            cid_to_skills[cid].append({
                "skill_id": sid,
                "slot": i,
                "raw_cond": c_raw,
                "raw_beh": b_raw,
                "in_fixtures": in_fix,
                "desc_key": r.get("desc", ""),
            })

    contexts = collect_emission_context(fixture_actids)

    # Sort IDs by fixture usage descending
    cid_fixture_count = {cid: sum(1 for u in uses if u["in_fixtures"]) for cid, uses in cid_to_skills.items()}
    sorted_cids = sorted(cid_to_skills.keys(), key=lambda c: -cid_fixture_count.get(c, 0))

    if args.id:
        sorted_cids = [args.id] if args.id in cid_to_skills else []
    else:
        sorted_cids = sorted_cids[: args.top]

    print(f"# None-typed condition IDs (showing {len(sorted_cids)})\n")
    for cid in sorted_cids:
        uses = cid_to_skills[cid]
        fixture_uses = [u for u in uses if u["in_fixtures"]]
        print(f"## `c{cid}` — fixture-uses: {len(fixture_uses)}, total uses: {len(uses)}")
        if not fixture_uses:
            print("(no fixture skill uses this id)\n")
            continue
        # Show up to 5 fixture skills with descriptions
        seen = set()
        for u in fixture_uses[:8]:
            if u["skill_id"] in seen:
                continue
            seen.add(u["skill_id"])
            desc = lang.get(u["desc_key"], "").strip()
            desc_short = (desc[:200] + "...") if len(desc) > 200 else desc
            ctxs = contexts.get(u["skill_id"], [])
            depths = sorted({c["depth"] for c in ctxs}) if ctxs else []
            parents = sorted({c["parent_aid"] for c in ctxs}) if ctxs else []
            print(f"- skill `{u['skill_id']}` slot{u['slot']}: cond=`{u['raw_cond']}` beh=`{u['raw_beh']}`")
            if desc_short:
                print(f"  > {desc_short}")
            if ctxs:
                print(f"  emit ctx: depths={depths}, parent_aids={parents[:6]}")
        print()


if __name__ == "__main__":
    main()
