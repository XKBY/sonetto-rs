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

use sonettobuf::{BeginRoundOper, CardInfo, FightStep};

use crate::state::battle::{
    context::RoundContext,
    manager::{
        buff_mgr::{
            DEFENDER_BUFF_UID_START, attacker_buff_uid_checkpoint, defender_buff_uid_checkpoint,
            reset_buff_uid_to, sync_buff_uid_counters_from_mgr,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        ex_point_mgr::sync_from_fight,
        round_mgr::{
            active_cloth_level, apply_cloth_power_delta, parse_cloth_recover_delta,
            seed_attacker_power_from_cloth, seed_entry_max_hp_from_fight,
        },
    },
    mechanics::injury_counter,
    passives::collector::{CollectedPassives, collect},
    round::{RoundState, steps::refresh::build_refresh_step},
};

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
    pub selected_non_temp: Vec<CardInfo>,
    pub deck_num: i32,
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
    current_deck: &[CardInfo],
    ai_deck: &[CardInfo],
    ai_override_steps: Option<&[FightStep]>,
    operations: &[BeginRoundOper],
    replay_selected_cards: Option<&[CardInfo]>,
    replay_silent_ops: Option<&[bool]>,
) -> RoundOpenPhaseData {
    round_ctx.sync();
    tracing::warn!("process_round round_index={}", round_ctx.round_index);
    let ctx = &mut *round_ctx.fight_ctx;
    ctx.clear_round_active_card_casts();
    ctx.mechanics.emission_timeline.reset(round_ctx.round_index);
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    injury_counter::sync_round_injury_index(battle_id, 1, round_ctx.round_index);
    injury_counter::sync_round_injury_index(battle_id, 2, round_ctx.round_index);
    seed_entry_max_hp_from_fight(ctx.fight);
    sync_from_fight(ctx.fight, &mut ctx.managers.ex_point_mgr);
    sync_buffs_from_fight(ctx.fight, &mut ctx.managers.buff_mgr);
    sync_buff_uid_counters_from_mgr(&ctx.managers.buff_mgr);
    if let Some(cloth) = active_cloth_level(ctx.fight) {
        seed_attacker_power_from_cloth(ctx.fight, &cloth);
        let recover_delta = parse_cloth_recover_delta(&cloth.recover, round_ctx.round_index);
        if recover_delta != 0 {
            apply_cloth_power_delta(ctx.fight, &cloth, recover_delta);
        }
    }

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

    state.player_deck = current_deck
        .iter()
        .filter(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false))
        .cloned()
        .collect();
    state.ai_cards = ai_deck.to_vec();
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
    tracing::warn!("current_deck ({} cards):", current_deck.len());
    for (i, c) in current_deck.iter().enumerate() {
        tracing::warn!(
            "  [{}] uid={:?} hero={:?} skill={:?}",
            i,
            c.uid,
            c.hero_id,
            c.skill_id
        );
    }
    tracing::warn!(
        "player_deck after filter ({} cards):",
        state.player_deck.len()
    );
    for (i, c) in state.player_deck.iter().enumerate() {
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

    let mut sim_deck = state.player_deck.clone();
    let mut selected_pairs: Vec<(usize, sonettobuf::CardInfo)> = Vec::new();

    tracing::warn!("=== CARD SELECTION ===");
    for op in operations {
        let op_type = op.oper_type.unwrap_or(0);
        let to_id = op.to_id.unwrap_or(0);
        let is_play = op_type == 2 || (op_type == 1 && to_id != 0);
        if is_play {
            let idx = (op.param1.unwrap_or(1) - 1) as usize;
            tracing::warn!("  pick idx={} from deck of {} cards:", idx, sim_deck.len());
            for (i, c) in sim_deck.iter().enumerate() {
                tracing::warn!("    [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
            }
            if idx < sim_deck.len() {
                let card = sim_deck.remove(idx);
                tracing::warn!("  -> selected uid={:?} skill={:?}", card.uid, card.skill_id);
                selected_pairs.push((selected_pairs.len(), card));
            } else {
                tracing::warn!(
                    "  -> idx {} OUT OF RANGE (deck size {})",
                    idx,
                    sim_deck.len()
                );
            }
        }
    }

    let selected_cards: Vec<sonettobuf::CardInfo> =
        selected_pairs.into_iter().map(|(_, c)| c).collect();
    let selected_temp: Vec<sonettobuf::CardInfo> = selected_cards
        .iter()
        .filter(|c| c.temp_card.unwrap_or(false))
        .cloned()
        .collect();
    let selected_non_temp: Vec<sonettobuf::CardInfo> = selected_cards
        .iter()
        .filter(|c| !c.temp_card.unwrap_or(false))
        .cloned()
        .collect();
    let mut selected_for_round_end = selected_non_temp.clone();
    selected_for_round_end.extend(selected_temp);
    let remaining_hand = sim_deck;

    tracing::warn!("=== RESULT ===");
    tracing::warn!("selected ({}):", selected_cards.len());
    for (i, c) in selected_cards.iter().enumerate() {
        tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
    }
    tracing::warn!("remaining ({}):", remaining_hand.len());
    for (i, c) in remaining_hand.iter().enumerate() {
        tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
    }

    let attacker_count = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.len())
        .unwrap_or(0);
    let deck_num = (attacker_count as i32) * 16;
    let steps = vec![build_refresh_step(selected_cards, remaining_hand, deck_num)];
    let collected = collect(ctx.fight, ctx.fight.battle_id.unwrap_or(0));

    RoundOpenPhaseData {
        state,
        steps,
        collected,
        selected_for_round_end,
        selected_non_temp,
        deck_num,
        defender_uid_checkpoint,
    }
}
