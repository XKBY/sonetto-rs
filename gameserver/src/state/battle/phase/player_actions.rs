//! Player-actions phase: walk the operations the client recorded
//! for this round, apply each one, and shape the resulting steps.
//!
//! For each player operation:
//! 1. Compute the cloth-power delta and ex-gain prelude before the
//!    card resolves.
//! 2. Execute the operation and apply the resulting step to managers.
//! 3. Run `expand_trigger_chain` and either embed the trigger output
//!    into the host SKILL wrapper's nested-skill child or splice it
//!    at the host's trigger insert index.
//! 4. Append `MonitorContinueChannel` reactives surfaced from the
//!    player's buffs.
//! 5. Inject any boss-side `BeAttacked` reactive on the host's damaged
//!    enemies.
//! 6. Inject card-host injury markers for the player carrying an
//!    injury counter.
//!
//! Mirrors `phase/enemy_actions.rs` but threaded for player-side
//! semantics (cloth power, ex gain, injury markers, channel
//! reactives).

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, BeginRoundOper, FightStep, effect_type_enum::EffectType, fight_step};

use crate::state::battle::{
    context::FightContext,
    event_queue::{
        BattleEvent, HostEventAccumulator, HostLane, check_host_lane_membership,
    },
    manager::{
        card_mgr::FightCardMgr,
        round_mgr::{FightRoundMgr, active_cloth_level, cloth_power_delta_for_operation},
    },
    mechanics::{channel as channel_mechanics, injury_counter, magic_circle},
    passives::collector::CollectedPassives,
    round::RoundState,
    step_walker,
    steps::{ex_gain, trigger_embed},
    trigger::passes::sync_blood_value_baseline,
};

fn capture_inserted_host_children(
    accumulator: &mut HostEventAccumulator,
    lane: HostAccumulatorLane,
    before: &[ActEffect],
    after: &[ActEffect],
) {
    if after.len() <= before.len() {
        return;
    }

    let mut consumed_before = vec![false; before.len()];
    for effect in after {
        let matched_before = before
            .iter()
            .enumerate()
            .find(|(idx, before_effect)| !consumed_before[*idx] && *before_effect == effect);
        if let Some((idx, _)) = matched_before {
            consumed_before[idx] = true;
            continue;
        }
        push_host_accumulator_lane(accumulator, lane, effect.clone());
    }
}

#[derive(Debug, Clone, Copy)]
enum HostAccumulatorLane {
    Direct,
    TriggerLane,
}

