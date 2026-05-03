#!/usr/bin/env python3
"""Primitive audit: for each condition + behavior wired in our Rust
impl that's referenced by fixture skills, show (a) the rule's type
name + default args, (b) sample skills using it with their in-game
descriptions, (c) the matching Rust enum variant location.

Goal: spot mis-implementations where the type name was wired but the
logic doesn't match what the description requires. Companion to
primitive_coverage.py (which finds UNMAPPED) — this one finds WRONG.

Usage:
  python scripts/primitive_audit.py                       # full markdown report
  python scripts/primitive_audit.py --conditions          # conditions only
  python scripts/primitive_audit.py --behaviors           # behaviors only
  python scripts/primitive_audit.py --filter HurtMagic    # focus on one type
"""

import argparse
import json
import re
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
GAMESERVER = REPO / "gameserver" / "src" / "state" / "battle"
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


def collect_skill_usage(fixture_skill_ids):
    """For each (condition_id, behavior_id), record sample skills that use it."""
    rows = load_table("skill_effect")
    cond_to_skills = defaultdict(list)
    beh_to_skills = defaultdict(list)
    for r in rows:
        if not isinstance(r, dict):
            continue
        sid = r["id"]
        in_fixtures = sid in fixture_skill_ids
        for i in range(1, 21):
            c_raw = r.get(f"condition{i}", "") or ""
            b_raw = r.get(f"behavior{i}", "") or ""
            c_id = parse_first_id(c_raw)
            b_id = parse_first_id(b_raw)
            if c_id:
                cond_to_skills[c_id].append({
                    "skill_id": sid, "slot": i, "raw": c_raw,
                    "in_fixtures": in_fixtures, "desc_key": r.get("desc", ""),
                })
            if b_id:
                beh_to_skills[b_id].append({
                    "skill_id": sid, "slot": i, "raw": b_raw,
                    "in_fixtures": in_fixtures, "desc_key": r.get("desc", ""),
                })
    return cond_to_skills, beh_to_skills


# Rust enum variant locator — returns file:line if found.
RUST_VARIANT_LOC = None


def build_rust_variant_index():
    """Scan Rust src for `BehaviorType::Foo` / `ConditionType::Foo` variant
    declaration sites in the types/*.rs files."""
    index = {"behavior": {}, "condition": {}}
    for kind, path in [
        ("behavior", GAMESERVER / "types" / "behavior.rs"),
        ("condition", GAMESERVER / "types" / "condition.rs"),
    ]:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        # Find variant name + line number
        for m in re.finditer(r"^\s+([A-Z][A-Za-z0-9]*)\s*[\{,\(]?", text, re.MULTILINE):
            name = m.group(1)
            if name in ("Self", "Some", "None", "Vec", "Option"):
                continue
            line_no = text[: m.start()].count("\n") + 1
            index[kind][name] = f"{path.relative_to(REPO).as_posix()}:{line_no}"
    return index


def find_variant_handler(variant_name, kind):
    """Find files where the variant is matched with a handler body.
    Heuristic: look for `BehaviorType::Foo` or `ConditionType::Foo`
    in any .rs file in gameserver/."""
    needle = f"{kind.capitalize()}Type::{variant_name}"
    matches = []
    for p in GAMESERVER.rglob("*.rs"):
        try:
            text = p.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for m in re.finditer(re.escape(needle), text):
            line_no = text[: m.start()].count("\n") + 1
            matches.append(f"{p.relative_to(REPO).as_posix()}:{line_no}")
    return matches


def write_report(target, kind, conds_or_behs, table, lang, args):
    """Print markdown-style audit report."""
    fixture_skill_ids = collect_fixture_actids()
    type_to_ids = defaultdict(list)
    for tid, ttype in table.items():
        if not ttype:
            continue
        type_to_ids[ttype].append(tid)

    # Build a Rust index
    rust_index = build_rust_variant_index()
    print(f"# {kind.capitalize()} primitive audit")
    print()
    print("Each entry: rule type → fixture skills using it → in-game description → Rust impl.")
    print("Mismatches between description and our wiring are the actionable signal.")
    print()
    # Sort by total fixture-skill count descending
    type_fixture_count = {
        ttype: sum(
            sum(1 for u in conds_or_behs.get(tid, []) if u["in_fixtures"]) for tid in tids
        )
        for ttype, tids in type_to_ids.items()
    }
    sorted_types = sorted(
        type_to_ids.items(),
        key=lambda kv: -type_fixture_count.get(kv[0], 0),
    )

    if args.filter:
        sorted_types = [(t, ids) for t, ids in sorted_types if args.filter.lower() in t.lower()]

    shown = 0
    for ttype, tids in sorted_types:
        if shown >= args.top and not args.filter:
            break
        if type_fixture_count.get(ttype, 0) == 0 and not args.include_unused:
            continue
        # Skip "None" — it's noisy and almost always means "no condition"
        if ttype == "None" and not args.filter:
            continue

        print(f"## `{ttype}`  (fixture-uses: {type_fixture_count.get(ttype, 0)})")
        print()
        # Rust impl status
        variant_locs = find_variant_handler(ttype, kind)
        if variant_locs:
            print(f"**Rust impl**: {len(variant_locs)} matches")
            for loc in variant_locs[:6]:
                print(f"- `{loc}`")
            if len(variant_locs) > 6:
                print(f"- (+{len(variant_locs) - 6} more)")
        else:
            print(f"**Rust impl**: ❌ no `{kind.capitalize()}Type::{ttype}` references found")
        print()
        # Sample fixture skills using this type
        print("**Fixture skills using it:**")
        seen_skills = set()
        for tid in tids:
            uses = conds_or_behs.get(tid, [])
            for u in uses:
                if not u["in_fixtures"]:
                    continue
                if u["skill_id"] in seen_skills:
                    continue
                seen_skills.add(u["skill_id"])
                desc = lang.get(u["desc_key"], "")
                desc_short = (desc[:160] + "...") if len(desc) > 160 else desc
                print(f"- skill `{u['skill_id']}` slot{u['slot']} (id `{tid}`): `{u['raw']}`")
                if desc_short:
                    print(f"  > {desc_short}")
                if len(seen_skills) >= 4:
                    break
            if len(seen_skills) >= 4:
                break
        print()
        shown += 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--conditions", action="store_true", help="conditions only")
    parser.add_argument("--behaviors", action="store_true", help="behaviors only")
    parser.add_argument("--filter", help="filter to a specific type name (case-insensitive)")
    parser.add_argument("--top", type=int, default=30, help="top N types by fixture usage")
    parser.add_argument("--include-unused", action="store_true",
                        help="include types with zero fixture usage")
    args = parser.parse_args()

    lang = load_lang()
    fixture_actids = collect_fixture_actids()
    cond_to_skills, beh_to_skills = collect_skill_usage(fixture_actids)
    cond_types = {r["id"]: r.get("type", "") for r in load_table("skill_behavior_condition") if isinstance(r, dict)}
    beh_types = {r["id"]: r.get("type", "") for r in load_table("skill_behavior") if isinstance(r, dict)}

    if args.behaviors or (not args.conditions and not args.behaviors):
        write_report("behavior", "behavior", beh_to_skills, beh_types, lang, args)
        print()
    if args.conditions or (not args.conditions and not args.behaviors):
        write_report("condition", "condition", cond_to_skills, cond_types, lang, args)


if __name__ == "__main__":
    main()
