use anyhow::Result;
use rand::thread_rng;
use sonettobuf::{CardInfo, FightRound};

use crate::state::battle::{
    card::{purge_dead_hero_cards, refill_deck},
    context::RoundContext,
    manager::{ex_point_mgr::build_ex_point_info, round_mgr::{active_cloth_level, apply_cloth_power_delta, FightRoundMgr}},
    fight_step::split_step_by_effect_limit,
    utils::alive_hero_uids,
};
use crate::state::battle::round::steps::transitions::build_next_round_begin_step;
use super::round_open::RoundOpenPhaseData;

pub(crate) fn build_round_output(
    mgr: &FightRoundMgr,
    round_ctx: &mut RoundContext<'_, '_>,
    mut open: RoundOpenPhaseData,
    ai_deck: Vec<CardInfo>,
    candidate_pool: &[CardInfo],
) -> Result<(FightRound, Vec<CardInfo>)> {
    let ctx = &mut *round_ctx.fight_ctx;
    if open.state.pending_cloth_power_delta != 0
        && let Some(cloth) = active_cloth_level(ctx.fight)
    {
        apply_cloth_power_delta(ctx.fight, &cloth, open.state.pending_cloth_power_delta);
    }
    open.state.is_finish = mgr.check_battle_end(ctx.fight);

    crate::state::battle::manager::ex_point_mgr::sync_to_fight(ctx.fight, &ctx.managers.ex_point_mgr);
    round_ctx.on_round_end();
    let ctx = &mut *round_ctx.fight_ctx;
    let ex_point_info = build_ex_point_info(ctx.fight, &ctx.managers.ex_point_mgr);
    tracing::warn!("=== ROUND END ===");

    let skill_infos = ctx.managers.calculate_mgr.build_player_skills();
    let hero_sp_attributes = ctx
        .managers
        .calculate_mgr
        .build_hero_sp_attributes(ctx.fight);
    let power = ctx
        .fight
        .attacker
        .as_ref()
        .and_then(|a| a.power)
        .unwrap_or(0);

    // Purge cards belonging to dead heroes
    let alive_uids = alive_hero_uids(ctx.fight);
    // Purge dead-hero cards and collect CARDREMOVE steps
    let purge_steps = purge_dead_hero_cards(&mut open.state.player_deck, &alive_uids);
    open.steps.extend(purge_steps);
    let before_cards1 = open.state.player_deck.clone();
    let team_a_cards1 = refill_deck(
        &mut thread_rng(),
        &mut open.state.player_deck,
        candidate_pool,
        &alive_uids,
        0,
        ctx.fight,
    );

    let next_round_begin_step = build_next_round_begin_step(open.state.player_deck.clone(), open.deck_num);
    open.steps = open
        .steps
        .into_iter()
        .flat_map(split_step_by_effect_limit)
        .collect();

    let attacker_main_count = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.len() as i32)
        .unwrap_or(3);

    let result = (
        FightRound {
            fight_step: open.steps,
            act_point: Some(if open.state.is_finish { 0 } else { attacker_main_count }),
            is_finish: Some(open.state.is_finish),
            move_num: Some(open.state.move_num),
            ex_point_info,
            ai_use_cards: ai_deck,
            power: Some(power),
            skill_infos,
            before_cards1,
            team_a_cards1,
            before_cards2: open.state.before_cards2,
            team_a_cards2: open.state.team_a_cards2,
            next_round_begin_step,
            use_card_list: vec![],
            cur_round: Some(ctx.fight.cur_round.unwrap_or(1) + 1),
            hero_sp_attributes,
            last_change_hero_uid: Some(0),
        },
        open.state.player_deck,
    );
    Ok(result)
}