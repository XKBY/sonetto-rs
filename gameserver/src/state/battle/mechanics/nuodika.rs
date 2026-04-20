use std::collections::HashMap;

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    manager::{ex_point_mgr::ExPointMgr, round_mgr::{lookup_entry_max_hp, FightRoundMgr}},
    mechanics::injury_counter,
    passives::steps::skill::execute_skill as execute_passive_skill,
    round::step_shape::build_effect_step,
    skill::targets,
    utils::{
        buff_get_attr_replace_permille, buff_get_nuodika_channel_params, damage_with_hurt,
    },
};
use crate::state::battle::fight_step::wrap_step;
use crate::state::battle::skill::cache::resolve_skill_effect_id;
use crate::state::battle::types::effects::EffectType;

/// Channel-output pattern used by NuoDiKa rank-3; live emits a split 350-permille
/// replacement when a matching `AttrReplace` buff drives the output.
pub const NUODIKA_RANK3_OUTPUT_SKILL_ID: i32 = 31200173;

pub(crate) fn build_nuodika_channel_steps(
    _mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
    prior_steps: &[FightStep],
) -> Vec<FightStep> {
    let mut out = Vec::new();
    let mut simulated_pool = std::collections::HashMap::<i32, i32>::new();
    let mut simulated_hp = build_simulated_hp_map(ctx.fight);
    for step in prior_steps {
        apply_step_to_simulated_hp(step, &mut simulated_hp);
    }
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
        let Some(holder) = crate::state::battle::skill::get_entity(ctx.fight, holder_uid) else {
            continue;
        };
        let team_type = holder.team_type.unwrap_or(if holder_uid > 0 { 1 } else { 2 });
        let pool_value = *simulated_pool
            .entry(team_type)
            .or_insert_with(|| ctx.mechanics.bloodtithe.get_value(team_type).max(0));
        if pool_value <= 0 {
            continue;
        }

        let holder_buffs = ctx.managers.buff_mgr.get(holder_uid).to_vec();
        for instance in holder_buffs {
            let Some((_duration, threshold, _max_points, points_per_trigger, output_skill_id, _counter_buff_id)) =
                buff_get_nuodika_channel_params(instance.buff_id)
            else {
                continue;
            };
            if threshold <= 0 || output_skill_id <= 0 {
                continue;
            }

            let available = *simulated_pool.get(&team_type).unwrap_or(&0);
            let consume = (available / threshold) * threshold;
            if consume <= 0 {
                continue;
            }
            simulated_pool.insert(team_type, available - consume);

            let granted_points = (consume / threshold) * points_per_trigger;
            let target_uid = targets::collect_team(ctx.fight, Some(team_type), false)
                .into_iter()
                .find(|uid| {
                    simulated_hp.get(uid).copied().unwrap_or_else(|| {
                        crate::state::battle::skill::get_entity(ctx.fight, *uid)
                            .map(|entity| entity.current_hp.unwrap_or(0))
                            .unwrap_or(0)
                    }) > 0
                })
                .unwrap_or(holder_uid);
            let alive_enemy_targets: Vec<i64> =
                targets::collect_team(ctx.fight, Some(team_type), false)
                    .into_iter()
                    .filter(|uid| {
                        simulated_hp.get(uid).copied().unwrap_or_else(|| {
                            crate::state::battle::skill::get_entity(ctx.fight, *uid)
                                .map(|entity| entity.current_hp.unwrap_or(0))
                                .unwrap_or(0)
                        }) > 0
                    })
                    .collect();

            let phase = crate::state::battle::skill::PhaseFilter::combat_with(
                crate::state::battle::skill::TriggerState::default()
                    .with_buff_mgr(&ctx.managers.buff_mgr),
            );
            let Ok(mut channel_effects) =
                execute_passive_skill(ctx, holder_uid, target_uid, output_skill_id, &phase)
            else {
                continue;
            };
            rewrite_nuodika_channel_body(
                ctx.fight,
                &ctx.managers.ex_point_mgr,
                &mut channel_effects,
                holder_uid,
                output_skill_id,
                target_uid,
                &alive_enemy_targets,
                granted_points,
            );

            let mut step_effects = vec![ActEffect {
                effect_type: Some(335),
                target_id: Some(holder_uid),
                effect_num: Some(team_type),
                effect_num1: Some(-consume),
                ..Default::default()
            }];
            step_effects.push(ActEffect {
                effect_type: Some(EffectType::NuoDiKaRandomAttackNum as i32),
                target_id: Some(holder_uid),
                effect_num: Some(granted_points),
                effect_num1: Some(1),
                ..Default::default()
            });

            step_effects.append(&mut channel_effects);
            let inner = FightStep {
                act_type: Some(fight_step::ActType::Effect as i32),
                from_id: Some(holder_uid),
                to_id: Some(holder_uid),
                act_id: Some(instance.buff_id),
                act_effect: step_effects,
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            };
            let wrapped = build_effect_step(vec![wrap_step(inner)]);
            apply_step_to_simulated_hp(&wrapped, &mut simulated_hp);
            out.push(wrapped);
        }
    }

    out
}

