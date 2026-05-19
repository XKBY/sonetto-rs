# Card Execution — After Play

How a played card (player or enemy) travels from selection through skill execution,
state mutation, trigger expansion, and final step assembly.

---

## Overview

```
Card selected
     │
     ▼
┌─────────────────────────────────────────────────────────┐
│  Resolve skill ID (choice cards, euphoria remapping)    │
│  Resolve target UID (dead-target fallback)              │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  SkillExecutor::execute_skill                           │
│    → behavior dispatch → ActEffect list                 │
│    → apply_pending_summons (deferred entity spawns)     │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  Inline passives (ActiveUseSkill) — player only         │
│    → fire matching passives → append effects            │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  make_skill_step(caster, target, skill_id, effects)     │
│    → raw FightStep with act_type=Skill                  │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  apply_step_and_maybe_sync (STATE MUTATION)             │
│    → play_step_data: HP, buffs, EX, bloodpool → Fight  │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  expand_trigger_chain (REACTIVE TRIGGERS)               │
│    → combat passives, buff reactives, HP sync, etc.     │
│    → produces additional FightSteps                     │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────┐
│  Host assembly (player) / direct push (enemy)           │
│    → splice triggers into host step's act_effect        │
│    → normalize effect order                             │
│    → push to round steps                               │
└─────────────────────────────────────────────────────────┘
```

---

## Player Card Execution

### Entry: `phase::player_actions::run`

Iterates `Vec<BeginRoundOper>`. Each operation is one card play.

### Step 1: Pre-operation bookkeeping

- Compute cloth-power delta (`cloth_power_delta_for_operation`).
- Compute pre-operation EX gain step (`ex_gain::pre_operation_ex_gain`) — standard +1 Moxie for the acting hero.
- Snapshot buff state (for deleted-buff detection after the step).
- Clear `buff_mgr.step_deleted_buff_ids()`.

### Step 2: `card_mgr.execute_operation` → `play_card`

**Skill resolution:**
1. Look up `CardInfo` from `state.selected_cards[op_index]` (or `replay_selected_cards`).
2. Extract `skill_id` from the card.
3. **Choice cards:** If `SKILL_CACHE` has no behaviors for this skill_id, use `oper.param3` as the chosen sub-skill.
4. **Euphoria:** `resolve_with_euphoria(fight, caster_uid, skill_id)` — remaps skill if caster is in euphoria state.

**Target resolution:**
- If `oper.to_id != 0`: validate target is alive, fall back to first alive enemy if dead.
- If `oper.to_id == 0`: pick first alive enemy.

**EX skill detection:**
- Compare resolved skill to entity's `ex_skill` field.
- If EX skill: immediately set `ex_point_mgr.set_ex_point(caster, 0)`.

**Skill execution:**
```rust
card_mgr.execute_skill_and_apply_pending_summons(
    rng, fight, managers, mechanics,
    caster_uid, target_uid, resolved_skill_id, &phase
)
```
This calls `SkillExecutor::execute_skill` (behavior dispatch → `Vec<ActEffect>`), then drains `pending_summons` to spawn entities on the live `Fight`.

**Inline passives (ActiveUseSkill):**
- For each passive in `collected.merged_for(caster_uid)`:
  - Build a `TriggerEvent` with `caster_uid`, `skill_id`, `action_order_index`.
  - Call `skill_should_fire(caster_uid, passive_skill_id, &event, 0, 0)`.
  - If it fires: `execute_passive_skill(ctx, caster_uid, target_uid, passive_skill_id, &phase)`.
  - Append resulting effects to `skill_effects`.

**Output:** `make_skill_step(display_caster_uid, target_uid, resolved_skill_id, op_index, skill_effects)` → `FightStep`.

### Step 3: State mutation — `apply_step_and_maybe_sync`

Walks every `ActEffect` in the step and applies mutations to live state:

| Effect type | Mutation |
|---|---|
| `Damage` / `Crit` / `FixedDamage` / ... | `entity.current_hp -= amount` |
| `Heal` | `entity.current_hp = min(current_hp + amount, max_hp)` |
| `BuffAdd` | `buff_mgr.add(target, buff_id, stacks, layer, from_uid, from_skill_id)` |
| `BuffUpdate` | `buff_mgr.update(buff_uid, new_count, new_layer)` |
| `BuffDel` | `buff_mgr.remove(buff_uid)` |
| `ExPointChange` (111) | `ex_point_mgr.add_ex_point(target, delta)` → `entity.ex_point` |
| `BloodpoolValueChange` | `bloodtithe.add_value(team, delta)` |
| `BloodpoolMaxChange` | `bloodtithe.set_max(team, max)` |
| `PowerChange` | `fight.attacker.power += delta` |
| `NewChangeWave` | Replace defender, clear dead-entity buffs, re-seed HP/EX |

After mutation: `sync_from_buff_mgr` + `sync_to_fight` writes manager state back to `Fight` entities.

