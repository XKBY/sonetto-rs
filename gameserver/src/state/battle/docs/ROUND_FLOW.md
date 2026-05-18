# Round Computation Flow

## Entry Points

- `on_begin_round` (handlers/dungeon/begin_round.rs) — client sends card operations
- `on_auto_round` (handlers/dungeon/auto_round.rs) — auto-play variant

Both call `BattleSimulator::process_round`, which calls `FightRoundMgr::process_round_with_replay`.

---

## Top-Level: `process_round_with_replay`

```
1. sync_new_change_wave_snapshot   (replay only: apply wave snapshots to fight state)
2. phase::round_open::run          → RoundOpenPhaseData
3. dot_settle_round_start          (defender Poison settle before player acts)
4. phase::player_actions::run
5. phase::non_terminal_round::run
6. Post-processing cleanups        (merge reactives, nautika, coalesce, repair passes)
7. build_round_output              → FightRound
```

---

## Phase 1 — `round_open`

Initialises state for the round:

- Sync fight snapshot, buff managers, ex-point manager
- Apply cloth level power recovery
- Seed `ENTRY_MAX_HP` tracker
- Build `RoundState`:
  - `selected_cards` ← `player_hand` filtered to alive-hero cards (simulation deck)
  - `ai_cards` ← ai_deck
- Select cards for round-end (non-temp cards the player chose)
- Compute `defender_uid_checkpoint`; `CARDDECKNUM` = `player_deck.len()`
- Collect passives (`CollectedPassives`)

---

## Phase 2 — `player_actions`

For each `BeginRoundOper` from the client:

1. Pre-operation EX gain step
2. `card_mgr.execute_operation` → raw `FightStep`
3. `apply_card_upgrades` on player deck
4. `apply_step_and_maybe_sync` (updates fight state + buff manager)
5. `expand_trigger_chain` → reactive/trigger steps
6. For player skill hosts: splice combat triggers, be-attacked reactives, magic circle embeds, channel monitors into the host wrapper
7. Check `battle_end` after each operation; break if finished

---

## Phase 3 — `non_terminal_round`

Runs only when `state.is_finish == false`. Sequence:

```
1. emit AllocateCardEnergy(selected_for_round_end)   [played cards energy allocation]
2. Attacker passive sweep (post-player-actions, ExcludeBattleRule, FirstMatch)
3. Snapshot before_cards2 = state.player_deck (deck remaining after player actions)
4. refill_deck → appends to state.player_deck; drawn cards = team_a_cards2
   (stored on RoundOpenPhaseData for build_round_output)
5. build_pre_enemy_transition_steps:
     step A: AllocateCardEnergy(effect_num1=1) + RoundEnd + SmallRoundEnd
     step B: DealCard2  (if any cards were drawn)
6. Defender bootstrap passive sweep (AllMatches)
7. phase::enemy_actions::run
8. channel_mechanics::inject_channel_followup_buffs_if_missing
9. Defender passive sweep (defender side, AllMatches)
10. Defender round-end buff tick broadcast
11. DOT/HoT settle (apply each step to state before pushing)
12. round_end_emission::emit_round_end_steps  (round-end buff ticks, expirations)
13. Wave advancement check:
    - if all defenders dead and more waves remain → WaveCleared
    - advance wave, sync new defender snapshot, emit change_wave step
    - if no more waves → Victory
    - if all heroes dead → Defeat
14. Post-wave attacker passive sweep (round-start passives, battle rules)
15. Cloth power delta application
```

### `enemy_actions`

For each AI step from `card_mgr.execute_ai_turn`:

- Wave fast-forward if a defender from a future wave acts
- Pre-skill EX gain for each new enemy caster
- `apply_step_and_maybe_sync`
- `expand_trigger_chain` → reactive steps
- For enemy skill hosts: same trigger/magic-circle splice as player actions

---

## Phase 4 — Post-processing (after non_terminal_round)

Applied to `open.steps` before output:

