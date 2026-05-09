#!/usr/bin/env python3
"""Skill walker — surface a skill's in-game description alongside its
parsed condition + behavior chain, with wiring status from the engine.

The point: 90% of bugs are "the engine isn't doing what the description
says". This tool puts the description and the implementation side-by-side
so you can read it and tell. No more chasing audit numbers without
knowing whether the wired primitives even *match* the intent.

For a given skill_id, prints:
  1. **Owner** — which hero/boss/psychube this belongs to
  2. **Description** — from prydwen scrape (`scripts/data/heroes/<slug>.json`)
  3. **Conditions** — every non-empty condition slot, with the parsed
     `(condition_id, type, parameters, target)` tuple plus a wiring
     status (✓/✗) against `gameserver/src/state/battle/skill/condition/`
  4. **Behaviors** — every non-empty behavior slot, with the parsed
     `(behavior_id, type, parameters, target)` tuple plus a wiring
     status against `gameserver/src/state/battle/skill/behavior/parser.rs`
  5. **Buff providers** — any buff whose features grant this skill (via
     `865 AddPassiveSkills` or similar). For chained grants, walks
     upstream to find the original carrier.
  6. **Channel chain** — flags if any gating buff is referenced in a
     `1024 MonitorContinueChannel` feature (i.e. this skill is a
     channel-monitored reactive, not an owner-card-fire passive).
  7. **Semantic gap heuristic** — flags obvious mismatches like
     "description mentions enemy/ally action trigger but conditions are
     all static (`HasBuffId` / `None`)".

Usage:
  python scripts/skill_walk.py 31260181              # one skill
  python scripts/skill_walk.py 30090111 30090112      # several
  python scripts/skill_walk.py --hero sentinel        # all skills owned by a hero
  python scripts/skill_walk.py --hero sentinel --include-keywords  # incl. keyword skills
  python scripts/skill_walk.py --grep poison --json   # scan + dump JSON

The output is text-table by default; `--json` produces a machine-readable
form that pairs nicely with `audit_skill_gaps.py`.
"""

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "data" / "excel2json"
HEROES = REPO / "scripts" / "data" / "heroes"
GAMESERVER_SRC = REPO / "gameserver" / "src" / "state" / "battle"


# --------------------------------------------------------------------- loaders


def load_table(name):
    text = (DATA / f"{name}.json").read_text(encoding="utf-8")
    data = json.loads(text)
    return data[1] if isinstance(data, list) and len(data) >= 2 else data


def load_lang():
    text = (DATA / "language_en.json").read_text(encoding="utf-8")
    obj, _ = json.JSONDecoder().raw_decode(text.lstrip())
    return dict(obj[1])


def load_heroes():
    out = {}
    for path in sorted(HEROES.glob("*.json")):
        if path.stem.startswith("_"):
            continue
        with path.open(encoding="utf-8") as f:
            out[path.stem] = json.load(f)
    return out


# ---------------------------- fixture activity scanner

# Cache: scan all fixture begin_round files once and remember every actId
# emitted in LIVE and OURS. Lets the walker tell you whether a skill is
# fixture-active (real bug if unwired) or fixture-dead (defensive only).

_FIXTURE_ACTIDS = None


def fixture_emission_counts():
    """Return `{(battle, side): Counter(actId -> count)}` across all
    `tests/<battle>/begin_round_*.json` (LIVE) and
    `tests/runs/<battle>/my_begin_round_*.json` (OURS). Cached after
    first call."""
    global _FIXTURE_ACTIDS
    if _FIXTURE_ACTIDS is not None:
        return _FIXTURE_ACTIDS
    import glob
    from collections import Counter

    out = {}

    def count_actids(path):
        c = Counter()

        def walk(steps):
            for s in steps:
                aid = s.get("actId", 0)
                if aid:
                    c[aid] += 1
                for eff in s.get("actEffect", []) or []:
                    child = eff.get("fightStep")
                    if child:
                        walk([child])

        try:
            with open(path, encoding="utf-8") as f:
                d = json.load(f)
        except Exception:
            return c
        walk(d.get("round", {}).get("fightStep", []))
        return c

    for battle in ["battle1", "battle2", "battle3"]:
        live = Counter()
        ours = Counter()
        for f in sorted(glob.glob(f"tests/{battle}/begin_round_*.json")):
            if "request" in f:
                continue
            live += count_actids(f)
        for f in sorted(glob.glob(f"tests/runs/{battle}/my_begin_round_*.json")):
            ours += count_actids(f)
        out[(battle, "LIVE")] = live
        out[(battle, "OURS")] = ours
    _FIXTURE_ACTIDS = out
    return out


def fixture_activity(skill_id):
    """Per-battle (LIVE, OURS, Δ) tuples for a skill_id. Returns
    `[(battle, live, ours, delta), ...]` for battles where either side
    has at least one emission."""
    counts = fixture_emission_counts()
    rows = []
    for battle in ["battle1", "battle2", "battle3"]:
        l = counts.get((battle, "LIVE"), {}).get(skill_id, 0)
        o = counts.get((battle, "OURS"), {}).get(skill_id, 0)
        if l or o:
            rows.append((battle, l, o, o - l))
    return rows


