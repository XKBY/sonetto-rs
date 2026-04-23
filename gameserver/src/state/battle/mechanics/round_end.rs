use sonettobuf::FightStep;

use crate::state::battle::buff_actions::use_skill_to_enemy::buff_get_use_skill_to_enemy_params;
use crate::state::battle::fight_step::{effect_container_step, wrap_step};
use crate::state::battle::{
    context::FightContext,
    mechanics::{injury_counter, magic_circle},
    passives::{collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill},
    round::step_shape::build_effect_step,
    skill::classification::has_injury_reactive_condition,
    skill::get_entity,
    skill::targets::collect_team,
    steps::trigger_embed,
    trigger::combat::{event_from_step, fire_combat_triggers},
    trigger::passes::build_belief_gain_step,
};

pub(crate) fn build_round_end_use_skill_to_enemy_steps(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
) -> Vec<FightStep> {
    let mut out = Vec::new();
    let holder_uids: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            ctx.fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter_map(|entity| entity.uid)
        .collect();

    for holder_uid in holder_uids {
        let Some(holder) = get_entity(ctx.fight, holder_uid) else {
            continue;
        };
        let team_type = holder
            .team_type
            .unwrap_or(if holder_uid > 0 { 1 } else { 2 });
        let target_uid = collect_team(ctx.fight, Some(if team_type == 1 { 2 } else { 1 }), false)
            .into_iter()
            .find(|uid| {
                get_entity(ctx.fight, *uid)
                    .map(|entity| entity.current_hp.unwrap_or(0) > 0)
                    .unwrap_or(false)
            })
            .unwrap_or(0);
        if target_uid == 0 {
            continue;
        }

        let holder_buffs = ctx.managers.buff_mgr.get(holder_uid).to_vec();
        for instance in holder_buffs {
            let Some((output_skill_id, _param)) =
                buff_get_use_skill_to_enemy_params(instance.buff_id)
            else {
                continue;
            };

            let phase = crate::state::battle::skill::PhaseFilter::combat_with(
                crate::state::battle::skill::TriggerState::default()
                    .with_buff_mgr(&ctx.managers.buff_mgr),
            );
            let Ok(mut skill_effects) =
                execute_passive_skill(ctx, holder_uid, target_uid, output_skill_id, &phase)
            else {
                continue;
            };
            if skill_effects.is_empty() {
                continue;
            }

            let preview_injuries = injury_counter::count_team_injury_effects_in_effects(
                ctx.fight,
                &skill_effects,
                team_type,
            );
            if let Some((cap, rate_per_stack)) = injury_counter::find_round_injury_skill_rate_params(
                ctx.fight,
                holder_uid,
                output_skill_id,
            ) {
                let battle_id = ctx.fight.battle_id.unwrap_or(0);
                let stacks = (injury_counter::get_round_injury_count(battle_id, team_type)
                    + preview_injuries)
                    .min(cap)
                    .max(0);
                if stacks > 0 {
                    injury_counter::apply_round_injury_skill_bonus(
                        ctx.fight,
                        &mut skill_effects,
                        holder_uid,
                        output_skill_id,
                        target_uid,
                        stacks,
                        rate_per_stack,
                    );
                }
            }
            let skill_event = if let Some(skill_step) =
                injury_counter::find_nested_skill_step_mut(&mut skill_effects, output_skill_id)
            {
                let circle_embeds = magic_circle::build_magic_circle_self_skill_embeds(
                    ctx,
                    &skill_step.clone(),
                    holder_uid,
                );
                if !circle_embeds.is_empty() {
                    let insert_at =
                        trigger_embed::find_trigger_insert_index(&skill_step.act_effect);
                    skill_step
                        .act_effect
                        .splice(insert_at..insert_at, circle_embeds);
                }
                Some(event_from_step(
                    ctx.fight,
                    skill_step.from_id.unwrap_or(0),
                    skill_step.to_id.unwrap_or(0),
                    skill_step.act_id.unwrap_or(0),
                    &skill_step.act_effect,
                ))
            } else {
                None
            };

            if let Some(skill_event) = skill_event {
                let injury_only = collected.filter(has_injury_reactive_condition);
                let trigger_steps = fire_combat_triggers(ctx, &injury_only, &skill_event);
                if !trigger_steps.is_empty()
                    && let Some(skill_step) = injury_counter::find_nested_skill_step_mut(
                        &mut skill_effects,
                        output_skill_id,
                    )
                {
                    for ts in trigger_steps {
                        let embedded = trigger_embed::trigger_step_to_embedded_effect(ts);
                        skill_step.act_effect.push(embedded);
                    }
                }
            }

            let inner =
                effect_container_step(holder_uid, holder_uid, instance.buff_id, skill_effects);
            let step = build_effect_step(vec![wrap_step(inner.clone())]);
            out.push(step);
            let event = event_from_step(
                ctx.fight,
                inner.from_id.unwrap_or(0),
                inner.to_id.unwrap_or(0),
                inner.act_id.unwrap_or(0),
                &inner.act_effect,
            );
            for &(belief_team_type, gain) in &event.bloodpool_gain_packets_by_team {
                if let Some(sync_step) = build_belief_gain_step(ctx.fight, belief_team_type, gain) {
                    out.push(sync_step);
                }
            }
        }
    }

    out
}
