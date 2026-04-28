//! Round-end emission helpers split out of the `FightRoundMgr`
//! god-class.
//!
//! These functions assemble the FightStep stream that closes a round:
//! the bloodtithe transition steps, the `effect_type=276` ChangeRound
//! marker, the terminal attacker passive, and the round-end broadcast
//! (with a battle-rule-derived synthetic `530000112` fallback for
//! battles whose `addition_rule` references the boss state cycle
//! `530000151`).
//!
//! `FightRoundMgr` is a unit struct, so passing `mgr: &FightRoundMgr`
//! is a namespacing convention rather than threading state. Callers
//! still get to use the existing `apply_step_and_maybe_sync` and
//! `collect_battle_rule_skills` methods through the reference;
//! consolidating both in a free-function module shrinks the
//! `round_mgr.rs` god-class.

use anyhow::Result;
use sonettobuf::{ActEffect, CardInfo, FightStep};

use crate::state::battle::{
    context::FightContext,
    fight_step::{FightStepBuilder, wrap_step},
    manager::{
        buff_mgr::next_buff_uid_for_target,
        round_mgr::{FightRoundMgr, skill_has_no_act_round_condition},
        traits::Manager,
    },
    mechanics::bloodtithe,
    passives::{collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill},
    round::step_shape::build_effect_step,
    skill::PhaseFilter,
    steps::broadcast,
    utils::buff_update,
};

/// Emit the canonical terminal-round step stream:
/// 1. Bloodtithe round-transition steps (one per active bloodpool).
/// 2. The `effect_type=276` ChangeRound marker carrying
///    `selected_for_round_end`.
/// 3. The terminal attacker round-end passive (if one fires).
/// 4. The round-end broadcast (with synthetic `530000112` fallback
///    for `addition_rule`-derived `530000151` battles).
pub(crate) fn emit_terminal_round_steps(
    mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
    selected_for_round_end: Vec<CardInfo>,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    for step in bloodtithe::build_round_transition_bloodtithe_steps(mgr, ctx, collected) {
        mgr.apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    steps.push(
        FightStepBuilder::effect()
            .with(ActEffect {
                effect_type: Some(276),
                effect_num: Some(1),
                card_info_list: selected_for_round_end,
                ..Default::default()
            })
            .build(),
    );
    if let Some(raw_step) = build_terminal_attacker_round_end_passive_step(mgr, ctx, collected) {
        mgr.apply_step_and_maybe_sync(ctx, &raw_step, true)?;
        steps.push(build_effect_step(vec![wrap_step(raw_step)]));
    }

    let broadcast = collect_terminal_round_end_broadcast(mgr, ctx);
    if !broadcast.is_empty() {
        steps.push(build_effect_step(broadcast));
    }

    Ok(())
}

/// Snapshot one duration tick of buff state, walk the broadcast
/// collector, then restore. Used by both the per-round and terminal
/// round-end paths so the broadcast reflects the post-tick state
/// without committing the tick to live state.
///
/// TODO(event-queue): same snapshot/restore preview pattern as the
/// defender-side block at the call site. Migrate to a
/// `PreviewRoundEndTick` event when EventQueue Phase 5 covers
/// preview semantics.
pub(crate) fn collect_attacker_round_end_broadcast(
    ctx: &mut FightContext<'_>,
    injected_channel_buffs: bool,
    preview_round_end_tick: bool,
) -> Vec<ActEffect> {
    let mut broadcast = if preview_round_end_tick || ctx.fight.cur_round.unwrap_or(1) == 1 {
        let buff_snapshot = ctx.managers.buff_mgr.clone();
        ctx.managers.buff_mgr.on_round_end();
        let out = broadcast::collect_buff_tick_broadcast(ctx, true);
        ctx.managers.buff_mgr = buff_snapshot;
        out
    } else {
        broadcast::collect_buff_tick_broadcast(ctx, true)
    };
    broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, true);
    if injected_channel_buffs {
        broadcast::adjust_attacker_round1_broadcast_uids(&mut broadcast);
    }
    broadcast
}

/// Build the terminal-round attacker broadcast. If the natural
/// broadcast already contains the `530000112` boss-state-cycle buff,
/// return it unchanged. Otherwise, when the battle's
/// `addition_rule` references the `530000151` boss cycle skill,
/// synthesize one `530000112` BuffUpdate per alive attacker so the
/// shape matches what the official client emits at battle end.
pub(crate) fn collect_terminal_round_end_broadcast(
    mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
) -> Vec<ActEffect> {
    let broadcast = collect_attacker_round_end_broadcast(ctx, false, true);
    if broadcast.iter().any(|effect| {
        effect
            .buff
            .as_ref()
            .and_then(|buff| buff.buff_id)
            .unwrap_or(0)
            == 530000112
    }) {
        return broadcast;
    }

    if !mgr.collect_battle_rule_skills(ctx.fight).contains(&530000151) {
        return broadcast;
    }

    let mut synthesized = Vec::new();
    if let Some(attacker) = ctx.fight.attacker.as_ref() {
        for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
            if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
                continue;
            }
            let Some(uid) = entity.uid else { continue };
            let buff_uid = next_buff_uid_for_target(uid);
            let mut effect = buff_update(uid, -1, 530000112, buff_uid, 0, 0);
            if let Some(buff) = effect.buff.as_mut() {
                buff.duration = Some(1);
                buff.count = Some(0);
            }
            synthesized.push(effect);
        }
    }

    if synthesized.is_empty() {
        broadcast
    } else {
        synthesized
    }
}

/// Walk attacker-side passives and return the first one that emits
/// a non-empty effect set under the `NoActRound` condition (skill
/// behavior condition `46301`). Used by the terminal-round emitter
/// to surface the one ally passive that closes out the round.
pub(crate) fn build_terminal_attacker_round_end_passive_step(
    mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
) -> Option<FightStep> {
    let _ = mgr;
    let passive_phase = PhaseFilter::combat();
    let battle_rule_skills = mgr.collect_battle_rule_skills(ctx.fight);

    for uid in collected.attacker_uids() {
        for skill_id in collected.merged_for(uid) {
            if battle_rule_skills.contains(&skill_id)
                || !skill_has_no_act_round_condition(skill_id)
            {
                continue;
            }
            if let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                && !effects.is_empty()
            {
                return Some(build_effect_step(effects));
            }
        }
    }

    None
}
