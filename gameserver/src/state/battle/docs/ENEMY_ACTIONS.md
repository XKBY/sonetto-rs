# Enemy Actions Phase

How AI-generated skill steps are applied to the round and shaped into wire-format `FightStep`s.

`phase/enemy_actions.rs` — called from `non_terminal_round` after the player-actions phase.

---

## Overview

```
enemy_actions::run
  1. sync_blood_value_baseline (both sides)
  2. ai_turn::execute_ai_turn  → Vec<FightStep>  (see AI_TURN.md)
  3. For each ai step:
       a. Wave fast-forward if caster belongs to a future wave
       b. Pre-skill EX gain step (standard-action ex for new negative-uid SKILL hosts)
       c. apply_step_and_maybe_sync + buff delta snapshot
       d. Route: non-host path OR host embedding pipeline
       e. register_round_host + push to steps
```

---

## Per-Step Processing

### Wave Fast-Forward

Before processing each step, if the caster's expected wave (`WaveMgr::expected_wave_for_uid`) is ahead of the current wave and not already covered by a replay snapshot, `WaveMgr::fast_forward_state_to_wave` is called on the **live** context. This ensures the fight state matches the wave the enemy belongs to.

### Pre-Skill EX Gain

For steps where `act_type == Skill` and `from_id < 0` (enemy SKILL host):
- If this is a new `(caster_uid, skill_id)` pair (or not the repeated `114300811` skill), emit a standard-action EX gain step via `ex_gain::standard_action_ex_gain_for_uid` and push it before the skill step.
- The caster uid is recorded in `state.enemy_skill_actors`.

### Apply Step and Buff Delta

`mgr.apply_step_and_maybe_sync(ctx, &step, true)` applies the step to the live fight state and syncs managers. A buff snapshot is taken before and after to compute `runtime_deleted_buff_ids` — these feed `expand_trigger_chain` so expired-buff triggers fire correctly.

---

## Routing: Non-Host vs Host

### Non-Host Path

Steps that are **not** an embedded SKILL host (i.e. `from_id >= 0` or not `ActType::Skill`) go through the simple path:

```
expand_trigger_chain(ctx, collected, &step, &deleted_buff_ids)
  → push all expanded steps directly to output
```

### Host Embedding Pipeline

Steps where `act_type == Skill` and `from_id >= 0` (positive-uid embedded skill host — rare for enemies but possible) go through the full embedding pipeline, matching the player-actions host path:

1. `inline_magic_circle_root_wrapper` — pre-embed magic circle root.
2. Build `HostEventAccumulator`; push all direct effects.
3. `apply_magic_circle_self_skill_embeds_with_accumulator` — embed magic circle sub-skills.
4. `expand_trigger_chain` — collect trigger steps.
5. Splice trigger steps as direct children of the host wrapper at `trigger_offset`.
6. `flatten_self_nested_skill_effects_v` — collapse self-nested SKILL wrappers.
7. `normalize_player_skill_effect_order_v` — reorder effects to match wire format.
8. Lane membership debug assert (warns + `debug_assert` if captured effects are missing from host).

> Note: enemy SKILL hosts with `from_id < 0` (the common case) take the **non-host path** — `expand_trigger_chain` emits them top-level. The host embedding pipeline only applies when `from_id >= 0`.

---

## Round Host Registration

After each step (both paths), `event_queue::register_round_host` records `(from_id, act_id, HostSide::Enemy, step_index)` for cross-phase bookkeeping.

---

## Key Differences from Player Actions

| Aspect | Player actions | Enemy actions |
|--------|---------------|---------------|
| EX gain prelude | `pre_operation_ex_gain` (per operation) | `standard_action_ex_gain_for_uid` (per new SKILL host) |
| Host embedding | Always for player SKILL hosts | Only for `from_id >= 0` hosts (rare) |
| BeAttacked / Injury lanes | Injected into host | Not injected (no player-side reactives) |
| Wave fast-forward | Preview only (ai_turn) | Live context (`fast_forward_state_to_wave`) |
| Trigger chain | Nested inside host wrapper | Top-level for `from_id < 0` hosts |
