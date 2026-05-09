# Schema cross-reference for currently-hardcoded paths

Purpose: identify which hardcoded skill/buff ID lookups in the engine
have a corresponding schema field that could drive them, and which
genuinely need code-side decisions. Companion to `_effecttime_catalog.md`.

The thesis: the engine encodes lots of behavior in raw integer IDs
("if buff_id == X..."). For each such hardcode we ask:

1. **Identifier**: is the raw ID just a stable identifier for a known
   concept (acceptable, like a constant)?
2. **Classifier**: is the raw ID being used to put the skill/buff into
   a behavioral category (problem — should be schema query)?

Classifiers are the retire targets. Identifiers are usually fine.

## Two patterns

### Pattern A — Schema does encode it; we just don't read it

Examples found so far:

| Hardcoded site | Schema field that encodes it | Decoder location |
|--|--|--|
| Round-start passive list (multiple files) | condition opcode suffix `104` | `skill_behavior_condition.id` |
| Over-fire suppression list | `roundLimit1/2/3` per behavior | `skill_effect.json` |
| `tuesday::pick_lock_sound_enemy_target` | target code 208 ("most current HP") | `targets.rs:285` already implements |
| Willow priority "most poison" | `behavior_type=AddTargetBuffByPoison` | `skill_behavior.json:60112` |
| Sotheby holder consume (`30090111\|30090112`) | `effectTime=208` + `act_type=AddBuffBoth` | `buff_act.json:850` |
| DOT carrier list (Poison/DeadlyPoison/Burn) | `effectTime=302` + `act_type` | retired by 4.76 dispatcher |

### Pattern B — Schema doesn't encode it; legitimate engine constant

Examples:

| Hardcoded site | Why it stays | Constant or function? |
|--|--|--|
| `HEDONISM_IMPLEMENT_SKILL_ID = 30630151` | Naming a specific Pickles passive for self-documentation | Constant |
| `is_pickles(model_id)` / `is_tuesday` etc. | Identifying a hero by stable model_id | Function |
| Boss state cycle skill IDs (`530000721` etc.) | Boss state machine — outside buff schema | Constants |
| Empathy redirect targets | Damage pipeline mutation context | Code |

## Detailed sweep

### `mechanics/channel.rs`

Survey: 8 hardcoded ID references. All three categories mixed.

| Line | Hardcode | Category | Schema field |
|--|--|--|--|
| L189-191 (comment) | docs reference `530000745`, `530000721`, `530000751-753` | Documentation | n/a |
| **L638** | `if channel_buff_id == 31020114` | **CLASSIFIER** | `30290003.bufftype.takeStage`? Investigate |
| **L688** | `let lopera_channel = channel_buff_id == 31020114` | **CLASSIFIER (same)** | same |
| **L721** | `31020151 => 0` formula override | **CLASSIFIER** | skill 31020151 has `condition=649210` (effectTime=210 phase) — special phase |
| L445 (comment) | docs `530000721/751/752` | Documentation | n/a |

**Finding**: `31020114` (Lopera channel buff) is treated as a special channel type. Its `bufftype` likely flags it. Schema query: load `skill_buff.json` row for 31020114 and check `typeId`. If `typeId` is in a known "channel-class type set," that's the predicate. **Worth investigating.**

`31020151 => 0` formula override: skill 31020151 has condition opcode `649210` — that's an effectTime=210 (post-attack) phase opcode we haven't catalogued. Probably the override is "this skill emits at phase 210 with zero base damage" — schema-derivable IF we honor the phase code.

### `mechanics/nautika.rs`

Survey: 7 hardcoded ID references. Mostly Nautika-specific marker IDs.

These are likely **identifier constants** for known Nautika skills (3120 series). Memory note: "consolidate_into_bundle" is shape-repair, not classifier. Stays as constants.

### `mechanics/phase_change.rs`

Survey: 6 references including `30980151` and `70009`.