def find_skill_in_hero(skill_id, hero):
    """Return (incantation_name, kind, rank, description, keywords) if
    the skill_id appears in this hero's incantation rows. For
    buff-granted passives that don't have a top-level slot, fall back to
    aggregating the hero's full incantation keywords + insights so the
    walker can still surface description text — every channel/halo
    reactive is described in some keyword of the parent skill."""
    for inc in hero.get("incantations", []):
        for lvl in inc.get("levels", []):
            if lvl.get("skill_id") == skill_id:
                return {
                    "incantation": inc.get("name"),
                    "kind": inc.get("kind"),
                    "rank": lvl.get("rank"),
                    "description": lvl.get("description", ""),
                    "keywords": [
                        {"name": k.get("name", ""), "description": k.get("description", "")}
                        for k in (inc.get("keywords") or [])
                    ],
                }
        for form in inc.get("alt_forms", []) or []:
            if skill_id in (form.get("skill_ids") or []):
                return {
                    "incantation": inc.get("name") + f" (alt form {form.get('form_index')})",
                    "kind": inc.get("kind"),
                    "rank": None,
                    "description": "(alt-form skill — see incantation primary description)",
                    "keywords": [],
                }
    # Fallback: skill isn't in slot data (likely a buff-granted reactive
    # like Sentinel's 31260181). Surface every keyword across the hero's
    # incantations + insights/euphoria — the description we want for
    # `31260181` is in the [Hour of Repentance] keyword text on Night
    # Watch, even though the skill_id itself isn't slotted there.
    keywords = []
    for inc in hero.get("incantations", []):
        for k in inc.get("keywords") or []:
            keywords.append(
                {"name": k.get("name", ""), "description": k.get("description", "")}
            )
    insight_lines = []
    for tier in hero.get("insights") or []:
        nm = tier.get("name") or tier.get("tier") or "insight"
        d = (tier.get("description") or "").strip()
        if d:
            insight_lines.append(f"[{tier.get('tier','?')} {nm}] {d}")
    for tier in hero.get("euphoria") or []:
        nm = tier.get("name") or tier.get("tier") or "euphoria"
        d = (tier.get("description") or "").strip()
        if d:
            insight_lines.append(f"[{tier.get('tier','?')} {nm}] {d}")
    fallback_desc = "\n".join(insight_lines) if insight_lines else ""
    if keywords or insight_lines:
        return {
            "incantation": "(buff-granted / not in slot data — showing hero keyword + insight context)",
            "kind": "derived",
            "rank": None,
            "description": fallback_desc,
            "keywords": keywords,
        }
    return None


def find_owner(skill_id, heroes, character_rows):
    """Return (owner_label, source_dict|None, hero|None). owner_label tells
    you where this skill comes from (which hero, or boss/battle-rule/psychube).
    source_dict is the per-hero record from prydwen, if any."""
    sid_str = str(skill_id)

    # Pass 1: prydwen-scraped heroes — preferred because the scrape carries
    # incantation descriptions + keyword glossary.
    for slug, hero in heroes.items():
        sig = hero.get("signature", "")
        if sig and sid_str.startswith(sig + "0"):
            src = find_skill_in_hero(skill_id, hero)
            return (f"hero:{slug} (id={hero.get('id')}, signature={sig})", src, hero)
        if skill_id in (hero.get("skill_ids") or []) or skill_id == hero.get("ex_skill_id"):
            src = find_skill_in_hero(skill_id, hero)
            return (f"hero:{slug} (id={hero.get('id')}, exact match)", src, hero)
        # Short buff/skill ids of the form `<sig><1-2 digits>` (e.g. Pickles
        # `30631` = sig 3063 + "1") aren't covered by `<sig>0…` because they
        # never carry the trailing zero. Match them when the prefix is the
        # full signature and the residual digits fit within the 2-character
        # buff-id slot the engine uses.
        if sig and sid_str.startswith(sig) and 0 < len(sid_str) - len(sig) <= 2:
            src = find_skill_in_hero(skill_id, hero)
            return (
                f"hero:{slug} (id={hero.get('id')}, sig+short — buff/keyword id)",
                src,
                hero,
            )

    # Pass 2: fall back to `character.json` so heroes without a prydwen
    # scrape still get a name. Pickles (3063) currently has no scrape file
    # but is in `data/excel2json/character.json` with signature "3063".
    for row in character_rows or []:
        if not isinstance(row, dict):
            continue
        sig = str(row.get("signature") or "").strip()
        name = row.get("nameEng") or ""
        hero_id = row.get("id")
        if not sig:
            continue
        if sid_str.startswith(sig + "0") or (
            sid_str.startswith(sig) and 0 < len(sid_str) - len(sig) <= 2
        ):
            return (
                f"hero:{name.lower() or '?'} (id={hero_id}, signature={sig}, no prydwen scrape)",
                None,
                None,
            )

    # boss/monster — id starts with 5xxxxxxxx or 1xxxxxxx (rough heuristic)
    if 1_000_000 <= skill_id < 9_999_999:
        return (f"battle-rule/psychube/boss (id range {skill_id // 100000}xxxxx)", None, None)
    if skill_id >= 100_000_000:
        return (f"high-id skill (boss/monster?)", None, None)
    return (f"unknown owner", None, None)


# --------------------------------------------------------- condition/behavior


def parse_chain_arg(raw):
    """Turn '19203#31260151' into ('19203', ['31260151']) — strip leading
    `!`/`！` (negation marker) and ignore them for parsing. Compound
    `&` / `|` are split by caller."""
    if not raw:
        return None, []
    s = raw.strip()
    while s and s[0] in "!！":
        s = s[1:]
    while s and s[-1] in "!！":
        s = s[:-1]
    parts = s.split("#")
    if not parts or not parts[0].isdigit():
        return None, []
    return parts[0], parts[1:]


def split_compound(raw):
    """Split a compound condition string like '19203#X&501212' into the
    list of leaf condition strings. Returns single-element list if no
    operator."""
    if not raw:
        return []
    s = raw.strip()
    while s and s[0] in "!！":
        s = s[1:]
    while s and s[-1] in "!！":
        s = s[:-1]
    if "&" in s:
        return s.split("&")
    if "|" in s:
        return s.split("|")
    return [s]


def lookup_condition_type(cond_id_str, condition_rows):
    if not cond_id_str:
        return ""
    cid = int(cond_id_str)
    for r in condition_rows:
        if isinstance(r, dict) and r.get("id") == cid:
            return r.get("type", "")
    return ""