### Step 4: Trigger expansion — `expand_trigger_chain`

Builds a `TriggerEvent` from the step's effects (scans for damaged UIDs, dealer UIDs, deleted buffs, added buffs, nested skill uses, etc.), then fires:

1. **Combat passives** — scans all entities for passive skills with combat-reactive conditions (`BeAttacked`, `ActiveUseSkill`, `HurtMagic`, etc.). Fires matching ones via `SkillExecutor`.
2. **Buff feature reactives** — walks active buffs with `effectTime` matching `BeAttackedReactive` (209), `OnCast` (208), `PostSkill` (212). Dispatches via `buff_actions::dispatch_stage`.
3. **HP sync pass** — emits HP-change effects for entities whose HP changed during triggers.
4. **Blood pool sync** — emits bloodpool delta effects.
5. **Blood value use skill** — fires skills gated on bloodpool thresholds.
6. **EX point sync** — emits EX-change effects for entities whose EX changed.
7. **Card energy sync** — emits card energy updates.

Each pass can produce additional `FightStep`s. The chain recurses: trigger steps themselves are applied via `apply_step_and_maybe_sync`, and if they produce further damage/buff changes, another trigger pass fires (depth-limited).

### Step 5: Host step assembly

For player SKILL steps, effects are assembled into a single host `FightStep`:

1. Direct skill effects → `Direct` lane of `HostEventAccumulator`.
2. Magic-circle embeds (if applicable) → spliced into direct effects.
3. Trigger chain results → converted to embedded effects (type 162 wrappers) → `Trigger` lane.
4. `BeAttacked` reactives from enemy side → `BeAttacked` lane.
5. Injury markers → `Injury` lane.
6. All lanes spliced into `host_step.act_effect` at computed insert indices.
7. `flatten_self_nested_skill_effects_v` — collapse redundant SKILL wrappers.
8. `normalize_player_skill_effect_order_v` — reorder to match LIVE wire format.
9. `register_round_host(caster, skill_id, HostSide::Player, step_index)`.
10. Push final `host_step` to round `steps`.

### Step 6: Post-operation cleanup

- `drain_and_emit_dead_hero_purge` — remove dead-hero cards from `player_deck`, emit `RemoveEntityCards` steps.
- Check `battle_end`; break if fight is over.
- After all operations: `refill_hand(player_hand, player_deck, ex_deck)` → replenish hand to target size.

---

## Enemy Card Execution

### Entry: `phase::enemy_actions::run`

Called during `non_terminal_round` phase, after player actions complete.

### Step 1: `card_mgr.execute_ai_turn`

**Two modes:**

#### A. Replay mode (`ai_override_steps` present)
- Iterates pre-captured `FightStep`s from a recorded battle.
- For each step with `caster_uid < 0` (enemy) and valid `skill_id`:
  - Fast-forwards wave state if caster belongs to a future wave.
  - Executes skill on a **preview fight clone** (not the live fight).
  - Applies summons to both preview and live fight.
  - Normalizes effects, clamps EX gain.
  - Builds `make_skill_step`.

#### B. Live mode (`ai_use_cards` from `RoundState`)
- `state.ai_use_cards` is populated during `round_open` from `generate_ai_deck`:
  - Each alive enemy picks `skill_group1[0]` (first skill).
  - Target is a random alive player hero.
- For each AI card:
  - Execute skill on preview fight clone.
  - Apply summons.
  - Build step.

**Key difference from player:** Enemy skill execution uses a **preview clone** of `Fight`/`Managers`/`Mechanics`. This means:
- Enemy skills don't see each other's mutations during the AI turn.
- Summons are applied to both preview and live fight (so subsequent enemies see spawned entities).
- The live `Fight` is only mutated by `apply_step_and_maybe_sync` in the outer loop.

### Step 2: Per-step processing in `enemy_actions::run`

For each `FightStep` returned by `execute_ai_turn`:

1. **Wave fast-forward:** If caster belongs to a future wave, advance live state.
2. **EX gain prelude:** `ex_gain::standard_action_ex_gain_for_uid` — emit +1 Moxie for the enemy actor (if applicable).
3. **State mutation:** `apply_step_and_maybe_sync(ctx, &step, true)` — same as player path.
4. **Buff snapshot delta:** Detect deleted buffs for trigger gating.
5. **Trigger expansion:** `expand_trigger_chain` — same reactive system as player.
6. **Host assembly:** Same `HostEventAccumulator` pattern:
   - Direct effects → `Direct` lane.
   - Trigger results → `Trigger` lane (embedded as type 162 wrappers).
   - BeAttacked reactives (player-side passives reacting to enemy attack) → `BeAttacked` lane.
   - Injury markers → `Injury` lane.
   - Splice all lanes, normalize order.
7. `register_round_host(caster, skill_id, HostSide::Enemy, step_index)`.
8. Push to round `steps`.

### Enemy AP (Action Points)