```rust
// L104 (comment): 'static list (e.g. 70009 from additionRule, 30980151 from...)'
```

**Finding**: this is a **CLASSIFIER** — checks if a skill is in "round-start passive injection" set. Schema query: any skill whose first condition opcode ends in `104` is a round-start passive. Retire via shared predicate.

### `heroes/pickles.rs`

```rust
const HEDONISM_IMPLEMENT_SKILL_ID: i32 = 30630151;
const HEDONISM_MARKER_SKILL_ID: i32 = 30630171;
const PICKLES_RECENT_ALLY_BUFF_IDS: [i32; 2] = [30630112, 30630113];
const HEDONISM_BUFF_IDS: [i32; 2] = [30630114, 30630115];
```

**All four are identifier constants** — they name specific Pickles
mechanics for self-documentation. The repair functions that use them
(post-emission shape repair) are themselves layer-2 (LIVE
serialization shape, retires only after EventQueue Phase 5).

**No retire here.** These are honest constants.

### `buff_actions/add_buff_both.rs`

```rust
const DUALITY_POTION_HOLDER_BUFF_ID: i32 = 30091120;

// L78
if ctx.buff_id == DUALITY_POTION_HOLDER_BUFF_ID {
    return;   // skip on-add path
}
```

**Finding**: this is a **CLASSIFIER** — "skip on-add for buffs whose
AddBuffBoth fires on cast not on add."

Schema test: `30091120`'s `850 AddBuffBoth` has `effectTime=208`
(on-cast). Other `AddBuffBoth` carriers (`72300002`, `109310005`) have
the same `effectTime=208` per buff_act.json. **All AddBuffBoth carriers
should skip on-add — the `effectTime=208` IS the predicate.**

Retire when `OnCast` dispatch site lands (catalog Phase C). The skip
becomes "if `effectTime` of this act is non-zero, dispatch this stage
in its own slot, not on-add."

### `buff_mgr.rs:122` distinct DOT carve-out

```rust
matches!(buff_id, 30980111 | 30980132)
```

**Finding**: classifier. Schema test: both buffs have `typeId` pointing
to the same `skill_bufftype` row, but their `includeTypes` field?

```python
buff 30980111 typeId 30980111
buff 30980132 typeId 30980111   # same type
```

Both share `typeId 30980111`. Need to look at that row. If the type
has a "do not merge instances" flag (perhaps `cannotRemove`,
`includeTypes='1'`, or similar), the carve-out is schema-derivable.

**Investigate**: does Tuesday's `30980111` typeId row have a flag that
generalizes to "distinct DOT carrier"?

## Already-decoded schema axes

Catalog decoders that work today:

| Schema axis | Decoder | Used by |
|--|--|--|
| `buff_act.effectTime` | direct field | 4.76 dispatcher |
| condition opcode digit suffix | `id // 1000 % 1000` (tail digits) | parser already does it for some |
| `behaviorTarget` codes (102/103/202/...) | hardcoded match in `targets.rs` | every skill resolution |
| `roundLimit1..N` per behavior | `skill_effect.json` per-behavior | partial honoring |
| `behavior_type` enum (60112=AddTargetBuffByPoison etc.) | per-handler `BehaviorAction` impls | mostly uniform |
| `bufftype.includeTypes` (10/12/14/15 → "stacking carrier") | `BuffMgr::is_stacked_include_type` | partial |

## Suspected-but-not-yet-decoded axes

| Suspected axis | Where it might live | What it would unlock |
|--|--|--|
| Channel-class predicate | `bufftype.takeStage`? `typeId` ranges? | Lopera/channel-specific carve-outs in `mechanics/channel.rs` |
| Lock-target picker rule | `bufftype.type=9`? | `tuesday::pick_lock_sound_enemy_target` |
| "Does not merge" carrier flag | `bufftype.includeTypes` variant? | Tuesday distinct DOT, Sotheby holder UID |
| Magic-circle target scope | `magic_circle.circleType`? `enemyAttrs/Buff` targeting code | `pick_enemy_buff_target_for_creator` dispatcher |
| Round-start-only passive class | condition opcode suffix already (104) | `phase_change`, `step_shape`, scope filter lists |

