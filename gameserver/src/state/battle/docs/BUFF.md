# Buff System

## Data model

`BuffInstance` (`manager/buff_mgr.rs`) is the runtime representation of one active buff:

| Field | Meaning |
|---|---|
| `uid` | Unique runtime ID (monotonic counter, separate range for defender side) |
| `buff_id` | Config ID from `skill_buff` table |
| `type_id` | `skill_bufftype` ID, used for group queries and exclude-type logic |
| `from_uid` | Entity UID that applied the buff |
| `from_skill_id` | Skill that applied the buff (0 = unknown) |
| `duration` | Rounds remaining; 0 = permanent |
| `stacks` | Maps to `buff.count` in packets |
| `layer` | Maps to `buff.layer` in packets; used by DoT for stack damage |
| `act_common_params` | Opaque string payload for some feature types |
| `refresh_policy` | How re-application is handled (see below) |

`BuffMgr` stores `HashMap<i64, Vec<BuffInstance>>` keyed by target UID.

---

## Refresh policies

`RefreshPolicy` controls what happens when a buff is applied to an entity that already holds it:

- **`UpdateInPlace`** (default) — keeps the existing UID, updates duration/stacks/layer in place.
- **`ReplaceOnExcludedOverlap`** — removes an active sibling matched by `bufftype.exclude_types`, then inserts fresh. Derived when `include_types` starts with `10` and `exclude_types` is non-empty.
- **`ReplaceOnSelfRefresh`** — removes the same `buff_id`, then inserts fresh.

Special cases handled before the policy check:
- **`uses_single_uid_layer_refresh`** (buff 30091120) — always refreshes the single instance in-place, preserving UID.
- **`uses_distinct_dot_carrier_instances`** (buffs 30980111, 30980132) — always pushes a new instance; multiple carriers coexist.
- **`is_stacked_include_type`** (include_type 10/12/14/15) — always pushes a new instance.
- **`is_poison_family`** (act IDs 803/844) — merges into one instance, accumulating `layer`.

---

## Lifecycle

### Adding (`BuffMgr::add`)

1. Build a `BuffInstance` from config defaults.
2. Override `stacks`/`layer` from call-site arguments.
3. Apply the stacking/refresh rules above.

### Round end (`Manager::on_round_end`)

- Decrement `duration` for all non-permanent buffs.
- Remove buffs whose `duration` reaches 0 and whose config `during_time` is non-zero.
- Removed instances are recorded in `step_deleted_buff_ids` for mid-step trigger matching.

### Removal

- `remove_by_uid` — remove one instance by buff UID.
- `remove_by_buff_id` — remove all instances with a given `buff_id`.
- `clear` — remove all buffs for an entity.
- All removal paths call `record_deleted` to populate `step_deleted_buff_ids`.

---

## Feature execution pipeline

When a buff is applied, `buff/apply.rs` calls into `buff_actions/`:

```
pre_buff_effects  →  [emit BUFFADD packet]  →  apply_buff_effects
```

`apply_before_buff_add_features` / `apply_after_buff_add_features` walk the buff's `features` string (`|`-separated entries, each `act_id#param…`), look up the `buff_act.type` string, and dispatch to handlers.

### `BuffStage` — when a handler fires

| Stage | `effectTime` | Trigger |
|---|---|---|
| `BeforeBuffAdd` | 0 | Before BUFFADD packet |
| `AfterBuffAdd` | 0 | After BUFFADD packet |
| `OnDeath` | 12 | Entity death |
| `RoundStartCure` | 102 | Round-start heal tick |
| `RoundStartCard` | 105 | Round-start card grant |
| `BeAttackedDefensive` | 207 | Incoming hit (defensive) |
| `OnCast` | 208 | Skill cast |
| `BeAttackedReactive` | 209 | Incoming hit (reactive) |
| `PostSkill` | 212 | After skill resolves |
| `RoundEndDot` | 302 | Round-end DoT tick |
| `OverflowHandler` | 305 | EX overflow |
| `RoundEndInjuryBank` | 307 | Round-end injury bank |

### Handler dispatch order

`BUFF_HANDLER_REGISTRY` (per-stage, new style) is walked first; `BUFF_ACTION_REGISTRY` (legacy cluster) is the fallback. First match wins.

