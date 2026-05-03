#!/usr/bin/env python3
"""Rule provenance: trace any actId emission back to the skill_effect
condition/behavior pair that produces it.

The battle engine is rule-based: every emission comes from a (condition,
behavior) pair on a skill_effect. When LIVE emits an actId that OURS
doesn't (or vice-versa), the gap is at the *piece* level — either a
condition isn't firing, or a behavior isn't emitting, or the engine
isn't reaching that pair at all. This tool answers:

  1. Which skill_effect rows can produce this actId?
  2. What condition/behavior slot is responsible?
  3. Who owns the skill (hero / boss / battle rule / psychube)?
  4. What does the in-game description say it should do?

For each candidate the tool prints the parsed condition + behavior
strings alongside our internal `ConditionType` / `BehaviorType` mapping
(via heuristics on the raw cN# prefix), so you can spot mis-implemented
primitives without grepping the codebase.

Usage:
  python scripts/rule_provenance.py 1143004                 # all sources
  python scripts/rule_provenance.py 31250144 --emission     # only what emits the actId
  python scripts/rule_provenance.py 30800161 --target-buff  # show buff features that reference it
  python scripts/rule_provenance.py 530000721 --children    # what does this skill itself invoke?
  python scripts/rule_provenance.py --audit-deltas          # run audit, then trace each delta
"""

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
TESTS = REPO / "tests"


def load_lang():
    text = (DATA / "language_en.json").read_text(encoding="utf-8")
    obj, _ = json.JSONDecoder().raw_decode(text.lstrip())
    return dict(obj[1])


def load_table(name):
    text = (DATA / f"{name}.json").read_text(encoding="utf-8")
    data = json.loads(text)
    return data[1] if isinstance(data, list) and len(data) >= 2 else data


def load_condition_types():
    """skill_behavior_condition.json: id -> type name."""
    rows = load_table("skill_behavior_condition")
    return {r["id"]: r.get("type", "") for r in rows if isinstance(r, dict)}


def load_behavior_types():
    """skill_behavior.json: id -> type name."""
    rows = load_table("skill_behavior")
    return {r["id"]: r.get("type", "") for r in rows if isinstance(r, dict)}


CONDITIONS = None
BEHAVIORS = None
LANG = None


def init_globals():
    global CONDITIONS, BEHAVIORS, LANG
    if CONDITIONS is None:
        CONDITIONS = load_condition_types()
    if BEHAVIORS is None:
        BEHAVIORS = load_behavior_types()
    if LANG is None:
        LANG = load_lang()


def parse_cN_id(raw):
    """`21209#3&40#0` → first id is 21209."""
    if not raw:
        return None
    head = raw.split("#")[0].split("&")[0].split("|")[0].strip()
    try:
        return int(head)
    except ValueError:
        return None


def parse_bN_id(raw):
    """`50010#3#1` → first id is 50010."""
    return parse_cN_id(raw)


def signature_owner(skill_id):
    s = str(skill_id)
    if s.startswith("5300"):
        return "battle rule"
    if s.startswith("4012") or s.startswith("251") or s.startswith("3011"):
        return "boss/monster"
    if s.startswith("11430") or s.startswith("11420") or s.startswith("11440") or s.startswith("11450"):
        return "boss reactive"
    if s.startswith("114300"):
        return "boss skill"
    # Hero signature: hero_id * 10000 + slot * 100 + rank
    if len(s) >= 6:
        hero_id = skill_id // 10000
        rows = load_table("character")
        hero = next((r for r in rows if isinstance(r, dict) and r.get("id") == hero_id), None)
        if hero:
            return f"hero {hero_id} ({hero.get('nameEng','?')})"
    if 1500 <= skill_id <= 1700:
        return "psychube"
    return "?"


def hero_dmg_type(skill_id):
    """1=Reality, 2=Mental, None=unknown."""
    rows = load_table("character")
    hero_id = skill_id // 10000
    hero = next((r for r in rows if isinstance(r, dict) and r.get("id") == hero_id), None)
    return hero.get("dmgType") if hero else None


def get_desc(skill_id):
    """Skill description text (English) via skill_effect.desc → language_en."""
    rows = load_table("skill_effect")
    s = next((r for r in rows if isinstance(r, dict) and r.get("id") == skill_id), None)
    if not s:
        return ""
    return LANG.get(s.get("desc", ""), "")


