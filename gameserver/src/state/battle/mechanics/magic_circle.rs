//! Magic-circle mechanic — config lookup helpers and runtime embeds.
//!
//! A magic circle is summoned by an ex skill (`skill_effect.isBigSkill == 1`)
//! that carries a `BehaviorType::AddMagicCircle { circle_id }` behavior slot.
//! The circle row in `data/excel2json/magic_circle.json` advertises its
//! carrier state buff (`selfBuff`) and the skill it fires while active
//! (`selfSkills`). All derivations go through config — no circle id, state
//! buff id, or self-skill id is hardcoded anywhere.

use config::magic_circle::MagicCircle;
use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    passives::steps::skill::execute_skill as execute_passive_skill,
    skill::{PhaseFilter, TriggerState},
    steps::trigger_embed,
    trigger::combat::event_from_step,
    types::effects::EffectType,
};

fn active_circle_row(fight: &sonettobuf::Fight) -> Option<&'static MagicCircle> {
    let circle = fight.magic_circle.as_ref()?;
    let circle_id = circle.magic_circle_id?;
    if circle.round.unwrap_or(0) == 0 {
        return None;
    }
    config::configs::get().magic_circle.get(circle_id)
}

/// Whether `skill_id` is listed as `selfSkills` on any magic circle.
/// Used by trigger/combat to exclude magic-circle emissions from team
/// bloodpool attribution.
pub fn is_magic_circle_self_skill(skill_id: i32) -> bool {
    config::configs::get()
        .magic_circle
        .iter()
        .filter_map(|c| c.self_skills.trim().parse::<i32>().ok())
        .any(|id| id == skill_id)
}

fn magic_circle_aura_state(
    ctx: &FightContext<'_>,
    host_step: &FightStep,
    host_caster_uid: i64,
) -> Option<(i32, i64, TriggerState)> {
    if host_caster_uid == 0 || host_step.act_type != Some(fight_step::ActType::Skill as i32) {
        return None;
    }

    let circle = ctx.fight.magic_circle.as_ref()?;
    let create_uid = circle.create_uid.unwrap_or(0);
    if create_uid == 0 || create_uid.signum() != host_caster_uid.signum() {
        return None;
    }

    let circle_cfg = active_circle_row(ctx.fight)?;
    let self_skill_id = circle_cfg.self_skills.trim().parse::<i32>().ok()?;
    if self_skill_id <= 0 || host_step.act_id == Some(self_skill_id) {
        return None;
    }
    let already_present = host_step.act_effect.iter().any(|effect| {
        effect
            .fight_step
            .as_ref()
            .map(|step| step.act_id == Some(self_skill_id))
            .unwrap_or(false)
    });
    if already_present {
        return None;
    }

    let event = event_from_step(
        ctx.fight,
        host_caster_uid,
        host_step.to_id.unwrap_or(0),
        host_step.act_id.unwrap_or(0),
        &host_step.act_effect,
    );
    let wrapper_host_offensive = host_step.act_type == Some(fight_step::ActType::Skill as i32)
        && host_step.to_id.unwrap_or(0) != 0
        && host_step.to_id.unwrap_or(0).signum() != host_caster_uid.signum()
        && direct_root_skill_child(host_step).is_some();
    if !event.dealt_damage(host_caster_uid) && !wrapper_host_offensive {
        return None;
    }

    let target_uid = if host_step.to_id.unwrap_or(0) != 0
        && host_step.to_id.unwrap_or(0).signum() != host_caster_uid.signum()
    {
        host_step.to_id.unwrap_or(0)
    } else {
        event.primary_target_uid
    };
    Some((
        self_skill_id,
        target_uid,
        TriggerState {
            active_use_skill: true,
            skill_id: host_step.act_id.unwrap_or(0),
            used_ex_skill: event.used_ex_skill,
            teammate_use_ex_skill: event.teammate_used_ex_skill(host_caster_uid),
            trigger_bullet: event.triggered_bullet_for(host_caster_uid),
            event_driven_only: false,
            be_attacked: event.took_damage(host_caster_uid),
            hurt_not_restraint: event.dealt_damage(host_caster_uid),
            hurt_restraint: event.dealt_damage(host_caster_uid),
            teammate_injury_count: event
                .damaged_uids
                .iter()
                .any(|&d| d.signum() == host_caster_uid.signum() && d != host_caster_uid),
            team_injury_count_round: event
                .damaged_uids
                .iter()
                .any(|&d| d.signum() == host_caster_uid.signum()),
            deleted_buff_ids: event.deleted_buff_ids.clone(),
        },
    ))
}