def lookup_behavior_type(beh_id_str, behavior_rows):
    if not beh_id_str:
        return ""
    bid = int(beh_id_str)
    for r in behavior_rows:
        if isinstance(r, dict) and r.get("id") == bid:
            return r.get("type", "")
    return ""


# --------------------------------------------------------- engine wiring scan

# Best-effort: scan source files once and remember the set of types our
# parser recognises.

_CONDITION_PARSED_TYPES = None
_BEHAVIOR_PARSED_TYPES = None


def _scan_string_match_arms(path, pattern):
    if not path.exists():
        return set()
    text = path.read_text(encoding="utf-8")
    return set(re.findall(pattern, text))


def wired_condition_types():
    global _CONDITION_PARSED_TYPES
    if _CONDITION_PARSED_TYPES is None:
        # combat.rs uses match cond_type { "ActiveUseSkill" => ... }
        # buff.rs uses similar. parser.rs handles special-case ids
        # (e.g. 585208 → TargetIsSelf, 586208 → TargetIsTeamNoMe).
        files = [
            GAMESERVER_SRC / "skill" / "condition" / "buff.rs",
            GAMESERVER_SRC / "skill" / "condition" / "combat.rs",
            GAMESERVER_SRC / "skill" / "condition" / "career.rs",
            GAMESERVER_SRC / "skill" / "condition" / "ex_point.rs",
            GAMESERVER_SRC / "skill" / "condition" / "life.rs",
            GAMESERVER_SRC / "skill" / "condition" / "bloodtithe.rs",
            GAMESERVER_SRC / "skill" / "condition" / "enter_fight.rs",
            GAMESERVER_SRC / "skill" / "condition" / "action.rs",
            GAMESERVER_SRC / "skill" / "condition" / "misc.rs",
        ]
        types = set()
        for f in files:
            types |= _scan_string_match_arms(f, r'"([A-Z][A-Za-z0-9]+)" =>')
        # parser.rs has special-case `if id == 585208` /
        # `id == 586208` style branches that don't go through cond_type
        # string lookup. The string returned by `lookup_condition_type`
        # for those rows would be e.g. `Equality` — which IS in the
        # data table type list but NOT in our parser cluster's match
        # arms. Extract the variant the special-case maps to and treat
        # it as wired so we don't false-flag.
        parser_path = GAMESERVER_SRC / "skill" / "condition" / "parser.rs"
        if parser_path.exists():
            text = parser_path.read_text(encoding="utf-8")
            # `return ConditionType::TargetIsSelf` / `::TargetIsTeamNoMe`
            for m in re.finditer(r"ConditionType::([A-Z][A-Za-z0-9]+)", text):
                types.add(m.group(1))
        _CONDITION_PARSED_TYPES = types
    return _CONDITION_PARSED_TYPES


def condition_type_is_wired(ctype, wired_set):
    """Map data-table type strings to wired ConditionType variants.
    Some data-table types don't have an exact match arm (because the
    parser handles them via id-based special cases), but the resulting
    `ConditionType` variant DOES exist. The lookup is approximate."""
    if not ctype:
        return False
    if ctype in wired_set:
        return True
    # Family / aliasing nuances:
    # - `Equality` (data-table type used for the 585208/586208 special
    #   cases) collapses to TargetIsSelf / TargetIsTeamNoMe via id-based
    #   parser branches.
    aliases = {
        "Equality": ("TargetIsSelf", "TargetIsTeamNoMe"),
    }
    if ctype in aliases and any(a in wired_set for a in aliases[ctype]):
        return True
    return False


def wired_behavior_types():
    global _BEHAVIOR_PARSED_TYPES
    if _BEHAVIOR_PARSED_TYPES is None:
        path = GAMESERVER_SRC / "skill" / "behavior" / "parser.rs"
        types = _scan_string_match_arms(path, r'"([A-Z][A-Za-z0-9]+)" =>')
        # Pick up `if id == NNN` arms (60073, 60110, etc.)
        if path.exists():
            text = path.read_text(encoding="utf-8")
            types |= {f"id={m}" for m in re.findall(r"if id == (\d+)", text)}
            # Pick up multi-arm match patterns like
            # `"AddBuff" | "AddBuffRound" | "AddBuffRound2" => ...` —
            # the simple regex above only catches the first quoted name.
            for m in re.finditer(
                r'"([A-Z][A-Za-z0-9]+)"(?:\s*\|\s*"([A-Z][A-Za-z0-9]+)")*\s*=>',
                text,
            ):
                # the regex itself only captures the first | branch,
                # but we also want subsequent ones — fall back to a
                # second pass that scans every quoted string preceding
                # `=>` separated by `|`.
                pass
            for m in re.finditer(r'((?:"[A-Z][A-Za-z0-9]+"\s*\|\s*)+"[A-Z][A-Za-z0-9]+")\s*=>', text):
                names = re.findall(r'"([A-Z][A-Za-z0-9]+)"', m.group(1))
                types.update(names)
            # Pattern guards like `t if t.starts_with("AttrFix")` —
            # treat the quoted prefix as a wired family marker.
            for m in re.finditer(r'starts_with\("([A-Z][A-Za-z0-9]+)"\)', text):
                types.add(m.group(1) + "*")
        _BEHAVIOR_PARSED_TYPES = types
    return _BEHAVIOR_PARSED_TYPES


def behavior_type_is_wired(btype, wired_set):
    if not btype:
        return False
    if btype in wired_set:
        return True
    # Family prefix matches (e.g. AttrFixByLoseHp matches AttrFix*).
    for w in wired_set:
        if w.endswith("*") and btype.startswith(w[:-1]):
            return True
    return False


# ----------------------------------------------------- buff feature topology