def find_skill_effect(skill_id):
    """Return the skill_effect row + parsed (condition, behavior) slot list."""
    rows = load_table("skill_effect")
    s = next((r for r in rows if isinstance(r, dict) and r.get("id") == skill_id), None)
    if not s:
        return None, []
    slots = []
    for i in range(1, 21):
        c_raw = s.get(f"condition{i}", "")
        b_raw = s.get(f"behavior{i}", "")
        ct_raw = s.get(f"conditionTarget{i}", "")
        bt_raw = s.get(f"behaviorTarget{i}", "")
        limit = s.get(f"limit{i}", 0)
        round_limit = s.get(f"roundLimit{i}", 0)
        if not c_raw and not b_raw:
            continue
        c_id = parse_cN_id(c_raw)
        b_id = parse_bN_id(b_raw)
        slots.append({
            "slot": i,
            "c_raw": c_raw, "c_id": c_id, "c_type": CONDITIONS.get(c_id, ""),
            "b_raw": b_raw, "b_id": b_id, "b_type": BEHAVIORS.get(b_id, ""),
            "ct": ct_raw, "bt": bt_raw,
            "limit": limit, "round_limit": round_limit,
        })
    return s, slots


def behaviors_emitting(act_id):
    """Find all skill_effect rows whose behavior args reference act_id.

    Returns list of (owner_skill_id, slot, behavior_raw, role).
    `role` is "AddBuff target", "UseSkill target", "AddPassiveSkills",
    or "arg-N" for unidentified positions.
    """
    rows = load_table("skill_effect")
    out = []
    str_id = str(act_id)
    for r in rows:
        if not isinstance(r, dict):
            continue
        for i in range(1, 21):
            b_raw = r.get(f"behavior{i}", "") or ""
            if not b_raw:
                continue
            # Match exact arg, not substring (avoid 30800161 matching 308001611)
            parts = b_raw.replace("|", "#").split("#")
            for j, p in enumerate(parts):
                p = p.strip()
                if p == str_id:
                    b_id = parse_bN_id(b_raw)
                    role = BEHAVIORS.get(b_id, f"behavior{b_id}")
                    out.append({
                        "owner_skill_id": r["id"],
                        "owner": signature_owner(r["id"]),
                        "slot": i,
                        "b_raw": b_raw,
                        "b_type": role,
                        "arg_position": j,
                    })
                    break
    return out


def buffs_referencing(act_id):
    """Find skill_buff rows whose features reference act_id."""
    rows = load_table("skill_buff")
    out = []
    str_id = str(act_id)
    for r in rows:
        if not isinstance(r, dict):
            continue
        feats = r.get("features", "") or ""
        if not feats:
            continue
        for entry in feats.split("|"):
            parts = entry.split("#")
            for p in parts[1:]:
                if p.strip() == str_id:
                    out.append({
                        "buff_id": r["id"],
                        "feature_entry": entry,
                        "buff_act": parts[0] if parts else "",
                    })
                    break
    return out


