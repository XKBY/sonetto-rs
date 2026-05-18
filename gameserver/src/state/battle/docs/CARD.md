# card

Battle card system — manages the player's hand, draw logic, and card operations.

## Files

| File | Purpose |
|------|---------|
| `mod.rs` | Re-exports public API |
| `op.rs` | `CardOpType` enum — all card operation variants (play, move, blood pool, etc.) |
| `pool.rs` | `build_player_deck` — queries DB for a player's heroes and builds a finite draw pool (8 copies of skill1 + 8 copies of skill2 per hero = 16 cards/hero; trial heroes use a static UID map). `build_ai_pool` — same but from config for a list of monster IDs (no DB) |
| `draw.rs` | `draw_deck_guaranteed_by_uid_with_rng` — draws N cards from a pool, guaranteeing at least one card per required UID, avoiding adjacent same-skill cards, and mirroring first/last for odd-sized hands |
| `deck.rs` | `generate_initial_player_hand` / `generate_ai_deck` / `generate_ai_initial_deck` / `default_max_ap` — high-level deck construction: opening hand sizing, AI enemy card generation, and AP cap lookup. `refill_hand` — draws cards from `player_deck` (removing them) to top up the hand to the target size |
| `opening.rs` | `build_opening_deck` / `apply_opening_deck` — reconstructs the authoritative server-side hand from a `FightRound` by merging `team_a_cards1` with injected SP cards (effect type 78) from fight steps |

## Data flow

```
DB / game data
     │
     ├─ build_player_deck  ← hero UIDs (player)
     │        │  16 cards/hero (8×skill1 + 8×skill2)
     │        ▼
     │  draw_deck_guaranteed_by_uid_with_rng  ← draws opening hand from pool
     │        │
     │        ▼
     │  generate_initial_player_hand  → CardInfoPush  (sent to client)
     │
     └─ build_ai_pool  ← monster IDs (AI, config only)
              │
              ▼
        draw_deck_guaranteed_by_uid_with_rng
              │
              ▼
        generate_ai_initial_deck  → Vec<CardInfo>
```

`generate_ai_deck` bypasses the pool entirely — it builds cards directly from live defender entity state using a seeded RNG (used mid-battle for subsequent AI rounds).

## Opening hand rules

| Active heroes | Hand size |
|---------------|-----------|
| 1 | 4 |
| 2 | 5 |
| 3 (no support) | 6 |
| 3 (with support) | 7 |
| 4 | 8 |

For odd-sized hands the last card is forced to equal the first card (game rule).
