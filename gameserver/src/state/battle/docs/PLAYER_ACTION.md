# Player Action Handling

How a single player card-play travels from the network packet to the final `FightStep`.

---

## 1. Network Entry — `on_begin_round`

`handlers/dungeon/begin_round.rs`

- Decodes `BeginRoundRequest` (contains `Vec<BeginRoundOper>` — one per card played).
- Reads `player_hand`, `player_deck`, `ai_deck`, and `fight_data_mgr` from `ActiveBattle`.
- Calls `BattleSimulator::process_round(opers, &mut player_hand, &mut player_deck, ai_deck, None)`.
- Writes `player_hand` and `player_deck` back to `ActiveBattle` after the round.
- Sends `BeginRoundReply { round }` to the client.
- Saves raw operations to DB for replay (unless `is_replay`).

---

## 2. Simulator — `BattleSimulator::process_round`

`state/battle/simulator.rs`

Thin wrapper: forwards to `FightRoundMgr::process_round_with_replay` with no replay overrides.

---

## 3. Round Manager — `FightRoundMgr::process_round_with_replay`

`state/battle/manager/round_mgr.rs:532`

Orchestrates five phases in order:

```
phase::round_open::run
  → dot_settle_round_start
  → phase::player_actions::run      ← where each BeginRoundOper is executed
  → phase::non_terminal_round::run
  → post-processing + build_round_output
```

---

## 4. Phase: `round_open`

`state/battle/phase/round_open.rs`

Prepares `RoundState` before any operation runs:

- Filters `player_hand` to alive-hero cards → simulation deck for card selection.
- Simulates the card selection the client sent (`operations`) against the deck to determine which cards were chosen and in what order.
- Splits chosen cards into `selected_for_round_end` (non-temp first, then temp) — used later for `AllocateCardEnergy`.
- Sets `state.selected_cards` from the play-order selection (used by `card_mgr::play_card` to look up the card at each `op_index`).
- Sets `state.replay_selected_cards` / `state.replay_silent_ops` when in replay mode.
- Collects passives (`CollectedPassives`) for the round.
- Returns `RoundOpenPhaseData`.

---

## 5. Phase: `player_actions`

`state/battle/phase/player_actions.rs`

Iterates over `operations: Vec<BeginRoundOper>`. For each operation:

### 5a. Pre-operation bookkeeping
- Compute cloth-power delta for this operation (`cloth_power_delta_for_operation`).
- Compute pre-operation EX gain step (`ex_gain::pre_operation_ex_gain`).
- Snapshot buff state before the operation.

### 5b. Execute the operation
`card_mgr.execute_operation(rng, ctx, state, oper)` → `FightStep`

Dispatches on `oper.oper_type`:

| `CardOpType`            | Action                          |
|-------------------------|---------------------------------|
| `PlayCard`              | `play_card`                     |
| `MoveCard` (to_id ≠ 0)  | `play_card`                     |
| `AssistBoss`            | `play_card`                     |
| `PlayerFinisherSkill`   | `play_card`                     |
| `BloodPool`             | `play_card`                     |
| `SimulateDissolveCard`  | `dissolve_card` (no skill emit) |

### 5c. `card_mgr::play_card`

`state/battle/manager/card_mgr.rs:90`

**Card and caster resolution**

- `op_index = state.used_cards.len()` — sequential index into the selected-card list.
- Replay silent-op: if `replay_silent_ops[op_index]` is true, push `0` to `used_cards` and return a default step (no emission).
- Look up the card: `state.replay_selected_cards[op_index]` (replay) or `state.selected_cards[op_index]` (live).
- `exec_caster_uid`: use `card.uid` if non-zero; otherwise search attacker entities by `model_id` inferred from `skill_id / 10000` (temp/precast cards arrive with `uid=0`).

**Skill ID resolution**

- Choice cards have no behaviors in `SKILL_CACHE` → read chosen skill from `oper.param3`.
- Otherwise use `card.skill_id`.
- Apply Euphoria override: `resolve_with_euphoria(fight, caster_uid, skill_id)`.

**Target resolution**

- Use `oper.to_id` if non-zero (validated via `resolve_requested_target_uid`).
- Otherwise auto-target the first alive defender.

**Skill execution**

- Record emission timeline entry (`EmissionPhase::CardCast`).
- EX card prefix: if `resolved_skill_id == entity.ex_skill`, prepend `build_direct_ex_card_prefix` effects.
- `execute_skill_and_apply_pending_summons(caster, target, skill_id, PhaseFilter::combat_with(...))` → `Vec<ActEffect>`.
- Temp card fallback: if effects are empty, retry with `PhaseFilter::unconditional()` then `enter_fight()`, then `build_temp_direct_bigskill_fallback`.
- Normalize effects: `normalize_skill_effects_for_operation`.