fn collect_add_passive_skill_ids(
    effects: &[ActEffect],
    host_caster_uid: i64,
    skip_skill_id: i32,
) -> Vec<i32> {
    fn walk(effects: &[ActEffect], out: &mut Vec<(i64, i32)>) {
        for effect in effects {
            if effect.effect_type == Some(EffectType::BuffAdd as i32)
                && let (Some(target_uid), Some(buff_id)) = (effect.target_id, effect.effect_num)
            {
                out.push((target_uid, buff_id));
            }
            if let Some(step) = effect.fight_step.as_ref() {
                walk(&step.act_effect, out);
            }
        }
    }

    let mut added_buffs = Vec::new();
    walk(effects, &mut added_buffs);

    let mut out = Vec::new();
    for (target_uid, buff_id) in added_buffs {
        if target_uid != host_caster_uid || buff_id <= 0 {
            continue;
        }
        crate::state::battle::utils::for_each_buff_feature_chain(buff_id, |act_type, parts| {
            if act_type != "AddPassiveSkills" {
                return;
            }
            for raw in parts.iter().skip(1) {
                for piece in raw.split(',') {
                    let Ok(skill_id) = piece.trim().parse::<i32>() else {
                        continue;
                    };
                    if skill_id > 0 && skill_id != skip_skill_id && !out.contains(&skill_id) {
                        out.push(skill_id);
                    }
                }
            }
        });
    }
    out
}

fn extend_with_active_add_passive_skill_ids(
    ctx: &FightContext<'_>,
    host_caster_uid: i64,
    skip_skill_id: i32,
    out: &mut Vec<i32>,
) {
    for instance in ctx.managers.buff_mgr.get(host_caster_uid) {
        crate::state::battle::utils::for_each_buff_feature_chain(
            instance.buff_id,
            |act_type, parts| {
                if act_type != "AddPassiveSkills" {
                    return;
                }
                for raw in parts.iter().skip(1) {
                    for piece in raw.split(',') {
                        let Ok(skill_id) = piece.trim().parse::<i32>() else {
                            continue;
                        };
                        if skill_id > 0 && skill_id != skip_skill_id && !out.contains(&skill_id) {
                            out.push(skill_id);
                        }
                    }
                }
            },
        );
    }
}