fn push_host_accumulator_lane(
    accumulator: &mut HostEventAccumulator,
    lane: HostAccumulatorLane,
    effect: ActEffect,
) {
    let event = BattleEvent::SerializedActEffect { effect };
    match lane {
        HostAccumulatorLane::Direct => accumulator.push_direct(event),
        HostAccumulatorLane::TriggerLane => accumulator.push_trigger_lane(event),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    mgr: &FightRoundMgr,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    card_mgr: &mut FightCardMgr,
    state: &mut RoundState,
    operations: Vec<BeginRoundOper>,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
    sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
    let cloth = active_cloth_level(ctx.fight);
    for oper in operations {
        let cloth_power_delta = cloth
            .as_ref()
            .map(|cloth| cloth_power_delta_for_operation(&oper, cloth))
            .unwrap_or(0);
        let ex_step_after_op = ex_gain::pre_operation_ex_gain(ctx, state, &oper);
        let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
        let step = card_mgr.execute_operation(rng, ctx, state, oper).await?;
        if step.act_type.unwrap_or(0) == 0 {
            continue;
        }
        if cloth_power_delta != 0 {
            state.pending_cloth_power_delta = state
                .pending_cloth_power_delta
                .saturating_add(cloth_power_delta);
        }

        mgr.apply_step_and_maybe_sync(ctx, &step, true)?;
        let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
        let runtime_deleted_buff_ids =
            mgr.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

        let is_player_skill = step.act_type == Some(fight_step::ActType::Skill as i32)
            && step.from_id.unwrap_or(0) >= 0;
        if !is_player_skill {
            let expanded_steps =
                mgr.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
            steps.extend(expanded_steps);
            state.is_finish = mgr.check_battle_end(ctx.fight);
            if state.is_finish {
                break;
            }
            continue;
        }

        let suppress_pre_op_ex =
            ex_gain::skill_suppresses_pre_operation_ex(step.act_id.unwrap_or(0));
        if !suppress_pre_op_ex && let Some(ex_step) = ex_step_after_op.clone() {
            steps.push(ex_step);
        }
        let mut host_step = step.clone();
        step_walker::inline_magic_circle_root_wrapper(&mut host_step);
        let mut accumulator = HostEventAccumulator::new();
        for effect in host_step.act_effect.clone() {
            accumulator.push_direct(BattleEvent::SerializedActEffect { effect });
        }
        let host_children_before_magic_circle = host_step.act_effect.clone();
        magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
        capture_inserted_host_children(
            &mut accumulator,
            HostAccumulatorLane::Direct,
            &host_children_before_magic_circle,
            &host_step.act_effect,
        );
        let expanded_steps =
            mgr.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
        // Splice combat triggers as direct children of the host wrapper.
        // LIVE always attaches reactive passives at depth=1 under the host
        // skill wrapper — verified across battle1/2/3 fixtures (every player
        // skill cast has triggers as flat children, no `host_act_id - 20`
        // pre-embed pattern).
        //
        // The earlier branch here used `host_step.act_id - 20` as a
        // `preferred_nested_act_id` lookup with an `or_else(rposition)`
        // fallback that picked ANY non-host SKILL fightStep when the
        // preferred id was missing. The `-20` was a brittle heuristic from
        // the original `76ca5690 feat(battle): achieve live pcap parity`
        // commit that doesn't match any host in current fixtures (0/40
        // player skill casts across battle1+2+3), and the rposition
        // fallback caused triggers to be wrongly embedded inside the first
        // emitted self-passive (e.g. Sotheby's `1148002` battle-rule
        // wrapper) — Willow's `31040141` reactive ended up at depth=2
        // nested inside `1148002` instead of as a direct child of
        // `30090111` at depth=1.
        let mut embedded_steps: Vec<ActEffect> = Vec::new();
        for trigger_step in expanded_steps.into_iter().skip(1) {
            let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
            embedded_steps.push(embedded);
        }
        if !embedded_steps.is_empty() {
            let embedded_steps_for_accumulator = embedded_steps.clone();
            let insert_at = step_walker::host_trigger_insert_index(&host_step);
            host_step
                .act_effect
                .splice(insert_at..insert_at, embedded_steps);
            for effect in &embedded_steps_for_accumulator {
                push_host_accumulator_lane(
                    &mut accumulator,
                    HostAccumulatorLane::TriggerLane,
                    effect.clone(),
                );
            }
        }
        let monitor_embeds =
            channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
        if !monitor_embeds.is_empty() {
            let monitor_embeds_for_accumulator = monitor_embeds.clone();
            let insert_at = step_walker::host_trigger_insert_index(&host_step);
            host_step
                .act_effect
                .splice(insert_at..insert_at, monitor_embeds);
            for effect in &monitor_embeds_for_accumulator {
                push_host_accumulator_lane(
                    &mut accumulator,
                    HostAccumulatorLane::TriggerLane,
                    effect.clone(),
                );
            }
        }
        trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
        trigger_embed::normalize_player_skill_effect_order(&mut host_step);
        let be_attacked_lane_start = accumulator.lane_iter(HostLane::BeAttacked).count();
        let be_attacked_insert_at = mgr.inject_be_attacked_reactives_onto_player_host(
            state,
            &host_step,
            ctx,
            &mut accumulator,
        );
        if let Some(insert_at) = be_attacked_insert_at {
            let drained: Vec<ActEffect> = accumulator
                .lane_iter(HostLane::BeAttacked)
                .skip(be_attacked_lane_start)
                .cloned()
                .collect();
            host_step.act_effect.splice(insert_at..insert_at, drained);
        }
        if let Some((holder_uid, injury_count)) =
            injury_counter::find_card_host_injury_marker_params(
                ctx.fight,
                host_step.from_id.unwrap_or(0),
            )
        {
            let mut markers = injury_counter::inject_card_host_injury_markers(
                &host_step,
                ctx.fight,
                holder_uid,
                injury_count,
                &mut accumulator,
            );
            // Splice in reverse-index order so earlier positions don't shift.
            markers.sort_by(|(a, _), (b, _)| b.cmp(a));
            for (idx, marker) in markers {
                host_step.act_effect.insert(idx, marker);
            }
        }
        let (direct, trigger, be_attacked, injury) = accumulator.lane_counts();
        tracing::debug!(
            target: "phase5_accumulator",
            "host_step.act_effect.len()={} acc.total={} (direct={} trigger={} be_attacked={} injury={}) skill_id={} caster={}",
            host_step.act_effect.len(),
            accumulator.child_count(),
            direct,
            trigger,
            be_attacked,
            injury,
            host_step.act_id.unwrap_or(0),
            host_step.from_id.unwrap_or(0),
        );
        let has_magic_circle_add = host_step
            .act_effect
            .iter()
            .any(|effect| effect.effect_type == Some(EffectType::Magiccircleadd as i32));
        if !has_magic_circle_add {
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
            match check_host_lane_membership(&captured_effects, &host_step.act_effect) {
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
                            "lane={} count={} skill_id={} caster={}",
                            lane_name,
                            lane_count,
                            host_step.act_id.unwrap_or(0),
                            host_step.from_id.unwrap_or(0),
                        );
                    }
                    tracing::warn!(
                        target: "phase5_membership",
                        "lane membership diff: {} captured effects missing from host (skill_id={} caster={})",
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
        steps.push(host_step);
        state.is_finish = mgr.check_battle_end(ctx.fight);
        if state.is_finish {
            break;
        }
    }

    Ok(())
}