## Investigation queue (in priority order)

1. **Channel-class predicate** — does `bufftype.takeStage` or a
   similar field separate channel buffs from regular buffs? If yes,
   `mechanics/channel.rs::lopera_channel` and others retire.
2. **"Does not merge" carrier flag** — verify Tuesday's `30980111`
   typeId row carries a flag that generalizes. If yes,
   `BuffMgr::is_stacked_include_type` retires the manual carve-out.
3. **Lock-target picker** — verify `30980131.bufftype.type=9` is the
   "lock" type and check if other `type=9` buffs exist with documented
   target rules.
4. **Magic-circle `enemyBuff` targeting** — broadcast vs single-pick;
   confirm against fixtures for battle3 wave-respawn rounds.

## Output for next sessions

After each investigation, this doc grows. Each successfully-decoded
axis enables a separate retire session (one PR per mechanic file
emptied). The pattern that's emerging:

**The "shadow subsystem" memory entry was right that hardcoding
accumulated — but the schema fields are mostly there. Each hardcode
is one investigation away from a schema query.**

---

## Investigation results (sessions appended)

### #1 — Channel-class predicate (`mechanics/channel.rs:638`) — RETIREABLE

**Schema field:** `skill_bufftype.type == 14` identifies all channel
buffs (319 across the game).

**Distinguishing channel sub-types:** the `act_id` in the buff's
features.

| Channel act_id | Type label | Used by |
|--:|--|--|
| 731 | ContinueChannel | Many heroes (rank-skill channels) |
| 732 | CastChannel variant | Many heroes |
| 733 | ContinueChannelXX | Many |
| 742 | CastChannel | Many (`*131..134` rank ladders) |
| **825** | **ConsumeBuffContinueChannel** | **Lopera (`31020114`/`23`/`24`), Sotheby Detonate2 (`82200004`)** |
| 838 | CountContinueChannel | Pickles, others |
| 846 | DuduBoneContinueChannel | Willow Hag's Bane families |
| 1024 | MonitorContinueChannel | Sentinel `31260131..134`, boss `7300003`, similar |
| 1054 | ConsumeBuffLayerCastChannel | Specific compound channels |

**Retire path:** replace `mechanics/channel.rs:638
if channel_buff_id == 31020114` with a schema query:
- `bufftype.type == 14`
- AND `features` contain `act_id == 825 ConsumeBuffContinueChannel`

That predicate matches Lopera's three channel ranks AND Sotheby's
holder — meaning the Lopera-specific carve-out is probably
incorrectly narrow today. Sotheby's `82200004` has the same channel
shape and probably needs the same handling.

### #2 — Tuesday distinct DOT carrier — TWO SCHEMA SOURCES

**`30980111` granted via `CatapultBuff (60074)`** — only 2 buffs in
the entire game grant Poison-family carriers via this behavior. Clean
schema query: "behavior is `CatapultBuff`."

**`30980132` granted via vanilla `AddBuff (1)`** from three different
skill_effects (`30980131/132/133`). Same behavior type Willow uses
(`31040005` from `31040141`).

**Hypothesis (under test in 4.77)**: LIVE keys carriers by
`(buff_id, from_skill_id)` rather than `buff_id` alone. Tuesday's
three EX rank tiers grant the same buff from different skill_ids →
distinct carriers. Willow's repeated grants from one skill_id →
merge. Same hypothesis covers Sotheby's
`uses_single_uid_layer_refresh(30091120)` carve-out.

If 4.77 confirms the hypothesis, both BuffMgr carve-outs retire as a
single structural change.

