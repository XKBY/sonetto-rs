use std::collections::HashSet;

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    buff_actions::monitor_continue::buff_get_monitor_continue_channel_params,
    context::FightContext,
    fight_step::{effect_container_step, wrap_step},
    manager::round_mgr::FightRoundMgr,
    passives::steps::skill::execute_skill as execute_passive_skill,
    skill::{PhaseFilter, TriggerState},
    steps::trigger_embed,
    trigger::combat::event_from_step,
    utils::buff_update,
};

/// Tracks entities that have MonitorContinueChannel passives.
/// Pre-built at battle start — zero runtime scanning.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChannelState {
    /// (uid, trigger_skill_id) for every ally with a MonitorContinueChannel passive.
    pub monitor_triggers: Vec<(i64, i32)>,
}

impl ChannelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        let cfg = config::configs::get();

        let entities = fight.attacker.iter().flat_map(|a| a.entitys.iter());

        for e in entities {
            let uid = e.uid.unwrap_or(0);

            for passive_id in &e.passive_skill {
                let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == *passive_id) else {
                    continue;
                };

                for beh in [
                    &skill.behavior1,
                    &skill.behavior2,
                    &skill.behavior3,
                    &skill.behavior4,
                    &skill.behavior5,
                ] {
                    if beh.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = beh.split('#').collect();

                    if !parts.first().map(|v| *v == "1").unwrap_or(false) {
                        continue;
                    }
                    let Some(buff_id) = parts.get(1).and_then(|v| v.parse::<i32>().ok()) else {
                        continue;
                    };

                    let is_attr_from_entity = cfg
                        .skill_buff
                        .iter()
                        .find(|b| b.id == buff_id)
                        .map(|b| {
                            b.features.split('|').any(|entry| {
                                let fp: Vec<&str> = entry.split('#').collect();
                                let act_id: i32 =
                                    fp.first().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                                cfg.buff_act
                                    .iter()
                                    .find(|a| a.id == act_id)
                                    .map(|a| a.r#type == "AttrFromEntity")
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);

                    if is_attr_from_entity {
                        self.monitor_triggers.push((uid, *passive_id));
                    }
                }
            }
        }
    }

    pub fn is_active(&self) -> bool {
        !self.monitor_triggers.is_empty()
    }
}

pub(crate) fn build_monitor_continue_channel_embeds(
    ctx: &mut FightContext<'_>,
    root_step: &FightStep,
    host_step: &FightStep,
) -> Vec<ActEffect> {
    let caster_uid = root_step.from_id.unwrap_or(0);
    if caster_uid <= 0 || root_step.act_type != Some(fight_step::ActType::Skill as i32) {
        return Vec::new();
    }

    let event = event_from_step(
        ctx.fight,
        caster_uid,
        root_step.to_id.unwrap_or(0),
        root_step.act_id.unwrap_or(0),
        &root_step.act_effect,
    );
    let target_uid = if root_step.to_id.unwrap_or(0) != 0
        && root_step.to_id.unwrap_or(0).signum() != caster_uid.signum()
    {
        root_step.to_id.unwrap_or(0)
    } else {
        event.primary_target_uid
    };
    if target_uid == 0 {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for instance in ctx.managers.buff_mgr.get(caster_uid).to_vec() {
        let Some((prerequisite_buff_id, _monitor_buff_id, emit_effect_id, emit_skill_id)) =
            buff_get_monitor_continue_channel_params(instance.buff_id)
        else {
            continue;
        };
        if prerequisite_buff_id > 0 && !ctx.managers.buff_mgr.has(caster_uid, prerequisite_buff_id)
        {
            continue;
        }
        if !seen.insert((emit_effect_id, emit_skill_id)) {
            continue;
        }
        let already_present = host_step.act_effect.iter().any(|effect| {
            effect
                .fight_step
                .as_ref()
                .map(|step| {
                    step.act_id == Some(emit_effect_id) || step.act_id == Some(emit_skill_id)
                })
                .unwrap_or(false)
        });
        if already_present {
            continue;
        }

        if let Some(existing) = ctx
            .managers
            .buff_mgr
            .get(caster_uid)
            .iter()
            .find(|b| b.buff_id == emit_effect_id)
        {
            let update_step = effect_container_step(
                caster_uid,
                caster_uid,
                emit_effect_id,
                vec![buff_update(
                    caster_uid,
                    existing.from_uid,
                    emit_effect_id,
                    existing.uid,
                    existing.stacks.max(1),
                    existing.layer,
                )],
            );
            if update_step.act_type == Some(fight_step::ActType::Effect as i32)
                && update_step.act_effect.len() == 1
                && let Some(effect) = update_step.act_effect.first().cloned()
                && effect.effect_type == Some(162)
            {
                out.push(effect);
            } else {
                out.push(wrap_step(update_step));
            }
        }

        let teammate_injury_hits = event
            .damaged_uids
            .iter()
            .filter(|&&d| d.signum() == caster_uid.signum())
            .count() as i32;
        let trigger_state = TriggerState {
            active_use_skill: true,
            skill_id: root_step.act_id.unwrap_or(0),
            action_order_index: 0,
            used_ex_skill: event.used_ex_skill,
            teammate_use_ex_skill: event.teammate_used_ex_skill(caster_uid),
            trigger_bullet: true,
            event_driven_only: false,
            be_attacked: event.was_attacked_by_enemy(caster_uid),
            hurt_not_restraint: event.dealt_damage(caster_uid),
            hurt_restraint: event.dealt_damage(caster_uid),
            teammate_injury_count: teammate_injury_hits,
            teammate_injury_count_not_reset: ctx
                .managers
                .buff_mgr
                .teammate_injury_not_reset(caster_uid),
            team_injury_count_round: teammate_injury_hits > 0,
            deleted_buff_ids: event.deleted_buff_ids.clone(),
            bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
            bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
        };

        let Ok(skill_effects) = execute_passive_skill(
            ctx,
            caster_uid,
            target_uid,
            emit_skill_id,
            &PhaseFilter::combat_with(trigger_state),
        ) else {
            continue;
        };
        out.extend(
            skill_effects
                .into_iter()
                .filter(|effect| effect.effect_type == Some(162)),
        );
    }

    out
}

pub(crate) fn inject_channel_followup_buffs_if_missing(
    mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
    collected: &crate::state::battle::passives::collector::CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> bool {
    if ctx.fight.cur_round.unwrap_or(1) != 1 {
        return false;
    }

    let attacker_uids: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .filter(|e| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0)
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    if attacker_uids.is_empty() {
        return false;
    }

    let cfg = config::configs::get();

    let mut channel_seed: Option<(i64, i32, i32, i32, i32)> = None;
    'find_seed: for uid in &attacker_uids {
        for instance in ctx.managers.buff_mgr.get(*uid) {
            let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id) else {
                continue;
            };
            for entry in buff_cfg.features.split('|') {
                let parts: Vec<&str> = entry.split('#').collect();
                let act_id = parts
                    .first()
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                let act_type = cfg
                    .buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type.as_str())
                    .unwrap_or("");
                if act_type != "ConsumeBuffContinueChannel" {
                    continue;
                }
                let Some(extra_skill_id) = parts
                    .get(1)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .filter(|v| *v > 0)
                else {
                    continue;
                };
                let target_type = parts
                    .get(3)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                let emit_effect_id = parts
                    .get(4)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                channel_seed = Some((
                    *uid,
                    instance.buff_id,
                    extra_skill_id,
                    target_type,
                    emit_effect_id,
                ));
                break 'find_seed;
            }
        }
    }
    let Some((caster_uid, channel_buff_id, extra_skill_id, target_type, emit_effect_id)) =
        channel_seed
    else {
        return false;
    };

    let selected_target_uid = mgr.first_alive_defender_uid(ctx.fight).unwrap_or(0);
    let target_uid = crate::state::battle::skill::targets::TargetResolver::new(
        ctx.fight,
        caster_uid,
        selected_target_uid,
    )
    .behavior(target_type)
    .resolve()
    .into_iter()
    .next()
    .or_else(|| (selected_target_uid != 0).then_some(selected_target_uid))
    .unwrap_or(caster_uid);

    let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
    let phase = crate::state::battle::skill::PhaseFilter::combat_with(
        crate::state::battle::skill::TriggerState::default().with_buff_mgr(&ctx.managers.buff_mgr),
    );
    let Ok(channel_effects) =
        execute_passive_skill(ctx, caster_uid, target_uid, extra_skill_id, &phase)
    else {
        return false;
    };
    if channel_effects.is_empty() {
        return false;
    }

    let mut channel_step =
        effect_container_step(caster_uid, caster_uid, channel_buff_id, channel_effects);
    if channel_buff_id == 31020114 && emit_effect_id > 0 {
        let bullet_embeds =
            build_display_only_consume_channel_embeds(ctx, caster_uid, target_uid, emit_effect_id);
        if !bullet_embeds.is_empty()
            && let Some(nested) = channel_step
                .act_effect
                .iter_mut()
                .find_map(|effect| effect.fight_step.as_mut())
        {
            let insert_at = trigger_embed::find_trigger_insert_index(&nested.act_effect);
            nested
                .act_effect
                .splice(insert_at..insert_at, bullet_embeds);
        }
    }

    if mgr
        .apply_step_and_maybe_sync(ctx, &channel_step, true)
        .is_err()
    {
        return false;
    }

    let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
    let runtime_deleted_buff_ids =
        mgr.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
    let trigger_steps: Vec<FightStep> = mgr
        .expand_trigger_chain(ctx, collected, &channel_step, &runtime_deleted_buff_ids)
        .into_iter()
        .skip(1)
        .filter(|step| trigger_embed::trigger_step_origin_uid(step) == Some(caster_uid))
        .collect();

    let mut host_step = channel_step.clone();
    let nested_skill_idx = host_step.act_effect.iter().position(|effect| {
        effect.effect_type
            == Some(crate::state::battle::types::effects::EffectType::FightStep as i32)
            && effect
                .fight_step
                .as_ref()
                .map(|step| step.act_id == Some(extra_skill_id))
                .unwrap_or(false)
    });

    if let Some(idx) = nested_skill_idx {
        if let Some(nested) = host_step
            .act_effect
            .get_mut(idx)
            .and_then(|effect| effect.fight_step.as_mut())
        {
            let lopera_channel = channel_buff_id == 31020114;
            let mut fallback_nested: Vec<ActEffect> = Vec::new();
            for trigger_step in trigger_steps {
                for embedded in explode_trigger_step_embeds(trigger_step) {
                    let duplicate_host = embedded
                        .fight_step
                        .as_ref()
                        .map(|step| {
                            step.act_type == Some(fight_step::ActType::Skill as i32)
                                && step.act_id == Some(extra_skill_id)
                                && step.from_id == Some(caster_uid)
                        })
                        .unwrap_or(false);
                    if duplicate_host {
                        continue;
                    }
                    if lopera_channel {
                        fallback_nested.push(embedded);
                        continue;
                    }
                    if !trigger_embed::insert_trigger_into_matching_nested(nested, embedded.clone())
                    {
                        fallback_nested.push(embedded);
                    }
                }
            }
            if !fallback_nested.is_empty() {
                if lopera_channel {
                    fallback_nested.sort_by_key(|effect| {
                        effect
                            .fight_step
                            .as_ref()
                            .map(|step| match step.act_id.unwrap_or(0) {
                                31020151 => 0,
                                433711 => 1,
                                _ => 2,
                            })
                            .unwrap_or(3)
                    });
                    nested.act_effect.extend(fallback_nested);
                } else {
                    let insert_at = trigger_embed::find_trigger_insert_index(&nested.act_effect);
                    nested
                        .act_effect
                        .splice(insert_at..insert_at, fallback_nested);
                }
            }
        }
    } else {
        let embedded_steps: Vec<ActEffect> = trigger_steps
            .into_iter()
            .map(trigger_embed::trigger_step_to_embedded_effect)
            .collect();
        if !embedded_steps.is_empty() {
            let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
            host_step
                .act_effect
                .splice(insert_at..insert_at, embedded_steps);
        }
    }

    steps.push(crate::state::battle::round::step_shape::build_effect_step(
        vec![wrap_step(host_step)],
    ));
    true
}

