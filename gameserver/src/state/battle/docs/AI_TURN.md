# AI Turn

How enemy skill casts are computed each round — from `RoundState.ai_use_cards` to a list of `FightStep`s.

---

## Entry Point

`phase::enemy_actions::run` calls `ai_turn::execute_ai_turn`, which dispatches to one of two paths:

```
state.ai_override_steps.is_some()
  → execute_ai_turn_replay   (replay mode: follow recorded steps)
  → execute_ai_turn_live     (live mode: derive casts from ai_use_cards)
```

---

## Live Mode — `execute_ai_turn_live`

Iterates `state.ai_use_cards` in order. For each card:

1. Resolve target: `resolve_target_fallback(preview_fight, card.target_uid)` — falls back to first alive hero if the stored target is dead.
2. Apply euphoria override: `resolve_with_euphoria(fight, caster_uid, skill_id)`.
3. Execute skill on the **preview** fight/managers/mechanics.
4. Apply any pending summons to both preview and live fight.
5. `normalize_skill_effects_for_operation` — unwrap inline root SKILL wrappers.
6. `clamp_ai_add_ex_with_max_effects` — cap `AddExPoint`/`ExPointChange` effects to the caster's ex-point maximum.
7. Build `FightStep` via `make_skill_step`.
8. `advance_ai_preview_after_cast` — update preview ex-point state for subsequent casts.

---

## Replay Mode — `execute_ai_turn_replay`

Iterates `override_steps` (recorded from a previous run). For each step where `from_id < 0` and `act_id != 0`:

- Skip if caster is dead in the **preview** fight.
- If `to_id == 0`, pick a random alive player as target.
- Extract `replay_primary_damage_targets` from the recorded effects and pass them to `executor.set_override_damage_targets` so damage lands on the same targets as the original.
- Otherwise follows the same execute → normalize → clamp → advance pipeline as live mode.

Wave fast-forward applies in both modes: if the caster belongs to a future wave that hasn't been reached yet, `WaveMgr::fast_forward_*_to_wave` is called on the preview context before executing.

---

## Preview State

Both modes maintain a **preview** clone of `(fight, managers, mechanics)` that is mutated as casts execute. This lets each cast see the ex-point state left by the previous cast without touching the authoritative live state. The live state is only updated via `apply_summon_batch` (summons must appear in both).

---

## Key Helpers

| Function | Purpose |
|----------|---------|
| `normalize_skill_effects_for_operation` | Unwrap self-nested SKILL root wrappers |
| `clamp_ai_add_ex_with_max_effects` | Cap ex-gain effects to available headroom |
| `advance_ai_preview_after_cast` | Apply standard-action ex gain + effect ex deltas to preview |
| `apply_preview_ex_delta` | Mutate preview entity ex-point with overflow-buff awareness |
| `resolve_with_euphoria` | Substitute euphoria-overridden skill ID before execution |
| `resolve_target_fallback` | Fall back to first alive hero if stored target is dead |
