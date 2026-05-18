# Battle Managers

Each manager owns a slice of battle state and exposes a focused API. They live inside `Managers` (fight_data_mgr.rs) and are passed via `FightContext`.

---

## `FightDataMgr` — top-level container

Owns the `Fight` proto and the `Managers` bundle. Entry point for all round processing.

- `build_initial_round` — constructs the first `FightRound` sent to the client
- `ctx()` / `ctx_with_rng()` — borrows fight + managers together as `FightContext`
- `seed_replay_*` — replay-mode helpers to fast-forward state from recorded effects

---

## `FightRoundMgr` — round orchestrator

Drives the per-round phase pipeline. Stateless (no owned fields); operates on `FightContext`.

- `process_round_with_replay` — runs all phases in order, returns `(FightRound, next_deck)`
- `build_round_output` — post-phase cleanup, computes `before_cards1/2`, `team_a_cards1/2`, next deck
- `apply_step_and_maybe_sync` — applies a single `FightStep` to fight state and syncs managers

See `ROUND_FLOW.md` for the full phase sequence.

---

## `CardMgr` — card execution

Executes player and AI card operations, resolves skills, and emits `FightStep`s.

- `execute_operation` — resolves a `BeginRoundOper` into a raw `FightStep`
- `execute_ai_turn` — runs the AI defender's card sequence for a wave
- `expand_trigger_chain` — resolves reactive/trigger steps after an operation

---

## `FightCalculateDataMgr` — stat calculation

Wraps `FightEntityDataMgr` + `BuffMgr` and owns pending effect queues. Applies step effects to fight state.

- `play_step_data` — applies all `ActEffect`s in a step, drains the event queue into new steps
- `build_player_skills` — builds `PlayerSkillInfo` list for the round response
- `build_hero_sp_attributes` — builds SP attribute snapshot for the round response

---

## `BuffMgr` — buff lifecycle

Owns all active `BuffInstance`s keyed by target UID. Handles add/update/remove with configurable refresh policies.

- `add` — adds or refreshes a buff according to its `RefreshPolicy`
- `get` / `find_instance_by_buff_id` / `has` — query active buffs
- `set_instance_duration` / `set_instance_layer` — mutate buff state
- `preview_round_end_lifecycle_takestage_103` — preview which buffs will expire at round end

---

## `ExPointMgr` — EX gauge + HP mirror

Tracks EX points and HP for every entity (attacker + defender). HP is mirrored here so damage/heal can be computed without mutating the `Fight` proto mid-step.

- `add_ex_point` / `consume_ex_point` / `set_ex_point` — EX gauge mutations
- `apply_damage` / `apply_heal` — HP mutations (clamped to max)
- `sync_from_fight` / `sync_to_fight` — bidirectional sync with the `Fight` proto

---

## `FightEntityDataMgr` — entity location cache

Caches `EntityLocation` (attacker/defender + index) for every entity UID. Rebuilt whenever the fight roster changes (wave advance, hero death).

- `rebuild_cache` — repopulates from current `Fight` state
- `get_location` — O(1) UID → location lookup
- `get_team_entities` — iterate all entities on one side

---

## `WaveMgr` — wave spawning

Manages defender wave progression and replay-mode fast-forward.

- `advance_wave` — kills current wave, spawns next, emits `ChangeWave` step
- `fast_forward_to_wave` — replay helper: spawns waves up to a target without emitting steps
- `expected_wave_for_uid` — infers which wave a UID belongs to (replay lookahead)

---

## `StageMgr` — (stub)

Reserved for stage/episode-level state. Not yet implemented.