**Inline passives (ActiveUseSkill)**

For each passive in `collected.merged_for(exec_caster_uid)`:
- Check `skill_should_fire` against a `TriggerEvent` built from this cast.
- Execute via `execute_passive_skill`; append non-empty effects to `skill_effects`.
- These become 162-wrapper children inside the card skill step's `actEffect`.

**Step construction** (`card_mgr.rs:343`)

- Push `op_index` to `state.used_cards`.
- Decrement `state.act_point` by 1 (skipped for temp cards).
- `make_skill_step(display_caster_uid, target_uid, resolved_skill_id, op_index, skill_effects)` → `FightStep`.
  - `from_id = display_caster_uid` (the wire caster; may be 0 for temp cards — `exec_caster_uid` was only used internally for execution).
  - `act_id = resolved_skill_id`.
  - `move_num = op_index`.
  - `act_effect = skill_effects` (main effects + inline passive 162-wrappers concatenated).

---

### 5d. Post-operation processing (back in `player_actions::run`)

**State application**

- Skip if `step.act_type == 0` (no-op / silent replay op).
- Accumulate `cloth_power_delta` onto `state.pending_cloth_power_delta`.
- `apply_step_and_maybe_sync`: run `calculate_mgr.play_step_data` (HP, buffs, ex-points), sync fight snapshot.
- Diff buff snapshots → `runtime_deleted_buff_ids`.

**Non-player-skill steps** (act_type ≠ SKILL or from_id < 0)

- `expand_trigger_chain` → append expanded steps.
- `drain_and_emit_dead_hero_purge`.
- Check `battle_end`; break if finished.

**Player-skill steps** (act_type == SKILL, from_id ≥ 0)

1. Push pre-op EX gain step (unless suppressed by `skill_suppresses_pre_operation_ex`).
2. `inline_magic_circle_root_wrapper` — flatten any magic-circle root wrapper into the host step.
3. Build `HostEventAccumulator`; push each `host_step.act_effect` as a direct-lane event.
4. `apply_magic_circle_self_skill_embeds_with_accumulator` — embed magic-circle self-skill effects.
5. `expand_trigger_chain(host_step)` → reactive/trigger steps.
6. **Trigger lane**: convert each trigger step (skip index 0, the host itself) to an embedded `ActEffect` via `trigger_step_to_embedded_effect`; splice into host at `host_trigger_insert_index`.
7. **Monitor lane**: `build_monitor_continue_channel_embeds` → splice after triggers at same insert index.
8. `flatten_self_nested_skill_effects_v` — collapse any self-nested SKILL wrappers.
9. `normalize_player_skill_effect_order_v` — reorder effects to match LIVE wire format.
10. **BeAttacked lane**: `inject_be_attacked_reactives_onto_player_host` → splice boss-side BeAttacked reactives at the computed insert position.
11. **Injury lane**: `find_card_host_injury_marker_params` + `inject_card_host_injury_markers` → insert injury markers at their exact indices (sorted reverse to avoid index shift).
12. Lane membership debug assert: all accumulator-captured effects must appear in `host_step.act_effect`.
13. `register_round_host(caster_uid, skill_id, HostSide::Player, step_index)`.
14. Push `host_step` to `steps`.
15. `drain_and_emit_dead_hero_purge` — remove dead-hero cards from `state.player_deck`, emit `RemoveEntityCards` steps.
16. Check `battle_end`; break if finished.

After all operations: snapshot `player_hand` → `before_cards2`; `refill_hand(player_hand, player_deck)` → `team_a_cards2`.

---

## 6. Phase: `non_terminal_round`

`state/battle/phase/non_terminal_round.rs`

Runs only when `state.is_finish == false`. Key steps relevant to player actions:

- Emits `AllocateCardEnergy(selected_for_round_end)` — the energy allocation for the cards the player played.
- Attacker passive sweep (post-player-actions).
- Enemy actions.
- DOT/HoT settlement, round-end buff ticks.

If `state.is_finish == true` at entry, delegates to `round_end_emission::emit_terminal_round_steps` instead.

---

## Key Types

| Type | Role |
|---|---|
| `BeginRoundOper` | One player action: `oper_type`, `from_id`, `to_id`, `param3` (choice), etc. |
| `RoundState` | Mutable round state: `selected_cards`, `used_cards`, `is_finish` |
| `FightCardMgr` | Executes card operations; owns `SkillExecutor` |
| `FightStep` | Wire representation of one action's output (skill wrapper + effects) |
| `HostEventAccumulator` | Collects direct/trigger/be-attacked/injury effects for a host step |
| `CollectedPassives` | Pre-collected passive skills for the round |