def buff_provider_chain(skill_id, buff_rows):
    """Find buffs whose features grant this skill_id (via 865
    AddPassiveSkills, or other passive-grant features). Walks upstream
    via 933 SubBuff and 771 MasterHalo to find the original carrier."""
    grant_acts = {"865"}
    sub_buff_acts = {"933"}
    master_halo_acts = {"771"}

    def grants_skill(buff, sid_str):
        f = buff.get("features", "") or ""
        for entry in f.split("|"):
            parts = entry.split("#")
            if not parts:
                continue
            if parts[0] in grant_acts and sid_str in parts[1:]:
                return True
        return False

    def parents(buff_id):
        out = []
        for b in buff_rows:
            if not isinstance(b, dict):
                continue
            f = b.get("features", "") or ""
            for entry in f.split("|"):
                parts = entry.split("#")
                if not parts:
                    continue
                # SubBuff: 933#child
                if parts[0] in sub_buff_acts and len(parts) >= 2 and parts[1] == str(buff_id):
                    out.append(("SubBuff", b["id"]))
                # MasterHalo: 771#x#child#y#z (slave is parts[2])
                if parts[0] in master_halo_acts and len(parts) >= 3 and parts[2] == str(buff_id):
                    out.append(("MasterHalo", b["id"]))
        return out

    granters = []
    for b in buff_rows:
        if isinstance(b, dict) and grants_skill(b, str(skill_id)):
            granters.append(b["id"])

    chain = []
    visited = set()
    stack = [(g, 0, "Direct") for g in granters]
    while stack:
        buff_id, depth, kind = stack.pop(0)
        if buff_id in visited or depth > 6:
            continue
        visited.add(buff_id)
        b = next((b for b in buff_rows if isinstance(b, dict) and b.get("id") == buff_id), None)
        chain.append({"buff_id": buff_id, "depth": depth, "kind": kind, "features": (b or {}).get("features", "")})
        for kind_p, parent_id in parents(buff_id):
            if parent_id not in visited:
                stack.append((parent_id, depth + 1, kind_p))
    return chain


def is_channel_monitored(buff_id, buff_rows):
    """Mirrors `gameserver::trigger::combat::is_channel_monitored_buff`:
    is `buff_id` referenced in any other buff's `1024 MonitorContinueChannel`
    feature?"""
    sid_str = str(buff_id)
    for b in buff_rows:
        if not isinstance(b, dict):
            continue
        f = b.get("features", "") or ""
        for entry in f.split("|"):
            parts = entry.split("#")
            if parts and parts[0] == "1024" and sid_str in parts[1:]:
                return True
    return False


def find_act_type_for_buff_id(act_id, act_rows):
    if not act_id:
        return ""
    for r in act_rows:
        if isinstance(r, dict) and r.get("id") == int(act_id):
            return r.get("type", "")
    return ""


# ----------------------------------------- buff feature decoder (inline 850/803/etc.)

# Hand-curated arg shape for known buff_act ids. Pulled from
# `gameserver/src/state/battle/buff_actions/*.rs` parsers — these label the
# numeric segments so the script can render `850#300901412#101#30091111` as
# `850 AddBuffBoth (buff_a=300901412 [Poison...], _=101, buff_b=30091111 [Cure...])`.
# Unrecognised ids fall back to a generic "look up any segment that matches a
# buff/skill id and annotate it" pass.
_ACT_ARG_LABELS = {
    "803": ["permille"],                                # Poison
    "844": ["permille"],                                # DeadlyPoison
    "849": ["permille"],                                # AdvancedCure
    "850": ["buff_a", "_", "buff_b"],                  # AddBuffBoth
    "865": ["skill_id", "skill_id", "skill_id", "skill_id"],  # AddPassiveSkills
    "933": ["child_buff"],                              # SubBuff
    "1024": ["watched_buff"],                           # MonitorContinueChannel
    "771": ["_", "slave_buff", "_", "_"],              # MasterHalo
    "772": ["_"],                                       # SlaveHalo
    "806": ["overflow_amount"],                         # ExPointOverflowBank (Rubuska)
    "60038": ["multiplier"],                            # OriginDamageFromInjuryBank
    "60040": ["multiplier"],                            # ConsumeInjuryBankAndDamage
    "60052": ["per_empathy"],                           # Kakania bounce damage
    "60073": ["dot_buff"],                              # SettleDotAndCostDotDuration
    "162": [],                                          # EmptyEffectMarker
    "167": [],                                          # StorageInjury
    "192": [],                                          # DamageFromAbsorb
    "195": [],                                          # InjuryBankHeal
}


def _name_for_buff_id(bid_str, buff_rows, lang):
    """If `bid_str` is a numeric id matching a row in `skill_buff.json`,
    return a short label `name [first sentence]` to annotate it. Returns
    empty string for non-matching values so the caller can leave the raw
    number in place."""
    try:
        bid = int(bid_str)
    except (ValueError, TypeError):
        return ""
    for r in buff_rows:
        if not isinstance(r, dict):
            continue
        if r.get("id") != bid:
            continue
        nm_key = r.get("name") or ""
        desc_key = r.get("desc") or ""
        nm = (lang.get(nm_key) or "").strip()
        desc = (lang.get(desc_key) or "").strip()
        first = desc.splitlines()[0] if desc else ""
        if nm and first:
            return f"{nm} — {first[:60]}"
        return nm or first[:60] or ""
    return ""


