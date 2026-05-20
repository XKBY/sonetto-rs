//! Round-open phase: card selection, deck setup, sync from `Fight`.
//!
//! This is the first phase the round-manager runs in `process_round`.
//! It seeds runtime managers (BuffMgr, ExPointMgr) from the `Fight`
//! snapshot, applies the cloth-power round-start delta, walks the
//! `BeginRoundOper` list to determine which cards the player played,
//! and produces the `RoundOpenPhaseData` payload that the rest of
//! the round phases consume.
//!
//! `FightRoundMgr` is a unit struct, so this used to be a `&self`
//! method on it — moving it to a free function and bundling the
//! payload struct alongside shrinks the round_mgr god-class without
//! threading state through the call.

use std::sync::Once;

use rand::{Rng, rngs::StdRng};
use sonettobuf::{BeginRoundOper, CardInfo, FightStep};

use sonettobuf::Fight;

use crate::state::battle::deck::DeckManager;
use crate::state::battle::{
    context::RoundContext,
    event_queue::reset_round_host_index,
    fight_step::FightStepBuilder,
    manager::{
        buff_mgr::{
            DEFENDER_BUFF_UID_START, attacker_buff_uid_checkpoint, defender_buff_uid_checkpoint,
            reset_buff_uid_to, sync_buff_uid_counters_from_mgr,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        entity_mgr::sync_from_fight,
        round_mgr::seed_entry_max_hp_from_fight,
    },
    mechanics::injury_counter,
    passives::collector::{CollectedPassives, collect},
    round::{RoundState, steps::refresh::build_refresh_step},
    card::{apply_card_upgrades, upgrade_level1},
};

fn ensure_battle_tracing() {
    static TRACE_INIT: Once = Once::new();

    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }

    TRACE_INIT.call_once(|| {
        let _ = std::panic::catch_unwind(common::init_tracing);
    });
}

/// Output payload of the round-open phase. Carries the seeded
/// `RoundState`, the initial step list (currently a single
/// `build_refresh_step`), the collected passives for this round, and
/// the per-round handles the rest of the phases reuse
/// (selected cards, deck cap, defender uid checkpoint).
pub(crate) struct RoundOpenPhaseData {
    pub state: RoundState,
    pub steps: Vec<FightStep>,
    pub collected: CollectedPassives,
    pub selected_for_round_end: Vec<CardInfo>,
    pub defender_uid_checkpoint: i64,
}