pub(crate) fn build_magic_circle_self_skill_embeds(
    ctx: &mut FightContext<'_>,
    host_step: &FightStep,
    host_caster_uid: i64,
) -> Vec<ActEffect> {
    let Some((self_skill_id, target_uid, trigger_state)) =
        magic_circle_aura_state(ctx, host_step, host_caster_uid)
    else {
        return Vec::new();
    };
    let phase = PhaseFilter::combat_with(trigger_state.clone());
    let Ok(skill_effects) =
        execute_passive_skill(ctx, host_caster_uid, target_uid, self_skill_id, &phase)
    else {
        return Vec::new();
    };
    let mut followup_skill_ids =
        collect_add_passive_skill_ids(&skill_effects, host_caster_uid, self_skill_id);
    extend_with_active_add_passive_skill_ids(
        ctx,
        host_caster_uid,
        self_skill_id,
        &mut followup_skill_ids,
    );

    let mut out: Vec<ActEffect> = skill_effects
        .into_iter()
        .filter(|effect| effect.effect_type == Some(162))
        .collect();

    for skill_id in followup_skill_ids {
        let Ok(skill_effects) =
            execute_passive_skill(ctx, host_caster_uid, target_uid, skill_id, &phase)
        else {
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

fn collect_last_nested_aura_target(
    ctx: &FightContext<'_>,
    effects: &[ActEffect],
    path_prefix: &mut Vec<usize>,
    best: &mut Option<Vec<usize>>,
) {
    for (idx, effect) in effects.iter().enumerate() {
        let Some(child) = effect.fight_step.as_ref() else {
            continue;
        };
        path_prefix.push(idx);
        if child.act_type == Some(fight_step::ActType::Skill as i32)
            && magic_circle_aura_state(ctx, child, child.from_id.unwrap_or(0)).is_some()
        {
            *best = Some(path_prefix.clone());
        }
        collect_last_nested_aura_target(ctx, &child.act_effect, path_prefix, best);
        path_prefix.pop();
    }
}

fn find_last_nested_aura_target(
    ctx: &FightContext<'_>,
    host_step: &FightStep,
) -> Option<Vec<usize>> {
    let add_idx = host_step
        .act_effect
        .iter()
        .position(|effect| effect.effect_type == Some(EffectType::MagicCircleAdd as i32))?;
    let mut best = None;
    let mut path = Vec::new();
    collect_last_nested_aura_target(
        ctx,
        &host_step.act_effect[add_idx + 1..],
        &mut path,
        &mut best,
    );
    best.map(|mut path| {
        if let Some(first) = path.first_mut() {
            *first += add_idx + 1;
        }
        path
    })
}

fn nested_step_ref_at_path<'a>(step: &'a FightStep, path: &[usize]) -> Option<&'a FightStep> {
    let mut current = step;
    for idx in path {
        current = current.act_effect.get(*idx)?.fight_step.as_ref()?;
    }
    Some(current)
}

fn nested_step_mut_at_path<'a>(
    step: &'a mut FightStep,
    path: &[usize],
) -> Option<&'a mut FightStep> {
    let mut current = step;
    for idx in path {
        current = current.act_effect.get_mut(*idx)?.fight_step.as_mut()?;
    }
    Some(current)
}

fn find_direct_root_skill_child(host_step: &FightStep) -> Option<usize> {
    let host_act_id = host_step.act_id?;
    let host_from = host_step.from_id?;
    host_step.act_effect.iter().position(|effect| {
        effect.effect_type == Some(162)
            && effect
                .fight_step
                .as_ref()
                .map(|child| {
                    child.act_type == Some(fight_step::ActType::Skill as i32)
                        && child.act_id == Some(host_act_id)
                        && child.from_id == Some(host_from)
                })
                .unwrap_or(false)
    })
}

fn direct_root_skill_child(host_step: &FightStep) -> Option<&FightStep> {
    host_step
        .act_effect
        .get(find_direct_root_skill_child(host_step)?)
        .and_then(|effect| effect.fight_step.as_ref())
}

pub(crate) fn apply_magic_circle_self_skill_embeds(
    ctx: &mut FightContext<'_>,
    host_step: &mut FightStep,
) {
    if let Some(path) = find_last_nested_aura_target(ctx, host_step)
        && let Some(target_snapshot) = nested_step_ref_at_path(host_step, &path).cloned()
    {
        let embeds = build_magic_circle_self_skill_embeds(
            ctx,
            &target_snapshot,
            target_snapshot.from_id.unwrap_or(0),
        );
        if !embeds.is_empty()
            && let Some(target_step) = nested_step_mut_at_path(host_step, &path)
        {
            let insert_at = trigger_embed::find_trigger_insert_index(&target_step.act_effect);
            target_step.act_effect.splice(insert_at..insert_at, embeds);
            return;
        }
    }

    let direct_root_snapshot = direct_root_skill_child(host_step).cloned();
    if let Some(target_snapshot) = direct_root_snapshot {
        let embeds = build_magic_circle_self_skill_embeds(
            ctx,
            &target_snapshot,
            target_snapshot.from_id.unwrap_or(0),
        );
        if !embeds.is_empty() {
            let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
            host_step.act_effect.splice(insert_at..insert_at, embeds);
            return;
        }
    }

    let embeds =
        build_magic_circle_self_skill_embeds(ctx, host_step, host_step.from_id.unwrap_or(0));
    if embeds.is_empty() {
        return;
    }
    let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
    host_step.act_effect.splice(insert_at..insert_at, embeds);
}