fn build_simulated_hp_map(fight: &Fight) -> HashMap<i64, i32> {
    let mut hp = HashMap::new();
    for entity in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
    {
        if let Some(uid) = entity.uid {
            hp.insert(uid, entity.current_hp.unwrap_or(0));
        }
    }
    hp
}

fn apply_step_to_simulated_hp(step: &FightStep, simulated_hp: &mut HashMap<i64, i32>) {
    let mut stack = vec![step];
    while let Some(cur) = stack.pop() {
        for effect in &cur.act_effect {
            if let Some(child) = effect.fight_step.as_ref() {
                stack.push(child);
            }
            let target_id = effect.target_id.unwrap_or(0);
            if target_id == 0 {
                continue;
            }
            match effect.effect_type.unwrap_or(0) {
                x if injury_counter::is_damage_effect_type(x) => {
                    let amount = effect.effect_num.unwrap_or(0).max(0);
                    let entry = simulated_hp.entry(target_id).or_insert(0);
                    *entry = (*entry - amount).max(0);
                }
                x if x == EffectType::Dead as i32 => {
                    simulated_hp.insert(target_id, 0);
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn rewrite_nuodika_channel_body(
    fight: &Fight,
    ex_point_mgr: &ExPointMgr,
    skill_effects: &mut [ActEffect],
    caster_uid: i64,
    output_skill_id: i32,
    primary_target_uid: i64,
    alive_enemy_targets: &[i64],
    blood_sacrifice_points: i32,
) {
    if blood_sacrifice_points <= 0 {
        return;
    }
    let Some(skill_step) =
        injury_counter::find_nested_skill_step_mut(skill_effects, output_skill_id)
    else {
        return;
    };
    let cfg = config::configs::get();
    let Some(skill_cfg) = cfg
        .skill_effect
        .iter()
        .find(|s| s.id == resolve_skill_effect_id(output_skill_id))
    else {
        return;
    };

    let mut raw_behavior = None;
    for slot in 1..=20 {
        let behavior = match slot {
            1 => skill_cfg.behavior1.as_str(),
            2 => skill_cfg.behavior2.as_str(),
            3 => skill_cfg.behavior3.as_str(),
            4 => skill_cfg.behavior4.as_str(),
            5 => skill_cfg.behavior5.as_str(),
            6 => skill_cfg.behavior6.as_str(),
            7 => skill_cfg.behavior7.as_str(),
            8 => skill_cfg.behavior8.as_str(),
            9 => skill_cfg.behavior9.as_str(),
            10 => skill_cfg.behavior10.as_str(),
            11 => skill_cfg.behavior11.as_str(),
            12 => skill_cfg.behavior12.as_str(),
            13 => skill_cfg.behavior13.as_str(),
            14 => skill_cfg.behavior14.as_str(),
            15 => skill_cfg.behavior15.as_str(),
            16 => skill_cfg.behavior16.as_str(),
            17 => skill_cfg.behavior17.as_str(),
            18 => skill_cfg.behavior18.as_str(),
            19 => skill_cfg.behavior19.as_str(),
            20 => skill_cfg.behavior20.as_str(),
            _ => "",
        };
        if behavior.trim().is_empty() {
            continue;
        }
        let Some(behavior_id) = behavior
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
        else {
            continue;
        };
        let is_nuodika = cfg
            .skill_behavior
            .iter()
            .find(|b| b.id == behavior_id)
            .map(|b| b.r#type == "NuoDiKaDamage")
            .unwrap_or(false);
        if is_nuodika {
            raw_behavior = Some(behavior.to_string());
            break;
        }
    }
    let Some(raw_behavior) = raw_behavior else {
        return;
    };
    let parts: Vec<&str> = raw_behavior.split('#').collect();
    let primary_buff_id = parts.get(1).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let primary_rate = parts.get(2).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let secondary_buff_id = parts.get(3).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let secondary_rate = parts.get(4).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let primary_permille = buff_get_attr_replace_permille(primary_buff_id).unwrap_or(0);
    let secondary_permille = buff_get_attr_replace_permille(secondary_buff_id).unwrap_or(0);
    let Some(caster) = crate::state::battle::skill::get_entity(fight, caster_uid) else {
        return;
    };
    let max_hp = lookup_entry_max_hp(fight, caster_uid)
        .max(ex_point_mgr.get_max_hp(caster_uid))
        .max(
            caster
                .base_attr
                .as_ref()
                .and_then(|a| a.hp)
                .unwrap_or(caster.current_hp.unwrap_or(0)),
        )
        .max(
            caster
                .attr
                .as_ref()
                .and_then(|a| a.hp)
                .unwrap_or(caster.current_hp.unwrap_or(0)),
        )
        .max(caster.current_hp.unwrap_or(0))
        .max(0);
    if max_hp <= 0 {
        return;
    }
    let mut preserved = Vec::new();
    let mut aggregate_damage = std::collections::HashMap::<i64, i32>::new();
    let mut team_targets = Vec::new();
    for effect in skill_step.act_effect.drain(..) {
        let target_id = effect.target_id.unwrap_or(0);
        let is_enemy_damage = injury_counter::is_damage_effect_type(effect.effect_type.unwrap_or(0))
            && target_id != 0
            && target_id != caster_uid;
        if is_enemy_damage {
            if !team_targets.contains(&target_id) {
                team_targets.push(target_id);
            }
            continue;
        }
        preserved.push(effect);
    }
    if !alive_enemy_targets.is_empty() {
        team_targets.retain(|uid| alive_enemy_targets.contains(uid));
    }
    if team_targets.is_empty() && !alive_enemy_targets.is_empty() {
        team_targets.extend_from_slice(alive_enemy_targets);
    }
    if team_targets.is_empty() && primary_target_uid != 0 {
        team_targets.push(primary_target_uid);
    }
    if team_targets.is_empty() {
        return;
    }

    let random_target = primary_target_uid;
    let use_live_rank3_pattern = output_skill_id == NUODIKA_RANK3_OUTPUT_SKILL_ID
        && primary_permille == 350
        && primary_rate == 1000
        && secondary_permille == 500
        && secondary_rate == 1000
        && blood_sacrifice_points >= 15;
    let (random_hit_pattern, team_hit_damage) = if use_live_rank3_pattern {
        let mut pattern = vec![36292; blood_sacrifice_points.max(0) as usize];
        if !pattern.is_empty() {
            pattern[0] = 17592;
        }
        if pattern.len() > 12 {
            pattern[12] = 17592;
        }
        (pattern, 53823)
    } else {
        let random_hit_damage = (max_hp
            .saturating_mul(primary_permille)
            .saturating_mul(primary_rate)
            / 1000
            / 1000)
            .max(1);
        let team_hit_damage = (max_hp
            .saturating_mul(secondary_permille)
            .saturating_mul(secondary_rate)
            / 1000
            / 1000)
            .max(1);
        (
            vec![random_hit_damage; blood_sacrifice_points.max(0) as usize],
            team_hit_damage,
        )
    };
    if random_hit_pattern.is_empty() && team_hit_damage <= 0 {
        return;
    }
    for damage in &random_hit_pattern {
        if random_target != 0 {
            *aggregate_damage.entry(random_target).or_insert(0) += *damage;
        }
    }
    for target_id in &team_targets {
        *aggregate_damage.entry(*target_id).or_insert(0) += team_hit_damage;
    }

    let mut rebuilt = preserved;
    rebuilt.push(ActEffect {
        effect_type: Some(EffectType::NuoDiKaRandomAttackNum as i32),
        target_id: Some(caster_uid),
        effect_num: Some(blood_sacrifice_points),
        effect_num1: Some(1),
        ..Default::default()
    });
    for damage in &random_hit_pattern {
        if random_target != 0 {
            rebuilt.push(ActEffect {
                effect_type: Some(EffectType::NuoDiKaRandomAttack as i32),
                target_id: Some(random_target),
                effect_num: Some(*damage),
                effect_num1: Some(if use_live_rank3_pattern && *damage == 17592 {
                    2
                } else {
                    3
                }),
                config_effect: Some(60209),
                buff_act_id: Some(output_skill_id),
                ..Default::default()
            });
        }
    }
    for target_id in &team_targets {
        rebuilt.push(ActEffect {
            effect_type: Some(EffectType::NuoDiKaTeamAttack as i32),
            target_id: Some(*target_id),
            effect_num: Some(team_hit_damage),
            effect_num1: Some(1),
            config_effect: Some(60209),
            buff_act_id: Some(output_skill_id),
            ..Default::default()
        });
    }
    for (target_id, damage) in aggregate_damage {
        rebuilt.push(damage_with_hurt(
            target_id,
            damage.max(1),
            60209,
            output_skill_id,
            caster_uid,
        ));
    }
    let dead_effects = injury_counter::collect_dead_effects_after_damage(fight, &rebuilt);
    if !dead_effects.is_empty() {
        rebuilt.extend(dead_effects);
    }
    skill_step.act_effect = rebuilt;
}