Enemy AP = number of alive defender entities. Each alive enemy gets exactly one action per round. There is no AP cost system for enemies — they always use all actions.

---

## Differences: Player vs Enemy

| Aspect | Player | Enemy |
|--------|--------|-------|
| Card source | `selected_cards` from client `BeginRoundOper` | `ai_use_cards` generated by `generate_ai_deck` |
| Skill resolution | Choice cards (param3), euphoria | Direct `skill_group1[0]`, no choice |
| Target resolution | Client-specified `to_id`, fallback to first alive | Random alive player hero (seeded RNG) |
| Execution context | Live `Fight` directly | Preview clone, then applied to live |
| Inline passives | Yes (`ActiveUseSkill` passives fire inline) | No inline passives during execution |
| EX skill handling | Detects EX, zeroes moxie, tracks `used_ex_skill` | No EX skill concept for enemies |
| AP consumption | `state.act_point -= 1` per non-temp card | No AP tracking; one action per alive enemy |
| Trigger expansion | Same | Same |
| Host assembly | Same | Same |

---

## State Mutation Detail: `play_step_data`

Located in `card_mgr.rs`. Called by `apply_step_and_maybe_sync` for every step (player and enemy).

Walks `step.act_effect` sequentially. For each effect:

```
match effect_type {
    Damage/Crit/AdditionalDamage/FixedDamage/OriginDamage/... →
        subtract effect_num from target's current_hp (floor at 0)
    
    Heal →
        add effect_num to target's current_hp (cap at max_hp)
    
    BuffAdd →
        buff_mgr.add_instance(target, buff_id, count, layer, from_uid, from_skill_id)
    
    BuffUpdate →
        buff_mgr.update_instance(buff_uid, new_count, new_layer)
    
    BuffDel →
        buff_mgr.remove_instance(buff_uid)
    
    ExPointChange (111) →
        ex_point_mgr.add_ex_point(target, delta)
        entity.ex_point = new_value
    
    BloodpoolValueChange →
        bloodtithe.add_value(team_type, delta)
    
    MaxHpChange →
        entity.max_hp = new_value
    
    CurrentHpChange →
        entity.current_hp = new_value (direct set, not delta)
}
```

After all effects: `sync_from_buff_mgr` reconciles buff-granted attributes, `sync_to_fight` writes EX point manager values back to entity fields.

---

## Trigger Chain Detail

`expand_trigger_chain` (`manager/round_mgr.rs`) orchestrates the reactive system after each step:

### TriggerEvent construction

Scans the step's `act_effect` list to populate:
- `damaged_uids` — entities that took any damage effect
- `cross_side_damaged_uids` — subset damaged by cross-side (enemy→player or player→enemy)
- `mental_damaged_uids` — subset damaged by Mental-type dealers
- `lost_expoint_uids` — entities with negative ExPointChange
- `dealer_uids` — entities that dealt damage
- `deleted_buff_ids` — buff IDs removed during this step
- `added_buff_uids` / `added_buff_ids` — buffs added during this step
- `nested_skill_uses` — sub-skill fires found in nested step payloads
- `bloodpool_gain_by_team` — bloodpool gains per team

### Pass execution order (via `TriggerPass` trait)

1. `CombatPassivesPass` — fires passive skills with combat-reactive conditions
2. `BuffFeatureReactivesPass` — fires buff features at `BeAttackedReactive`/`OnCast`/`PostSkill` stages
3. `HpSyncPass` — emits HP delta effects for entities changed by triggers
4. `BloodPoolSyncPass` — emits bloodpool delta effects
5. `BloodValueUseSkillPass` — fires skills gated on bloodpool thresholds
6. `ExPointSyncPass` — emits EX delta effects
7. `CardEnergySyncPass` — emits card energy updates

Each pass can produce `FightStep`s. Those steps are themselves applied (`play_step_data`), and if they cause further state changes, the chain recurses (with depth limiting via `SkillExecutor::call_depth`).

---

## Key Files

| File | Role |
|------|------|
| `phase/player_actions.rs` | Player card loop + host assembly |
| `phase/enemy_actions.rs` | Enemy card loop + host assembly |
| `manager/card_mgr.rs` | `play_card`, `execute_ai_turn`, `play_step_data` |
| `skill/executor.rs` | `SkillExecutor::execute_skill` — behavior dispatch |
| `trigger/combat.rs` | `expand_trigger_chain`, `TriggerEvent`, `skill_should_fire` |
| `trigger/passes/*.rs` | Individual trigger passes (HP sync, blood pool, etc.) |
| `event_queue.rs` | `HostEventAccumulator`, lane-based effect collection |
| `steps/trigger_embed.rs` | Converts trigger steps to embedded effects (type 162) |
| `steps/ex_gain.rs` | Pre-operation EX gain step construction |
| `fight_step/builder.rs` | `make_skill_step`, `ActEffectBuilder` |
| `card/deck.rs` | `generate_ai_deck`, `refill_hand` |
