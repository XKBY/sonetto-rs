#!/usr/bin/env python3
"""Primitive coverage report: condition + behavior types used by skills
that emit non-zero audit deltas, cross-referenced against our Rust
ConditionType / BehaviorType enums.

Tells you which primitives are unmapped, stubbed, or have suspicious
implementations — the foundation work that unblocks per-skill fixes.

Usage:
  python scripts/primitive_coverage.py                    # all primitives in fixtures
  python scripts/primitive_coverage.py --unmapped-only    # show only unimplemented
  python scripts/primitive_coverage.py --top 20           # top 20 by usage frequency
"""

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
GAMESERVER = REPO / "gameserver" / "src" / "state" / "battle"


def load_table(name):
    text = (DATA / f"{name}.json").read_text(encoding="utf-8")
    data = json.loads(text)
    return data[1] if isinstance(data, list) and len(data) >= 2 else data


def load_condition_types():
    rows = load_table("skill_behavior_condition")
    return {r["id"]: r.get("type", "") for r in rows if isinstance(r, dict)}


def load_behavior_types():
    rows = load_table("skill_behavior")
    return {r["id"]: r.get("type", "") for r in rows if isinstance(r, dict)}


def load_buff_act_types():
    rows = load_table("buff_act")
    return {r["id"]: r.get("type", "") for r in rows if isinstance(r, dict)}


def parse_first_id(raw):
    if not raw:
        return None
    head = raw.split("#")[0].split("&")[0].split("|")[0].strip()
    try:
        return int(head)
    except ValueError:
        return None


def collect_skills_in_fixtures():
    """Return set of every actId that appears in either LIVE or OURS replay."""
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
            d = REPO / "tests" / which.rstrip("/") / battle if which == "" else REPO / "tests" / "runs" / battle
            if not d.is_dir():
                continue
            pattern = "begin_round_*.json" if which == "" else "my_begin_round_*.json"
            for f in d.glob(pattern):
                if "request" in f.name:
                    continue
                with f.open(encoding="utf-8") as fp:
                    walk(json.load(fp).get("round", {}).get("fightStep", []))
    return seen


def collect_conditions_behaviors_used(skill_ids):
    """For each skill_effect referenced, collect the condition + behavior
    type IDs it uses across all 20 slots."""
    rows = load_table("skill_effect")
    rows_by_id = {r["id"]: r for r in rows if isinstance(r, dict)}
    cond_usage = Counter()
    beh_usage = Counter()
    cond_to_skills = defaultdict(set)
    beh_to_skills = defaultdict(set)
    for sid in skill_ids:
        s = rows_by_id.get(sid)
        if not s:
            continue
        for i in range(1, 21):
            c_raw = s.get(f"condition{i}", "") or ""
            b_raw = s.get(f"behavior{i}", "") or ""
            c_id = parse_first_id(c_raw)
            b_id = parse_first_id(b_raw)
            if c_id:
                cond_usage[c_id] += 1
                cond_to_skills[c_id].add(sid)
            if b_id:
                beh_usage[b_id] += 1
                beh_to_skills[b_id].add(sid)
    return cond_usage, beh_usage, cond_to_skills, beh_to_skills


def collect_buff_acts_used():
    """Buff_act types from skill_buff features."""
    rows = load_table("skill_buff")
    usage = Counter()
    act_to_buffs = defaultdict(set)
    for r in rows:
        if not isinstance(r, dict):
            continue
        feats = r.get("features", "") or ""
        if not feats:
            continue
        for entry in feats.split("|"):
            parts = entry.split("#")
            if not parts:
                continue
            try:
                act_id = int(parts[0].strip())
            except ValueError:
                continue
            usage[act_id] += 1
            act_to_buffs[act_id].add(r["id"])
    return usage, act_to_buffs


def grep_rust_for(needle):
    """Search gameserver Rust source for a needle. Returns line count."""
    count = 0
    for p in GAMESERVER.rglob("*.rs"):
        try:
            text = p.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        if needle in text:
            count += text.count(needle)
    return count


def check_condition_impl_status(condition_types):
    """For each condition type name, return (refs_in_rust, parsed?).

    refs_in_rust: number of times the type name appears in gameserver
    source. Heuristic: if the type appears inside `ConditionType::<Name>`
    or string-matched in a parser arm, we count it.
    """
    status = {}
    type_to_rust = {}
    for cid, ctype in condition_types.items():
        if not ctype:
            status[cid] = ("", 0, "no-type-name")
            continue
        # Search for the type in Rust src — look for ConditionType:: variant
        variant_refs = grep_rust_for(f"ConditionType::{ctype}")
        # Also string-match
        str_refs = grep_rust_for(f'"{ctype}"')
        type_to_rust[cid] = (ctype, variant_refs, str_refs)
        if variant_refs == 0 and str_refs == 0:
            status[cid] = (ctype, 0, "UNMAPPED")
        elif str_refs > 0 and variant_refs == 0:
            status[cid] = (ctype, str_refs, "string-match-only?")
        else:
            status[cid] = (ctype, variant_refs, "wired")
    return status, type_to_rust