def decode_features(features_str, act_rows, buff_rows, lang):
    """Decode a buff `features` string into annotated `|`-separated entries.
    Returns a list of `{raw, act_id, act_type, args, decoded_str}` dicts —
    one per `|` segment — so callers can render or process them.

    Example: `"850#300901412#101#30091111|865#30631"` →
        [
          {"raw": "850#300901412#101#30091111", "act_id": "850",
           "act_type": "AddBuffBoth", "args": ["300901412","101","30091111"],
           "decoded_str": "850 AddBuffBoth (buff_a=300901412 [Poison: ...], _=101, buff_b=30091111 [Cure: ...])"},
          {"raw": "865#30631", "act_id": "865", "act_type": "AddPassiveSkills",
           "args": ["30631"],
           "decoded_str": "865 AddPassiveSkills (skill_id=30631)"},
        ]
    """
    if not features_str:
        return []
    out = []
    for entry in features_str.split("|"):
        entry = entry.strip()
        if not entry:
            continue
        parts = entry.split("#")
        act_id = parts[0]
        args = parts[1:]
        act_type = find_act_type_for_buff_id(act_id, act_rows) if act_id.isdigit() else ""
        labels = _ACT_ARG_LABELS.get(act_id, [])
        rendered = []
        for i, a in enumerate(args):
            label = labels[i] if i < len(labels) else None
            ann = _name_for_buff_id(a, buff_rows, lang)
            if label and label != "_":
                if ann:
                    rendered.append(f"{label}={a} [{ann}]")
                else:
                    rendered.append(f"{label}={a}")
            else:
                if ann:
                    rendered.append(f"{a} [{ann}]")
                else:
                    rendered.append(a)
        decoded = f"{act_id} {act_type or '?'}"
        if rendered:
            decoded += f" ({', '.join(rendered)})"
        out.append({
            "raw": entry,
            "act_id": act_id,
            "act_type": act_type,
            "args": args,
            "decoded_str": decoded,
        })
    return out


# -------------------------------------------------------------- semantic flag

EVENT_TRIGGER_PHRASES = [
    ("after an enemy", "enemy action"),
    ("when an enemy", "enemy action"),
    ("when attacked", "attacked"),
    ("when being attacked", "attacked"),
    ("after being attacked", "attacked"),
    ("when an ally", "ally action"),
    ("after an ally", "ally action"),
    ("when teammate", "teammate action"),
    ("when an ally triggers", "ally bullet trigger"),
    ("after the carrier attacks", "self attack"),
    ("after attacking", "self attack"),
    ("after casting", "self cast"),
    ("when triggering bullet", "bullet trigger"),
    ("when an enemy uses", "enemy uses skill"),
    ("at the start of the round", "round start"),
    ("at the start of each round", "round start"),
    ("when a round starts", "round start"),
    ("at the end of the round", "round end"),
    ("when a round ends", "round end"),
]


def detect_event_triggers(description):
    if not description:
        return []
    lower = description.lower()
    found = []
    for phrase, label in EVENT_TRIGGER_PHRASES:
        if phrase in lower:
            found.append(label)
    return list(dict.fromkeys(found))  # dedup, preserve order


STATIC_CONDITION_TYPES = {"None", "HasBuffId", "NoBuffId", "EnterFight"}


# Skills flagged in `memory/MEMORY.md` as architectural — fixing them needs
# multi-file or event-queue work, not a one-shot edit. Drift on these is
# expected; the classifier surfaces it as `architectural` so the user
# doesn't waste time chasing them.
ARCHITECTURAL_SKILL_IDS = {
    # 434415 / 435611 — psychube riders (Recoleta's "The Final Roll" /
    # Rubuska's "The Wandering Improviser"). Need PsychubeRider event
    # attachment per `_434415_psychube_findings.md`.
    434415, 435611,
    # Sotheby Duality Potion — 2 prior failed attempts, needs 4-piece fix.
    30091120, 30091111, 30091123, 30091129, 30091130, 30090146, 300901412,
    # Battle3 boss / wave-spawn timing related — upstream damage-value drift.
    1145004, 1145006, 1145002, 1144005, 1144007, 1144002, 1143002, 1143004,
    1143006, 11450041, 11450061, 1148002, 1144005,
    # Battle3 boss skills (40120111 etc.) — value drift from missing inline
    # passive families.
    40120111, 114300831, 114300811,
}


def architectural_skill_ids():
    return ARCHITECTURAL_SKILL_IDS


def findings_files_for_skill(skill_id, repo_root=REPO):
    """Return the list of `_<…>_findings.md` files at `repo_root` whose
    name contains the skill_id digits. Used by the drift classifier to
    flag a skill as `blocked` (prior failed-attempt context exists)."""
    sid_str = str(skill_id)
    out = []
    try:
        for path in repo_root.glob("_*_findings*.md"):
            if sid_str in path.name:
                out.append(path.name)
    except OSError:
        pass
    return out


def classify_drift(activity, skill_id, findings):
    """Categorise a skill's drift using the audit signal we already have.

    Inputs:
      - activity: list of `(battle, live, ours, delta)` from `fixture_activity`
      - skill_id: int
      - findings: list of findings filenames (from `findings_files_for_skill`)

    Returns one of:
      - `clean`       — no fixture activity OR all Δ=0
      - `blocked`     — a `_<sid>_findings.md` documents a prior failed attempt
      - `architectural` — skill_id in the architectural memory set
      - `small`       — max |Δ| ≤ 2 across all battles, fixture-active in ≤ 1 battle
      - `actionable`  — has fixture activity with drift but doesn't match above
    """
    if not activity:
        return "clean"
    deltas = [abs(d) for _, _, _, d in activity]
    if all(d == 0 for d in deltas):
        return "clean"
    if findings:
        return "blocked"
    if skill_id in ARCHITECTURAL_SKILL_IDS:
        return "architectural"
    active_battles = sum(1 for _, l, o, _ in activity if l or o)
    if max(deltas) <= 2 and active_battles <= 1:
        return "small"
    return "actionable"


def has_event_driven_condition(parsed_conditions):
    for slot in parsed_conditions:
        for leaf in slot["leaves"]:
            t = leaf.get("type", "")
            if t and t not in STATIC_CONDITION_TYPES:
                return True
    return False


# ------------------------------------------------------------------ main fn