### `FeatureTiming` — which stages a feature type runs in

- **`AfterBuffAdd`** (default) — runs only in `AfterBuffAdd`.
- **`BeforeAndAfterBuffAdd`** — runs in both `BeforeBuffAdd` and `AfterBuffAdd`. Applied to: `Attr`, `EachChangeAttr`, `LostHpCountAddBuff`.

---

## Handler catalogue

| Handler | `act_type` | Stage | Effect |
|---|---|---|---|
| `AttrBeforeHandler` | `Attr` | `BeforeBuffAdd` | Broadcasts attribute change before buff add |
| `AttrAfterHandler` | `Attr` | `AfterBuffAdd` | Applies attribute change after buff add |
| `EachChangeAttrBefore/After` | `EachChangeAttr` | Both | Per-stack attribute scaling |
| `AttrFromEntityHandler` | `AttrFromEntity` | `AfterBuffAdd` | Attribute derived from another entity |
| `AttrOnlyCalDamageHandler` | `AttrOnlyCalDamage*` | `AfterBuffAdd` | Damage-calculation-only attribute modifier |
| `ShieldHandler` | `Shield` | `AfterBuffAdd` | Grants shield = `max_hp × permille / 1000` |
| `PoisonHandler` | `Poison` | `RoundEndDot` | DoT tick per layer at round end |
| `DeadlyPoisonHandler` | `DeadlyPoison` | `RoundEndDot` | DoT tick (deadly variant) |
| `BurnHandler` | `Burn` | `RoundEndDot` | DoT tick based on caster stat |
| `MasterHaloHandler` | `Halo` | `AfterBuffAdd` | Applies halo buff to allies |
| `SlaveHaloHandler` | `Halo` | `AfterBuffAdd` | Receives halo from master |
| `AddBuffBothHandler` | `AddBuffBoth` | `AfterBuffAdd` | Adds secondary buff to both sides |
| `LostHpCountAddBuffBefore/After` | `LostHpCountAddBuff` | Both | Adds buff scaled by lost HP |
| `ProbabilityAddBuffHandler` | `ProbabilityAddBuff` | `AfterBuffAdd` | Conditional buff add |
| `ReviveHandler` | `Revive` | `AfterBuffAdd` | Marks entity for revive |
| `CureUpByLostHpHandler` | `CureUpByLostHp` | `AfterBuffAdd` | Heal scaled by lost HP |
| `RaspberryHandler` | `Raspberry` | `AfterBuffAdd` | Raspberry mechanic bootstrap |
| `MarkerHandler` | various markers | `AfterBuffAdd` | Emits marker effect packets |
| `NoOpHandler` | `NoOp` / feature 772 | any | Emits nothing |

---

## DoT specifics

Poison/DeadlyPoison damage = `caster_atk × permille / 1000 × 1390 / 1000` (always crits).  
Burn damage = `caster_stat[attr_id] × rate / 1000`.  
Both deal one hit per `layer` per tick. `apply_real_hurt_fix` is applied before the crit multiply to account for injury-bank reductions.

---

## Querying buffs

```rust
buff_mgr.has(uid, buff_id)              // any instance with this buff_id?
buff_mgr.has_type(uid, type_id)         // any instance with this type_id?
buff_mgr.count_buff_ids(uid, &[...])    // sum of stacks across listed buff_ids
buff_mgr.count_type(uid, type_id)       // sum of stacks for a type_id
buff_mgr.get(uid)                       // all instances for an entity
buff_mgr.find_instance_by_buff_id(uid, buff_id)
```

`step_deleted_buff_ids()` exposes buff IDs removed in the current step for `BuffIdDel` trigger conditions. Reset by `clear_step_deleted_buff_ids()` at each step boundary.

---

## How buffs affect stats at calculation time

Buffs do **not** mutate `FightEntityInfo.attr` directly. Instead, `get_attr_bonus` (`utils.rs`) scans all active `BuffInstance`s on an entity at the moment a stat is needed and sums the contributions. The result is added to the base stat inline during damage/heal calculation.

### Attr IDs