fn build_display_only_consume_channel_embeds(
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    emit_effect_id: i32,
) -> Vec<ActEffect> {
    let mut shadow_fight = ctx.fight.clone();
    let mut shadow_managers = ctx.managers.clone();
    let mut shadow_mechanics = ctx.mechanics.clone();
    let mut shadow_ctx = FightContext::new(
        &mut shadow_fight,
        &mut shadow_managers,
        &mut shadow_mechanics,
    );
    let phase = PhaseFilter::combat_with(
        TriggerState::on_active_use_skill(emit_effect_id)
            .with_buff_mgr(&shadow_ctx.managers.buff_mgr),
    );
    let Ok(skill_effects) = execute_passive_skill(
        &mut shadow_ctx,
        caster_uid,
        target_uid,
        emit_effect_id,
        &phase,
    ) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for effect in skill_effects {
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        if effect.effect_type
            != Some(crate::state::battle::types::effects::EffectType::FightStep as i32)
            || step.act_type != Some(fight_step::ActType::Skill as i32)
            || step.act_id != Some(emit_effect_id)
        {
            continue;
        }
        out.push(wrap_step(effect_container_step(
            caster_uid,
            caster_uid,
            emit_effect_id,
            step.act_effect.clone(),
        )));
    }

    if let Some(existing) = ctx
        .managers
        .buff_mgr
        .get(caster_uid)
        .iter()
        .find(|buff| buff.buff_id == emit_effect_id)
    {
        out.push(wrap_step(effect_container_step(
            caster_uid,
            caster_uid,
            emit_effect_id,
            vec![buff_update(
                caster_uid,
                existing.from_uid,
                emit_effect_id,
                existing.uid,
                existing.stacks.max(1),
                existing.layer,
            )],
        )));
    }

    out
}

fn explode_trigger_step_embeds(trigger_step: FightStep) -> Vec<ActEffect> {
    if trigger_step.act_type == Some(fight_step::ActType::Effect as i32) {
        let exploded: Vec<ActEffect> = trigger_step
            .act_effect
            .iter()
            .filter(|effect| effect.effect_type == Some(162))
            .cloned()
            .collect();
        if !exploded.is_empty() {
            return exploded;
        }
    }
    vec![trigger_embed::trigger_step_to_embedded_effect(trigger_step)]
}
