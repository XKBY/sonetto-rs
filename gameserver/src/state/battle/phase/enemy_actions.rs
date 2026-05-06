//! Enemy-actions phase: apply each enemy SkillEmit captured by the
//! AI deck to the round state and shape the resulting steps.
//!
//! For each AI step the phase:
//! 1. Sets up the per-action ex-gain prelude when the actor is a
//!    negative-uid SKILL caster.
//! 2. Applies the step to managers and snapshots the buff delta so
//!    deleted-buff ids feed `expand_trigger_chain`.
//! 3. Either lets `expand_trigger_chain` emit the result top-level
//!    (for non-host emissions) or routes the host through the
//!    magic-circle pre-embed + nested-skill embedding pipeline that
//!    matches the official client's nested-wrapper shape.
//!
//! Most of the per-action work calls back into `FightRoundMgr`'s
//! state-free helper methods. `mgr: &FightRoundMgr` is threaded in
//! purely for namespacing, since the unit struct holds no state.

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    event_queue::{self, BattleEvent, HostEventAccumulator, HostLane, HostSide},
    manager::{card_mgr::FightCardMgr, round_mgr::FightRoundMgr},
    mechanics::magic_circle,
    passives::collector::CollectedPassives,
    round::RoundState,
    step_walker,
    steps::{ex_gain, trigger_embed},
    trigger::passes::sync_blood_value_baseline,
};

pub(crate) async fn run(
    mgr: &FightRoundMgr,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    card_mgr: &mut FightCardMgr,
    state: &mut RoundState,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
    sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
    state.enemy_skill_actors.clear();
    let ai_steps = card_mgr.execute_ai_turn(rng, ctx, state).await?;
    for step in ai_steps {
        let pre_skill_ex_step = if step.act_type == Some(fight_step::ActType::Skill as i32)
            && let Some(caster_uid) = step.from_id
            && caster_uid < 0
        {
            state.enemy_skill_actors.insert(caster_uid);
            ex_gain::standard_action_ex_gain_for_uid(mgr, ctx, caster_uid)
        } else {
            None
        };
        if let Some(ex_step) = pre_skill_ex_step {
            steps.push(ex_step);
        }

        let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
        mgr.apply_step_and_maybe_sync(ctx, &step, true)?;
        let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
        let runtime_deleted_buff_ids =
            mgr.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
        let is_embedded_skill_host = step.act_type == Some(fight_step::ActType::Skill as i32)
            && step.from_id.unwrap_or(0) >= 0;
        if !is_embedded_skill_host {
            let expanded_steps =
                mgr.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
            steps.extend(expanded_steps);
            continue;
        }

        let mut host_step = step.clone();
        step_walker::inline_magic_circle_root_wrapper(&mut host_step);
        let mut accumulator = HostEventAccumulator::new();
        for effect in host_step.act_effect.clone() {
            accumulator.push_direct(BattleEvent::SerializedActEffect { effect });
        }
        let mc_kind = magic_circle::apply_magic_circle_self_skill_embeds_with_accumulator(
            ctx,
            &mut host_step,
            &mut accumulator,
        );
        let expanded_steps =
            mgr.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
        // Splice combat triggers as direct children of the host wrapper.
        // Same fix applied to `phase/player_actions.rs` in `4cdf572d` —
        // the `host_act_id - 20` heuristic was dead code (0/18 enemy
        // SKILL hosts across battle1/2/3 have a `host_id - 20` nested
        // wrapper) and the `or_else(rposition)` fallback was rerouting
        // boss-side reactive triggers into the wrong wrapper.
        let trigger_offset = accumulator.lane_iter(HostLane::Trigger).count();
        for trigger_step in expanded_steps.into_iter().skip(1) {
            let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
            accumulator.push_trigger_lane(BattleEvent::SerializedActEffect { effect: embedded });
        }
        accumulator.splice_lane_drain_into_host(
            HostLane::Trigger,
            trigger_offset,
            &mut host_step,
            |host| step_walker::host_trigger_insert_index(host),
        );
        host_step.act_effect = trigger_embed::flatten_self_nested_skill_effects_v(
            host_step.act_type,
            host_step.act_id,
            host_step.from_id,
            std::mem::take(&mut host_step.act_effect),
        );
        host_step.act_effect = trigger_embed::normalize_player_skill_effect_order_v(
            host_step.act_type,
            std::mem::take(&mut host_step.act_effect),
        );
        let mc_skipped = matches!(mc_kind, magic_circle::MagicCircleApplyKind::NestedPath);
        if !mc_skipped {
            let mut captured_effects: Vec<&ActEffect> = Vec::new();
            for effect in accumulator.iter_captured_act_effects() {
                if let Some(nested_step) = effect.fight_step.as_ref()
                    && nested_step.act_type == Some(fight_step::ActType::Skill as i32)
                    && nested_step.act_id == host_step.act_id
                {
                    captured_effects.extend(nested_step.act_effect.iter());
                } else {
                    captured_effects.push(effect);
                }
            }
            match event_queue::check_host_lane_membership(&captured_effects, &host_step.act_effect)
            {
                Ok(()) => {}
                Err(diff) => {
                    for (lane_name, lane) in [
                        ("direct", HostLane::Direct),
                        ("trigger", HostLane::Trigger),
                        ("be_attacked", HostLane::BeAttacked),
                        ("injury", HostLane::Injury),
                    ] {
                        let lane_count = accumulator.lane_iter(lane).count();
                        tracing::debug!(
                            target: "phase5_membership",
                            "lane={} count={} skill_id={} caster={} (enemy)",
                            lane_name,
                            lane_count,
                            host_step.act_id.unwrap_or(0),
                            host_step.from_id.unwrap_or(0),
                        );
                    }
                    tracing::warn!(
                        target: "phase5_membership",
                        "lane membership diff: {} captured effects missing from host (skill_id={} caster={} enemy=true)",
                        diff.missing_from_host.len(),
                        host_step.act_id.unwrap_or(0),
                        host_step.from_id.unwrap_or(0),
                    );
                    debug_assert!(
                        diff.missing_from_host.is_empty(),
                        "Phase 5 lane membership assertion: {} captured effects missing from host (skill_id={})",
                        diff.missing_from_host.len(),
                        host_step.act_id.unwrap_or(0),
                    );
                }
            }
        }
        event_queue::register_round_host(
            host_step.from_id.unwrap_or(0),
            host_step.act_id.unwrap_or(0),
            HostSide::Enemy,
            steps.len(),
        );
        steps.push(host_step);
    }
    Ok(())
}