def walk_buff(buff_id, ctx):
    """Lightweight buff-walker for IDs that aren't in `skill_effect` but ARE
    in `skill_buff`. Surfaces what we know: name/desc text, the resolved
    bufftype config, fixture activity, owner hero, decoded features, and the
    set of skills that grant this buff via 865 AddPassiveSkills (or similar).
    Designed to make `python scripts/skill_walk.py 30091111` produce useful
    output for buff IDs instead of an error."""
    buff = next(
        (r for r in ctx["buff_rows"] if isinstance(r, dict) and r.get("id") == buff_id),
        None,
    )
    if buff is None:
        return {"skill_id": buff_id, "error": "not in skill_effect or skill_buff"}

    bt_id = buff.get("typeId")
    bt = next(
        (r for r in ctx["bufftype_rows"] if isinstance(r, dict) and r.get("id") == bt_id),
        None,
    ) if bt_id else None

    owner_label, _, _ = find_owner(buff_id, ctx["heroes"], ctx["character"])

    name = ctx["lang"].get(buff.get("name", ""), "") or ""
    desc = ctx["lang"].get(buff.get("desc", ""), "") or ""

    # Find skills that grant this buff via 865 AddPassiveSkills features
    granters = []
    for r in ctx["skill_effect"]:
        if not isinstance(r, dict): continue
        # Check every behavior slot for `1#<this_buff_id>` (AddBuff target)
        for i in range(1, 21):
            beh = (r.get(f"behavior{i}") or "").strip()
            if not beh:
                continue
            parts = beh.split("#")
            if len(parts) >= 2 and str(buff_id) in parts[1:]:
                granters.append({
                    "skill_id": r.get("id"),
                    "behavior_slot": i,
                    "raw": beh,
                })
                break  # one entry per skill

    decoded_features = decode_features(
        buff.get("features", "") or "",
        ctx["act_rows"],
        ctx["buff_rows"],
        ctx["lang"],
    )

    activity = fixture_activity(buff_id)

    return {
        "buff_id": buff_id,
        "kind": "buff",
        "name": name,
        "desc": desc,
        "owner": owner_label,
        "type_id": bt_id,
        "is_good_buff": buff.get("isGoodBuff"),
        "during_time": buff.get("duringTime"),
        "effect_count": buff.get("effectCount"),
        "include_types": (bt or {}).get("includeTypes", ""),
        "exclude_types": (bt or {}).get("excludeTypes", ""),
        "bufftype_type": (bt or {}).get("type"),
        "bufftype_group": (bt or {}).get("group"),
        "take_stage": (bt or {}).get("takeStage"),
        "raw_features": buff.get("features", ""),
        "decoded_features": decoded_features,
        "granted_by_skills": granters,
        "fixture_activity": activity,
    }


def render_buff_text(result):
    out = []
    bid = result["buff_id"]
    out.append(f"=== buff {bid} {('— ' + result['name']) if result['name'] else ''} ===")
    out.append(f"  owner: {result['owner']}")
    if result.get("desc"):
        out.append("  description:")
        for line in result["desc"].splitlines():
            line = line.strip()
            if line:
                out.append(f"    > {line}")
    out.append(f"  typeId: {result.get('type_id')}  bufftype.type: {result.get('bufftype_type')}  group: {result.get('bufftype_group')}")
    out.append(
        f"  isGoodBuff: {result.get('is_good_buff')} (1=good 0=bad 2=neutral)  "
        f"duringTime: {result.get('during_time')}  effectCount: {result.get('effect_count')}"
    )
    out.append(f"  includeTypes: {result.get('include_types')!r}  excludeTypes: {result.get('exclude_types')!r}")
    if result.get("decoded_features"):
        out.append("  features:")
        for d in result["decoded_features"]:
            out.append(f"    • {d['decoded_str']}")
    elif result.get("raw_features"):
        out.append(f"  raw features: {result['raw_features']!r}")
    if result.get("granted_by_skills"):
        out.append("  granted by skill_effect rows (1#<this_id> in some behavior slot):")
        for g in result["granted_by_skills"][:10]:
            out.append(f"    skill_effect {g['skill_id']} slot{g['behavior_slot']}: {g['raw']!r}")
        if len(result["granted_by_skills"]) > 10:
            out.append(f"    ... and {len(result['granted_by_skills']) - 10} more")
    activity = result.get("fixture_activity") or []
    if activity:
        out.append("  fixture activity (per-battle LIVE / OURS / Δ):")
        for battle, l, o, d in activity:
            tag = "✓" if d == 0 else ("⚠" if abs(d) <= 1 else "✗")
            out.append(f"    {tag} {battle}: LIVE={l} OURS={o} Δ={d:+d}")
    else:
        out.append("  fixture activity: NONE (buff never appears in any test fixture by actId)")
    return "\n".join(out)