def check_behavior_impl_status(behavior_types):
    status = {}
    for bid, btype in behavior_types.items():
        if not btype:
            status[bid] = ("", 0, "no-type-name")
            continue
        variant_refs = grep_rust_for(f"BehaviorType::{btype}")
        str_refs = grep_rust_for(f'"{btype}"')
        if variant_refs == 0 and str_refs == 0:
            status[bid] = (btype, 0, "UNMAPPED")
        elif str_refs > 0 and variant_refs == 0:
            status[bid] = (btype, str_refs, "string-match-only?")
        else:
            status[bid] = (btype, variant_refs, "wired")
    return status


def check_buff_act_impl_status(buff_act_types):
    """Buff acts are routed by ID. Check whether the type-id appears in
    parser arms or buff_actions/ handlers."""
    status = {}
    for aid, atype in buff_act_types.items():
        # buff_act ids are matched as integers in Rust parser arms
        int_match = grep_rust_for(f"act_id == {aid}") + grep_rust_for(f'"{aid}"')
        # Also check the type name
        name_refs = grep_rust_for(f'"{atype}"') if atype else 0
        if int_match == 0 and name_refs == 0:
            status[aid] = (atype, 0, "UNMAPPED")
        else:
            status[aid] = (atype, max(int_match, name_refs), "wired-or-stringly")
    return status


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--unmapped-only", action="store_true",
                        help="show only primitives that aren't wired in Rust")
    parser.add_argument("--top", type=int, default=30,
                        help="show top N by usage")
    args = parser.parse_args()

    print("Loading config tables...")
    cond_types = load_condition_types()
    beh_types = load_behavior_types()
    buff_act_types = load_buff_act_types()

    print("Scanning replay fixtures for actIds...")
    fixture_skill_ids = collect_skills_in_fixtures()
    print(f"  {len(fixture_skill_ids)} unique actIds in fixtures")

    print("Collecting conditions + behaviors referenced by those skills...")
    cond_usage, beh_usage, cond_to_skills, beh_to_skills = (
        collect_conditions_behaviors_used(fixture_skill_ids)
    )
    buff_act_usage, _ = collect_buff_acts_used()

    print("Cross-referencing against Rust source...")
    cond_status, _ = check_condition_impl_status(cond_types)
    beh_status = check_behavior_impl_status(beh_types)
    act_status = check_buff_act_impl_status(buff_act_types)

    def emit_table(title, usage, status, top=args.top):
        print(f"\n=== {title} ===")
        print(f"{'id':>10} {'type':<35} {'usage':>6} {'status':<25} sample skills")
        print("-" * 105)
        rows = []
        for pid, count in usage.most_common(top * 3):
            s = status.get(pid, ("?", 0, "unknown"))
            type_name, refs, st = s
            if args.unmapped_only and "UNMAPPED" not in st:
                continue
            rows.append((pid, type_name, count, st))
        rows.sort(key=lambda x: (-x[2], x[0]))
        shown = 0
        for pid, type_name, count, st in rows:
            if shown >= top:
                break
            sample = sorted(list((cond_to_skills if title.startswith("Condition") else beh_to_skills).get(pid, set())))[:4]
            sample_str = ", ".join(str(s) for s in sample) if sample else ""
            print(f"{pid:>10} {type_name:<35} {count:>6} {st:<25} {sample_str}")
            shown += 1

    emit_table("Conditions used by fixture skills", cond_usage, cond_status)
    emit_table("Behaviors used by fixture skills", beh_usage, beh_status)

    # Buff acts: filter to ones used by fixture-related buffs only
    print(f"\n=== Buff acts (top {args.top} by usage across all skill_buff) ===")
    print(f"{'id':>6} {'type':<35} {'usage':>6} status")
    print("-" * 70)
    shown = 0
    for aid, count in buff_act_usage.most_common(args.top * 3):
        s = act_status.get(aid, ("?", 0, "unknown"))
        type_name, refs, st = s
        if args.unmapped_only and "UNMAPPED" not in st:
            continue
        print(f"{aid:>6} {type_name:<35} {count:>6} {st}")
        shown += 1
        if shown >= args.top:
            break


if __name__ == "__main__":
    main()