| Pass | Purpose |
|---|---|
| `merge_post_turn_reactives_into_host` | Attach late reactive steps to their host |
| `strip_duplicate_change_round_markers` | Nautika dedup |
| `consolidate_into_bundle` | Nautika bundle |
| `strip_redundant_post_round_emissions` | Nautika cleanup |
| `coalesce_late_tail_exclude_battle_rule_passives` | Merge trailing passive emissions |
| `repair_boss_state_cycle_second_wave` | Boss state machine fix |
| `AttachmentResolver::apply` | Retro-attach standalone psychube wrappers |
| `repair_round_end_hedonism_emission` | Pickles hero fix |
| `repair_rubuska_round_end_heal_markers` | Rubuska hero fix |

---

## Phase 5 — `build_round_output`

Assembles the `FightRound` response:

1. Apply pending cloth power delta
2. Final `check_battle_end`
3. Sync ex-point manager back to fight
4. `round_ctx.on_round_end()` (increments round index)
5. Purge dead-hero cards from `state.player_deck`
6. Snapshot `before_cards1` = `state.player_deck` (post-purge, pre-refill)
7. `refill_deck` → appends to `state.player_deck`; drawn cards = `team_a_cards1`
8. Split steps by effect limit
9. Return `FightRound` + `state.player_deck` (next round's deck)

`state.player_deck` is the single source of truth throughout — no separate `next_round_cards`.

### `FightRound` card fields

| Field | Value |
|---|---|
| `before_cards1` | `state.player_deck` after dead-hero purge, before next-round refill |
| `team_a_cards1` | Cards drawn by the next-round refill |
| `before_cards2` | `state.player_deck` snapshot before Phase 3 refill (remaining after player actions) |
| `team_a_cards2` | Cards drawn by the Phase 3 refill |

---

## `AllocateCardEnergy` Wire Format

`AllocateCardEnergy` (effect type 276) is a **signal-only** effect on `ActEffect`. The client Lua reads only `effectNum1`:

- `effect_num1 = 1` → Allocate (trigger the card energy animation)
- `effect_num1 = 0` → Clear (skip)

**No card list is carried in the `ActEffect` itself.** The card data (`beforeCards1/2`, `teamACards1/2`) is carried in the `FightRound` proto fields, which the client reads separately via `FightHandCardDataMgr`.

Two `AllocateCardEnergy` emissions per round:
1. **After player actions** (step in `non_terminal_round` line 1): carries `selected_for_round_end` — the cards the player played. Uses `effect_num1=1`.
2. **In `build_pre_enemy_transition_steps`** (step A): signals the refill animation before enemy actions. Uses `effect_num1=1`.

The correct builder should be:
```rust
Self::bare(BattleEffectType::AllocateCardEnergy as i32)
    .effect_num1(1)
    .build()
```
No `card_info_list`, no `effect_num`.

---

## Key Data Structures

| Type | Role |
|---|---|
| `RoundState` | Mutable round state: `player_deck`, `ai_cards`, `is_finish`, `move_num`, `pending_cloth_power_delta` |
| `RoundOpenPhaseData` | Output of round_open: state + steps + collected passives + deck_num + checkpoints |
| `CollectedPassives` | Pre-collected passive skills for the round |
| `FightRound` | Wire response: steps, next deck, ex info, round number |
| `BattleEndState` | `Ongoing` / `WaveCleared` / `Victory` / `Defeat` |

---

## Card Deck Lifecycle Per Round

```
current_deck (from DB / previous round)
  → filter alive-hero cards → state.player_deck          [round_open]
  → mutations during player_actions (upgrades, temp cards added/consumed)
  → snapshot → before_cards2                             [non_terminal_round start]
  → refill_deck → team_a_cards2 appended to state.player_deck
  → enemy actions run against full deck
  → purge_dead_hero_cards                                [build_round_output]
  → snapshot → before_cards1
  → refill_deck → team_a_cards1 appended to state.player_deck
  → state.player_deck returned as next current_deck
```

`card_limit`: 1 hero→4, 2→5, 3→6, 4→8, N→min(N+4, 9)
