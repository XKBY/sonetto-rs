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
//! 5. Graft any boss-side `BeAttacked` reactive on the host's damaged
//!    enemies.
//! 6. Inject card-host injury markers for the player carrying an
//!    injury counter.
//!
//! Mirrors `phase/enemy_actions.rs` but threaded for player-side
//! semantics (cloth power, ex gain, injury markers, channel
//! reactives).

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, BeginRoundOper, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
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
        magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
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
            let insert_at = step_walker::host_trigger_insert_index(&host_step);
            host_step
                .act_effect
                .splice(insert_at..insert_at, embedded_steps);
        }
        let monitor_embeds =
            channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
        if !monitor_embeds.is_empty() {
            let insert_at = step_walker::host_trigger_insert_index(&host_step);
            host_step
                .act_effect
                .splice(insert_at..insert_at, monitor_embeds);
        }
        trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
        trigger_embed::normalize_player_skill_effect_order(&mut host_step);
        mgr.graft_be_attacked_reactives_onto_player_host(state, &mut host_step, ctx);
        if let Some((holder_uid, injury_count)) =
            injury_counter::find_card_host_injury_marker_params(
                ctx.fight,
                host_step.from_id.unwrap_or(0),
            )
        {
            injury_counter::inject_card_host_injury_markers(
                &mut host_step,
                ctx.fight,
                holder_uid,
                injury_count,
            );
        }
        steps.push(host_step);
        state.is_finish = mgr.check_battle_end(ctx.fight);
        if state.is_finish {
            break;
        }
    }

    Ok(())
}
