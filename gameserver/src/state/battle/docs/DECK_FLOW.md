# Player Deck — Functions & Data Flow

## State structs

| Struct | Field | Role |
|--------|-------|------|
| `ActiveBattle` | `player_hand: Vec<CardInfo>` | Cards currently in the player's hand; mutated each round |
| `ActiveBattle` | `player_deck: Vec<CardInfo>` | Finite draw pool (16 cards/hero); cards removed as drawn, never replenished |
| `ActiveBattle` | `act_point: i32` | Max AP stored for seeding `CardInfoPush` |
| `RoundState` | `act_point: i32` | Remaining AP budget; decremented per non-temp play |
| `RoundState` | `move_num: i32` | Move counter written to `FightRound::move_num` |
| `FightRound` | `before_cards1` | `player_hand` after dead-hero purge, before next-round refill |
| `FightRound` | `team_a_cards1` | Cards drawn by the next-round refill (in `build_round_output`) |
| `FightRound` | `before_cards2` | `player_hand` snapshot before Phase 3 refill |
| `FightRound` | `team_a_cards2` | Cards drawn by the Phase 3 refill |

---

## Battle start

```
handlers/dungeon/start_dungeon.rs  on_start_dungeon
  │
  ├─ card::generate_initial_player_hand(pool, user_id, fight_group, act_point)
  │     ├─ pool::build_player_deck(pool, user_id, hero_uids)
  │     │     └─ for each hero: DB lookup → Skill::get_skill_groups → make_card × 16
  │     └─ draw::draw_deck_guaranteed_by_uid_with_rng(candidates, hero_uids, hand_size, rng)
  │           rules: guarantee ≥1 card per hero, no adjacent same-skill, mirror first=last for odd hands
  │     → CardInfoPush { card_group, deal_card_group, act_point, move_num=0 }
  │
  ├─ pool::build_player_deck(pool, user_id, all_hero_uids)
  │     → player_deck: Vec<CardInfo>  (16 cards/hero, finite draw pool)
  │
  ├─ fight_data_mgr::build_initial_round(fight, player_hand)
  │     └─ FightRound { team_a_cards1 = player_hand, act_point, move_num=0 }
  │
  ├─ card::apply_opening_deck(round)
  │     └─ merges team_a_cards1 + SP cards injected by effect type 78 in fight steps
  │     → final_cards: Vec<CardInfo>  (authoritative server-side hand)
  │
  ├─ ActiveBattle.player_hand = final_cards
  ├─ ActiveBattle.player_deck = player_deck
  └─ send CardInfoPushCmd(CardInfoPush)
```

### Opening hand size rules

| Active heroes | Hand size |
|---------------|-----------|
| 1 | 4 |
| 2 | 5 |
| 3 (no support) | 6 |
| 3 (with support) | 7 |
| 4 | 8 |
| other | min(hero_count + 4, 9) |

---

## Round open

```
handlers/dungeon/begin_round.rs  on_begin_round
  │
  └─ BattleSimulator::process_round(&mut player_hand, &mut player_deck, ai_deck, opers, None)
        │
        └─ phase::round_open::run(player_hand, player_deck, opers)
              ├─ filter player_hand to alive-hero cards → sim_deck for simulation
              ├─ simulate BeginRoundOper list → selected_cards, remaining_hand
              └─ build_refresh_step → USECARDS(selected) + CARDSPUSH(remaining) + CARDDECKNUM(player_deck.len())
```

---

## Card operations (per play)

```
FightCardMgr::execute_operation(oper, state)
  │
  ├─ CardOpType::PlayCard
  │     └─ play_card(card_index, state)
  │           ├─ remove card from state.selected_cards[card_index]
  │           ├─ decrement state.act_point  (non-temp only)
  │           ├─ push to state.used_cards
  │           └─ execute skill
  │
  ├─ CardOpType::SimulateDissolveCard
  │     └─ dissolve_card(card_index, state)
  │           ├─ remove card from state.selected_cards[card_index]
  │           └─ emit cards_push effect with remaining hand
  │
  └─ CardOpType::MoveCard / MoveUniversal
        └─ reorder state.selected_cards
```

---

## Round end / carry-forward

```
phase::build_round_output(player_hand, player_deck, ...)
  │
  ├─ purge_dead_hero_cards(player_hand)
  ├─ before_cards1 = player_hand.clone()       (post-purge snapshot)
  ├─ refill_hand(rng, player_hand, player_deck, ...) → team_a_cards1
  │     draws from player_deck (removing cards), appends to player_hand
  └─ FightRound { before_cards1, team_a_cards1, before_cards2, team_a_cards2 }

player_hand and player_deck are written back to ActiveBattle after each round.
```

---

## Full lifecycle

```
on_start_dungeon
  └─ generate_initial_player_hand ──► CardInfoPush ──► client
  └─ build_player_deck ──► ActiveBattle.player_deck  (finite pool, shrinks each round)
  └─ apply_opening_deck ──► ActiveBattle.player_hand  (authoritative hand)

on_begin_round (each round)
  └─ process_round(&mut player_hand, &mut player_deck)
        └─ round_open: filter player_hand to alive-hero cards for simulation
        └─ execute_operation × N: mutate selected_cards (drawn from player_hand)
        └─ build_round_output: purge dead-hero cards, refill_hand from player_deck,
                               write FightRound fields, emit CARDSPUSH for next round
  └─ ActiveBattle.player_hand = updated hand
  └─ ActiveBattle.player_deck = updated pool (cards removed by refill_hand)
```