def walk_skill(skill_id, ctx):
    skill = next(
        (r for r in ctx["skill_effect"] if isinstance(r, dict) and r.get("id") == skill_id),
        None,
    )

    # Indirection: many boss skills (114300811, 114300831, etc.) live in
    # `skill.json` with a `skillEffect` field that points to a different
    # row id in `skill_effect.json`. Resolve via skill.json before falling
    # through to buff lookup.
    indirect_via = None
    if skill is None:
        skill_meta = next(
            (
                r
                for r in ctx.get("skill_rows", [])
                if isinstance(r, dict) and r.get("id") == skill_id
            ),
            None,
        )
        if skill_meta:
            target_eff = skill_meta.get("skillEffect")
            if target_eff and target_eff != skill_id:
                resolved = next(
                    (
                        r
                        for r in ctx["skill_effect"]
                        if isinstance(r, dict) and r.get("id") == target_eff
                    ),
                    None,
                )
                if resolved is not None:
                    skill = resolved
                    indirect_via = (skill_id, target_eff)

    if skill is None:
        # Fall through to buff lookup — many "skill" IDs in the codebase
        # are actually buff IDs (30091111, 30800111, 31040005, etc.).
        return walk_buff(skill_id, ctx)
    name = ctx["lang"].get(skill.get("name", ""), "") or ""
    desc_self = ctx["lang"].get(skill.get("desc", ""), "") or ""

    owner_label, source_record, hero = find_owner(skill_id, ctx["heroes"], ctx["character"])
    description = (source_record or {}).get("description") or desc_self

    # --- conditions / behaviors
    parsed_conditions = []
    parsed_behaviors = []
    for i in range(1, 21):
        cond = (skill.get(f"condition{i}") or "").strip()
        ctgt = skill.get(f"conditionTarget{i}")
        beh = (skill.get(f"behavior{i}") or "").strip()
        btgt = skill.get(f"behaviorTarget{i}")
        if not cond and not beh:
            continue
        leaves = []
        for leaf_str in split_compound(cond):
            cid_str, params = parse_chain_arg(leaf_str)
            ctype = lookup_condition_type(cid_str, ctx["condition_rows"])
            wired = condition_type_is_wired(ctype, ctx["wired_conditions"]) or ctype in (
                "None",
                "",
            )
            leaves.append(
                {
                    "raw": leaf_str,
                    "id": cid_str,
                    "type": ctype,
                    "params": params,
                    "wired": wired,
                }
            )
        parsed_conditions.append({"slot": i, "raw": cond, "leaves": leaves, "target": ctgt})

        bid_str, bparams = parse_chain_arg(beh)
        btype = lookup_behavior_type(bid_str, ctx["behavior_rows"])
        b_wired = behavior_type_is_wired(btype, ctx["wired_behaviors"]) or (
            f"id={bid_str}" in ctx["wired_behaviors"]
        )
        parsed_behaviors.append(
            {
                "slot": i,
                "raw": beh,
                "id": bid_str,
                "type": btype,
                "params": bparams,
                "target": btgt,
                "wired": b_wired,
            }
        )

    # --- buff providers
    providers = buff_provider_chain(skill_id, ctx["buff_rows"])

    # --- channel-monitored gating buffs (dedup, preserve order)
    channel_chain_buffs = []
    seen_channel = set()
    for slot in parsed_conditions:
        for leaf in slot["leaves"]:
            if leaf["type"] == "HasBuffId":
                for p in leaf["params"]:
                    try:
                        bid = int(p)
                    except ValueError:
                        continue
                    if bid in seen_channel:
                        continue
                    if is_channel_monitored(bid, ctx["buff_rows"]):
                        channel_chain_buffs.append(bid)
                        seen_channel.add(bid)

    # --- semantic gap detection
    triggers = detect_event_triggers(description)
    has_event_cond = has_event_driven_condition(parsed_conditions)
    semantic_flags = []
    if triggers and not has_event_cond:
        semantic_flags.append(
            f"description hints event trigger ({', '.join(triggers)}) "
            f"but conditions are all static — needs upstream mechanism (channel/halo/buff feature)"
        )
    if channel_chain_buffs:
        semantic_flags.append(
            f"channel-monitored gating buff(s) {channel_chain_buffs} — fires via MonitorContinueChannel"
        )
    unwired_conds = [
        leaf["type"]
        for slot in parsed_conditions
        for leaf in slot["leaves"]
        if not leaf["wired"]
    ]
    if unwired_conds:
        semantic_flags.append(f"unwired condition types: {sorted(set(unwired_conds))}")
    unwired_behs = [b["type"] for b in parsed_behaviors if not b["wired"] and b["type"]]
    if unwired_behs:
        semantic_flags.append(f"unwired behavior types: {sorted(set(unwired_behs))}")

    activity = fixture_activity(skill_id)
    findings = findings_files_for_skill(skill_id)
    drift_class = classify_drift(activity, skill_id, findings)

    # Decode each provider buff's full `features` string so the user can
    # read `850 AddBuffBoth (buff_a=…, …)` instead of a raw `850#x#y#z`.
    for prov in providers:
        prov["decoded_features"] = decode_features(
            prov.get("features", "") or "",
            ctx["act_rows"],
            ctx["buff_rows"],
            ctx["lang"],
        )

    return {
        "skill_id": skill_id,
        "indirect_via": indirect_via,
        "name": name,
        "owner": owner_label,
        "incantation": (source_record or {}).get("incantation"),
        "kind": (source_record or {}).get("kind"),
        "rank": (source_record or {}).get("rank"),
        "description": description,
        "keywords": (source_record or {}).get("keywords") or [],
        "conditions": parsed_conditions,
        "behaviors": parsed_behaviors,
        "buff_providers": providers,
        "channel_chain_buffs": channel_chain_buffs,
        "event_triggers": triggers,
        "has_event_driven_condition": has_event_cond,
        "semantic_flags": semantic_flags,
        "fixture_activity": activity,
        "findings_files": findings,
        "drift_class": drift_class,
    }