def trace_actid(act_id, args):
    init_globals()
    print(f"=== Rule provenance for actId={act_id} ===")
    print(f"Owner classification: {signature_owner(act_id)}")
    if args.target_buff is False:
        # Emission #1: actId IS a skill that emits as a wrapped fightStep
        skill, slots = find_skill_effect(act_id)
        if skill:
            print(f"\n[A] {act_id} is a skill_effect — fires as wrapped emission")
            desc = get_desc(act_id)
            if desc:
                print(f"    desc: {desc[:200]}")
            for slot in slots:
                cond_summary = f"{slot['c_raw']} → {slot['c_type'] or '(unmapped)'}"
                beh_summary = f"{slot['b_raw']} → {slot['b_type'] or '(unmapped)'}"
                bounds = []
                if slot["limit"]:
                    bounds.append(f"limit={slot['limit']}")
                if slot["round_limit"]:
                    bounds.append(f"roundLimit={slot['round_limit']}")
                bounds_str = " " + " ".join(bounds) if bounds else ""
                print(f"    slot{slot['slot']}: c{slot['c_id'] or '?'}={cond_summary} ct={slot['ct']}")
                print(f"             b{slot['b_id'] or '?'}={beh_summary} bt={slot['bt']}{bounds_str}")
        else:
            print(f"\n[A] {act_id} not a skill_effect")

        # Emission #2: behaviors that produce this actId as output
        if not args.emission_only or args.emission_only is False:
            emitters = behaviors_emitting(act_id)
            if emitters:
                print(f"\n[B] {len(emitters)} skill_effect behaviors reference {act_id}:")
                for e in emitters[:30]:
                    print(f"    skill {e['owner_skill_id']} ({e['owner']}) slot{e['slot']}: {e['b_type']} {e['b_raw']}")
                if len(emitters) > 30:
                    print(f"    ... +{len(emitters)-30} more")

    # Buff features referencing it
    buffs = buffs_referencing(act_id)
    if buffs:
        print(f"\n[C] {len(buffs)} skill_buff features reference {act_id}:")
        for b in buffs[:20]:
            print(f"    buff {b['buff_id']}: feature={b['feature_entry']} (act {b['buff_act']})")
        if len(buffs) > 20:
            print(f"    ... +{len(buffs)-20} more")

    # Children: if it's a skill, what does IT invoke?
    if args.children:
        skill, slots = find_skill_effect(act_id)
        if slots:
            print(f"\n[D] {act_id}'s downstream invocations:")
            for slot in slots:
                if slot["b_type"] in ("UseSkill", "RandomUseSkill", "AddBuff", "AddBuffBoth", "AddBuffRanId", "AddPassiveSkills"):
                    print(f"    slot{slot['slot']}: {slot['b_type']}#{slot['b_raw']}")


def audit_deltas(args):
    """Aggregate top audit deltas and trace each."""
    init_globals()
    # Reuse audit logic
    sys.path.insert(0, str(REPO / "scripts"))
    from collections import Counter
    def walk(steps):
        for s in steps:
            yield s
            for eff in s.get("actEffect", []) or []:
                child = eff.get("fightStep")
                if child:
                    yield from walk([child])

    deltas = defaultdict(int)
    for battle in ["battle1", "battle2", "battle3"]:
        live_dir = TESTS / battle
        ours_dir = TESTS / "runs" / battle
        if not live_dir.is_dir():
            continue
        for live_file in sorted(live_dir.glob("begin_round_*.json")):
            if "request" in live_file.name:
                continue
            rn = live_file.stem.split("_")[-1]
            ours_file = ours_dir / f"my_begin_round_{rn}.json"
            if not ours_file.exists():
                continue
            with live_file.open(encoding="utf-8") as f:
                live = json.load(f).get("round", {})
            with ours_file.open(encoding="utf-8") as f:
                ours = json.load(f).get("round", {})
            l_counts = Counter()
            o_counts = Counter()
            for s in walk(live.get("fightStep", [])):
                if s.get("actId", 0):
                    l_counts[s["actId"]] += 1
            for s in walk(ours.get("fightStep", [])):
                if s.get("actId", 0):
                    o_counts[s["actId"]] += 1
            for aid in set(l_counts) | set(o_counts):
                if l_counts[aid] != o_counts[aid]:
                    deltas[aid] += abs(l_counts[aid] - o_counts[aid])

    top = sorted(deltas.items(), key=lambda x: -x[1])[: args.top]
    for aid, mag in top:
        print(f"\n{'=' * 70}")
        print(f"actId={aid} Σ|Δ|={mag}")
        trace_actid(aid, args)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("act_id", nargs="?", type=int, help="actId to trace")
    parser.add_argument("--emission-only", action="store_true",
                        help="only show direct emission (skip behavior refs)")
    parser.add_argument("--target-buff", action="store_true",
                        help="only show buff features that reference this id")
    parser.add_argument("--children", action="store_true",
                        help="show what this skill invokes downstream")
    parser.add_argument("--audit-deltas", action="store_true",
                        help="trace top audit deltas")
    parser.add_argument("--top", type=int, default=10,
                        help="how many top deltas to trace (default: 10)")
    args = parser.parse_args()

    if args.audit_deltas:
        audit_deltas(args)
    elif args.act_id is not None:
        trace_actid(args.act_id, args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