| ID | Stat |
|---|---|
| 101 | Max HP |
| 102 | Attack |
| 103 | Defense |
| 201 | Crit rate (permille) |
| 202 | Anti-crit (permille) |
| 203 | Crit damage bonus (permille) |
| 204 | Crit damage reduction (permille) |
| 205 | AddDmg — outgoing damage multiplier bonus (permille) |
| 206 | DropDmg — incoming damage multiplier reduction (permille) |

### Feature types that contribute to `get_attr_bonus`

**`Attr`** — flat additive bonus. Format: `act_id#attr_id#amount`.  
Contribution = `amount × stack_multiplier`.  
Exception: stacked DropDmg (attr 206) always adds `amount` once regardless of stack count (live-like per-hit shield behavior).

**`AttrOnlyCalDamageAttack` / `BeAttacked` / `AttackType` / `BeAttackedType`** — same format and math as `Attr`, but only applies during damage calculation (not to base stat queries).

**`AttrByLostHp`** — scales bonus by the entity's lost HP ratio. Format: `act_id#source_attr#attr_ids(comma)#amounts(comma)#max_stacks`.  
Contribution = `base_amount × floor(lost_hp_permille × max_stacks / 1000)`.

### Stack multiplier

For stacked buff types (include_type 10/12/14/15), all instances sharing the same `features+type_id` key are collapsed: `stack_multiplier = instance_count + sum(layer - 1 for each instance with layer > 0)`.  
For non-stacked buffs: `stack_multiplier = max(stacks, 1)`.

---

## How buffs affect damage calculation

`calculate_damage` (`skill/damage.rs`) assembles the final hit value in this order:

1. **Effective ATK** — `caster.attr.attack + pending_attr_bonus(102)`.  
   If the caster has `AttrOnlyCalDamageReplaceAttr`, ATK is replaced: `replace_stat × permille / 1000` (e.g. Nuodika scales off Max HP instead of ATK).

2. **Defense mitigation** — `effective_atk × 1000 / (1000 + target_defense + pending_attr_bonus(103))`.

3. **Skill multiplier** — `base_param / 1000` (from skill config).

4. **Crit multiplier** — if `is_crit`: `(base_crit_permille + crit_dmg_bonus(203) - crit_dmg_reduction(204)) / 1000`.  
   Base crit permille = `fight_const[12] × 10` (default 1500).

5. **Career restraint** — fixed multiplier from career matchup table.

6. **AddDmg / DropDmg** — `dmg × (1000 + AddDmg(205) - DropDmg(206)) / 1000`.  
   Both buff-granted Attr(205/206) and the hero's static `exAttr.addDmg/dropDmg` (hardcoded per hero_id) are summed.

7. **RealHurtFix** — `dmg × (1000 + sum_of_RealHurtFix_519_permille) / 1000`.  
   Applied via `apply_real_hurt_fix`; used for debuffs like Tuesday's "Genesis DMG Taken +X%".

### Crit determination

`should_crit_hit` checks in order:
1. Caster has `MustCrit` or `MustCritBuff` buff feature → always crit.
2. Caster or target has `CantCrit` buff feature → never crit.
3. Otherwise: `effective_crit = clamp(crit_rate(201) + technic_bonus - anti_crit(202), 0, 1000)`. Roll a deterministic permille from `(round + version + battle_id, caster_uid, target_uid, skill_id)`.

---

## How buffs affect healing

`calculate_heal` (`skill/damage.rs`):  
`final_heal = base_param + floor(caster_attack × base_param / 100)`.

`calculate_heal_by_two_attr`:  
`final_heal = floor(target_missing_hp × missing_percent / 1000) + floor(caster_max_hp × caster_hp_percent / 1000)`.

Buff-granted heal modifiers (e.g. `CureUpByLostHp`) emit a notification effect (`ActEffect` type 347) at buff-add time; the actual HP change is applied separately by the skill behavior, not by the buff feature itself.

HP-modifying `Attr` buffs (attr_id 101) broadcast `MaxHpChange + CurrentHpChange` pairs in the `BeforeBuffAdd` stage so the client updates the HP bar before the buff icon appears. The new max = `base_hp + base_hp × rate / 1000` (for `Attr`) or `target_max_hp + caster_max_hp × source_rate / 1000` (for `EachChangeAttr`).