/// Run the round-open phase:
/// - Sync `RoundContext`, `BuffMgr`, `ExPointMgr` from the `Fight`.
/// - Apply cloth-power round-start seed + recover delta.
/// - Walk `BeginRoundOper` to derive selected/temp/non-temp/remaining
///   card splits.
/// - Build the round's initial refresh step.
/// - Collect attacker/defender passives.
pub(crate) fn run(
    round_ctx: &mut RoundContext<'_, '_>,
    rng: &mut StdRng,
    deck_mgr: &mut DeckManager,
    ai_override_steps: Option<&[FightStep]>,
    operations: &[BeginRoundOper],
    replay_selected_cards: Option<&[CardInfo]>,
    replay_silent_ops: Option<&[bool]>,
) -> RoundOpenPhaseData {
    ensure_battle_tracing();
    round_ctx.sync();
    tracing::warn!("process_round round_index={}", round_ctx.round_index);
    let ctx = &mut *round_ctx.fight_ctx;
    ctx.clear_round_active_card_casts();
    ctx.mechanics.emission_timeline.reset(round_ctx.round_index);
    reset_round_host_index();
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    injury_counter::sync_round_injury_index(battle_id, 1, round_ctx.round_index);
    injury_counter::sync_round_injury_index(battle_id, 2, round_ctx.round_index);
    seed_entry_max_hp_from_fight(ctx.fight);
    sync_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
    sync_buffs_from_fight(ctx.fight, &mut ctx.managers.buff_mgr);
    sync_buff_uid_counters_from_mgr(&ctx.managers.buff_mgr);

    if let Some(a) = &ctx.fight.attacker {
        for e in &a.entitys {
            tracing::warn!(
                "process_round ctx.fight uid={} hp={}",
                e.uid.unwrap_or(0),
                e.current_hp.unwrap_or(0)
            );
        }
    }

    let mut state = RoundState::new(ctx.fight);
    let attacker_uid_checkpoint = attacker_buff_uid_checkpoint();
    let mut defender_uid_checkpoint = defender_buff_uid_checkpoint();
    if defender_uid_checkpoint < DEFENDER_BUFF_UID_START {
        defender_uid_checkpoint = DEFENDER_BUFF_UID_START;
    }
    reset_buff_uid_to(attacker_uid_checkpoint.max(0));

    state.ai_override_steps = ai_override_steps.map(|steps| steps.to_vec());
    if let Some(cards) = replay_selected_cards {
        if !cards.is_empty() {
            state.replay_selected_cards = Some(cards.to_vec());
        }
    }
    if let Some(ops) = replay_silent_ops {
        if !ops.is_empty() {
            state.replay_silent_ops = Some(ops.to_vec());
        }
    }

    tracing::warn!("=== ROUND START ===");
    tracing::warn!("player_hand ({} cards):", deck_mgr.player_hand.len());
    for (i, c) in deck_mgr.player_hand.iter().enumerate() {
        tracing::warn!(
            "  [{}] uid={:?} hero={:?} skill={:?}",
            i,
            c.uid,
            c.hero_id,
            c.skill_id
        );
    }
    tracing::warn!(
        "player_hand after filter ({} cards):",
        deck_mgr.player_hand.iter().filter(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false)).count()
    );
    for (i, c) in deck_mgr.player_hand.iter().filter(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false)).enumerate() {
        tracing::warn!(
            "  [{}] uid={:?} hero={:?} skill={:?}",
            i,
            c.uid,
            c.hero_id,
            c.skill_id
        );
    }
    tracing::warn!("operations ({}):", operations.len());
    for (i, o) in operations.iter().enumerate() {
        tracing::warn!(
            "  [{}] type={:?} param1={:?} to_id={:?}",
            i,
            o.oper_type,
            o.param1,
            o.to_id
        );
    }

    deck_mgr.player_hand.retain(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false));
    let mut selected_pairs: Vec<(usize, sonettobuf::CardInfo)> = Vec::new();

    for op in operations {
        let op_type = op.oper_type.unwrap_or(0);
        let to_id = op.to_id.unwrap_or(0);
        let is_play = op_type == 2 || (op_type == 1 && to_id != 0);
        let is_move = op_type == 1 && to_id == 0;
        if is_move {
            let from = (op.param1.unwrap_or(1) - 1) as usize;
            let to = (op.param2.unwrap_or(1) - 1) as usize;
            tracing::warn!("  move idx={} -> idx={} (deck size {})", from, to, deck_mgr.player_hand.len());
            if from < deck_mgr.player_hand.len() && to < deck_mgr.player_hand.len() {
                let uid = deck_mgr.player_hand[from].uid.unwrap_or(0);
                if uid > 0 {
                    ctx.managers.entity_mgr.add_ex_point(uid, 1);
                }
                let card = deck_mgr.player_hand.remove(from);
                deck_mgr.player_hand.insert(to, card);
                let upgrades = apply_card_upgrades(&mut deck_mgr.player_hand, ctx.fight);
                for _ in 0..upgrades { ctx.on_compose_card(uid); }
            }
        } else if is_play {
            let idx = (op.param1.unwrap_or(1) - 1) as usize;
            tracing::warn!("  pick idx={} from deck of {} cards:", idx, deck_mgr.player_hand.len());
            for (i, c) in deck_mgr.player_hand.iter().enumerate() {
                tracing::warn!("    [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
            }
            if idx < deck_mgr.player_hand.len() {
                let card = deck_mgr.player_hand.remove(idx);
                tracing::warn!("  -> selected uid={:?} skill={:?}", card.uid, card.skill_id);
                let card_uid = card.uid.unwrap_or(0);
                selected_pairs.push((selected_pairs.len(), card));
                let upgrades = apply_card_upgrades(&mut deck_mgr.player_hand, ctx.fight);
                for _ in 0..upgrades { ctx.on_compose_card(card_uid); }
            } else {
                tracing::warn!(
                    "  -> idx {} OUT OF RANGE (deck size {})",
                    idx,
                    deck_mgr.player_hand.len()
                );
            }
        } else if op_type == 3 {
            let universal_idx = (op.param1.unwrap_or(1) - 1) as usize;
            let target_idx = (op.param2.unwrap_or(1) - 1) as usize;
            let is_universal = deck_mgr.player_hand.get(universal_idx)
                .and_then(|c| c.skill_id)
                .map_or(false, |id| id == 30000001);
            if is_universal && target_idx < deck_mgr.player_hand.len() {
                if let Some(next_skill) = upgrade_level1(&deck_mgr.player_hand[target_idx], ctx.fight) {
                    deck_mgr.player_hand[target_idx].skill_id = Some(next_skill);
                    deck_mgr.player_hand.remove(universal_idx);
                }
            }
            tracing::warn!("  upgrade idx={} with universal idx={} (is_universal={})", target_idx, universal_idx, is_universal);
        }
    }

    

    let selected_cards: Vec<sonettobuf::CardInfo> =
        selected_pairs.into_iter().map(|(_, c)| c).collect();

    // set to the deck after simulating all operations
    state.selected_cards = selected_cards.clone();

    let steps = vec![build_refresh_step(selected_cards.clone(), deck_mgr.player_hand.clone(), deck_mgr.player_deck.len() as i32)];

    let collected = collect(ctx.fight, ctx.fight.battle_id.unwrap_or(0));

    RoundOpenPhaseData {
        state,
        steps,
        collected,
        selected_for_round_end: selected_cards.clone(),
        defender_uid_checkpoint,
    }
}