def render_text(result):
    out = []
    if result.get("error"):
        out.append(f"skill {result['skill_id']}: ERROR — {result['error']}")
        return "\n".join(out)
    sid = result["skill_id"]
    out.append(f"=== skill {sid} {('— ' + result['name']) if result['name'] else ''} ===")
    if result.get("indirect_via"):
        from_id, to_id = result["indirect_via"]
        out.append(f"  resolved via skill.json: skill {from_id} → skillEffect {to_id}")
    out.append(f"  owner: {result['owner']}")
    if result.get("incantation"):
        out.append(
            f"  source: {result['kind']} '{result['incantation']}' "
            f"{('(rank ' + str(result['rank']) + ')') if result['rank'] else ''}".rstrip()
        )
    desc = (result.get("description") or "").strip()
    if desc:
        out.append("  description:")
        for line in desc.splitlines():
            line = line.strip()
            if line:
                out.append(f"    > {line}")
    if result.get("keywords"):
        out.append("  keywords:")
        for kw in result["keywords"]:
            kw_desc = (kw.get("description") or "").strip().splitlines()[0] if kw.get("description") else ""
            out.append(f"    [{kw.get('name')}] {kw_desc[:200]}")
    out.append("  conditions:")
    if not result["conditions"]:
        out.append("    (none)")
    for slot in result["conditions"]:
        leaf_strs = []
        for leaf in slot["leaves"]:
            mark = "✓" if leaf["wired"] else "✗"
            params = ("#" + "#".join(leaf["params"])) if leaf["params"] else ""
            leaf_strs.append(f"c{leaf['id']}{params} → {mark} {leaf['type'] or '(none/unknown)'}")
        tgt = f" target={slot['target']}" if slot.get("target") else ""
        out.append(f"    slot{slot['slot']}: {', '.join(leaf_strs)}{tgt}")
    out.append("  behaviors:")
    if not result["behaviors"]:
        out.append("    (none)")
    for beh in result["behaviors"]:
        mark = "✓" if beh["wired"] else "✗"
        params = ("#" + "#".join(beh["params"])) if beh["params"] else ""
        tgt = f" target={beh['target']}" if beh.get("target") else ""
        out.append(
            f"    slot{beh['slot']}: b{beh['id']}{params} → {mark} {beh['type'] or '(unknown)'}{tgt}"
        )
    if result["buff_providers"]:
        out.append("  buff providers (this skill is granted via buff features):")
        for prov in result["buff_providers"]:
            out.append(
                f"    buff {prov['buff_id']} (depth {prov['depth']}, via {prov['kind']}):"
            )
            decoded = prov.get("decoded_features") or []
            if decoded:
                for d in decoded:
                    out.append(f"      • {d['decoded_str']}")
            elif prov.get("features"):
                out.append(f"      raw features: {prov['features']!r}")
    if result["channel_chain_buffs"]:
        out.append(
            f"  channel-monitored gating buffs: {result['channel_chain_buffs']} "
            f"(referenced by some buff's MonitorContinueChannel act 1024)"
        )
    activity = result.get("fixture_activity") or []
    if activity:
        out.append("  fixture activity (per-battle LIVE / OURS / Δ):")
        for battle, l, o, d in activity:
            tag = "✓" if d == 0 else ("⚠" if abs(d) <= 1 else "✗")
            out.append(f"    {tag} {battle}: LIVE={l} OURS={o} Δ={d:+d}")
    else:
        out.append("  fixture activity: NONE (skill never fires in any test fixture)")
    drift_class = result.get("drift_class")
    if drift_class:
        # `clean / small / actionable / architectural / blocked` — pick a
        # tag the user can grep for.
        cls_tag = {
            "clean": "✓",
            "small": "⚠",
            "actionable": "‼",
            "architectural": "🏛",
            "blocked": "⛔",
        }.get(drift_class, "?")
        out.append(f"  drift class: {cls_tag} {drift_class}")
    findings = result.get("findings_files") or []
    if findings:
        out.append("  findings files (prior failed attempts):")
        for f in findings:
            out.append(f"    - {f}")
    if result["semantic_flags"]:
        out.append("  semantic flags:")
        for f in result["semantic_flags"]:
            out.append(f"    ⚠ {f}")
    return "\n".join(out)


def collect_skill_ids_for_hero(heroes, slug, include_keywords=False):
    if slug not in heroes:
        return []
    h = heroes[slug]
    out = list(h.get("skill_ids") or [])
    if h.get("ex_skill_id"):
        out.append(h["ex_skill_id"])
    return sorted(set(out))


def grep_skills(needle, skill_effect, lang):
    needle = needle.lower()
    out = []
    for r in skill_effect:
        if not isinstance(r, dict):
            continue
        sid = r.get("id")
        if sid is None:
            continue
        desc_key = r.get("desc", "")
        desc = (lang.get(desc_key) or "").lower()
        if needle in desc:
            out.append(sid)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("skill_ids", type=int, nargs="*", help="one or more skill ids")
    ap.add_argument("--hero", help="walk all skills owned by this hero slug")
    ap.add_argument("--grep", help="walk every skill whose description matches NEEDLE (case-insensitive)")
    ap.add_argument("--json", action="store_true", help="emit JSON instead of text")
    ap.add_argument("--include-keywords", action="store_true", help="(reserved) include keyword skills")
    args = ap.parse_args()

    skill_effect = load_table("skill_effect")
    skill_rows = load_table("skill")
    buff_rows = load_table("skill_buff")
    bufftype_rows = load_table("skill_bufftype")
    condition_rows = load_table("skill_behavior_condition")
    behavior_rows = load_table("skill_behavior")
    act_rows = load_table("buff_act")
    character = load_table("character")
    lang = load_lang()
    heroes = load_heroes()
    wired_conds = wired_condition_types()
    wired_behs = wired_behavior_types()

    ctx = {
        "skill_effect": skill_effect,
        "skill_rows": skill_rows,
        "buff_rows": buff_rows,
        "bufftype_rows": bufftype_rows,
        "condition_rows": condition_rows,
        "behavior_rows": behavior_rows,
        "act_rows": act_rows,
        "character": character,
        "lang": lang,
        "heroes": heroes,
        "wired_conditions": wired_conds,
        "wired_behaviors": wired_behs,
    }

    skill_ids = list(args.skill_ids)
    if args.hero:
        skill_ids.extend(collect_skill_ids_for_hero(heroes, args.hero, args.include_keywords))
    if args.grep:
        skill_ids.extend(grep_skills(args.grep, skill_effect, lang))
    skill_ids = sorted(set(skill_ids))

    if not skill_ids:
        ap.print_help()
        sys.exit(1)

    results = [walk_skill(sid, ctx) for sid in skill_ids]
    if args.json:
        print(json.dumps(results, indent=2, ensure_ascii=False))
    else:
        for r in results:
            if r.get("kind") == "buff":
                print(render_buff_text(r))
            else:
                print(render_text(r))
            print()


if __name__ == "__main__":
    main()
