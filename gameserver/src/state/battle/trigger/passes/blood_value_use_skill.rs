use std::{collections::HashMap, sync::Mutex};

use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::{
    buff_actions::blood_value_use_skill::buff_get_blood_value_use_skill_params,
    context::FightContext,
    fight_step::{effect_container_step, wrap_step},
    passives::{collector::CollectedPassives, steps::skill::execute_skill},
    round::step_shape::build_effect_step,
    skill::PhaseFilter,
    steps::trigger_embed,
    trigger::combat::{TriggerEvent, event_from_step, fire_combat_triggers},
    types::effects::EffectType,
};

use super::TriggerPass;

static BLOOD_VALUE_ACCUM_TRACKER: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BLOOD_VALUE_BASELINE_TRACKER: Lazy<Mutex<HashMap<(i32, i32), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct BloodValueUseSkillPass;

pub fn sync_blood_value_baseline(battle_id: i32, team_type: i32, current_total: i32) {
    if battle_id == 0 || team_type <= 0 {
        return;
    }
    BLOOD_VALUE_BASELINE_TRACKER
        .lock()
        .unwrap()
        .insert((battle_id, team_type), current_total.max(0));
}

impl TriggerPass for BloodValueUseSkillPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        if battle_id == 0 || !ctx.mechanics.bloodtithe.initialized {
            return Vec::new();
        }

        let mut out = Vec::new();
        let holder_uids: Vec<i64> = collected
            .attacker_uids()
            .into_iter()
            .chain(collected.defender_uids())
            .collect();

        for holder_uid in holder_uids {
            let Some(holder) = crate::state::battle::skill::get_entity(ctx.fight, holder_uid)
            else {
                continue;
            };
            let team_type = holder
                .team_type
                .unwrap_or(if holder_uid > 0 { 1 } else { 2 });
            let delta = event.bloodpool_gain(team_type);
            if delta <= 0 {
                continue;
            }

            let mut holder_effects = Vec::new();
            let holder_buffs = ctx.managers.buff_mgr.get(holder_uid).to_vec();
            for instance in holder_buffs {
                let Some((carrier_buff_id, threshold, wrapper_skill_id)) =
                    buff_get_blood_value_use_skill_params(instance.buff_id)
                else {
                    continue;
                };
                if threshold <= 0 || wrapper_skill_id <= 0 {
                    continue;
                }
                if carrier_buff_id > 0
                    && instance.buff_id != carrier_buff_id
                    && !ctx.managers.buff_mgr.has(holder_uid, carrier_buff_id)
                {
                    continue;
                }

                let trigger_count = {
                    let baseline_total = BLOOD_VALUE_BASELINE_TRACKER
                        .lock()
                        .unwrap()
                        .get(&(battle_id, team_type))
                        .copied()
                        .unwrap_or(0);
                    let mut accum = BLOOD_VALUE_ACCUM_TRACKER.lock().unwrap();
                    let entry = accum
                        .entry((battle_id, instance.uid))
                        .or_insert_with(|| baseline_total.rem_euclid(threshold));
                    *entry += delta;
                    let count = (*entry / threshold).max(0);
                    if count > 0 {
                        *entry %= threshold;
                    }
                    count
                };
                if trigger_count <= 0 {
                    continue;
                }

                let target_uid = if event.primary_target_uid != 0
                    && event.primary_target_uid.signum() != holder_uid.signum()
                    && crate::state::battle::skill::get_entity(ctx.fight, event.primary_target_uid)
                        .map(|entity| entity.current_hp.unwrap_or(0) > 0)
                        .unwrap_or(false)
                {
                    event.primary_target_uid
                } else {
                    crate::state::battle::skill::targets::collect_team(
                        ctx.fight,
                        Some(team_type),
                        false,
                    )
                    .into_iter()
                    .find(|uid| {
                        crate::state::battle::skill::get_entity(ctx.fight, *uid)
                            .map(|entity| entity.current_hp.unwrap_or(0) > 0)
                            .unwrap_or(false)
                    })
                    .unwrap_or(holder_uid)
                };

                for _ in 0..trigger_count {
                    if let Ok(mut effects) = execute_skill(
                        ctx,
                        holder_uid,
                        target_uid,
                        wrapper_skill_id,
                        &PhaseFilter::combat_with(
                            ctx.active_use_trigger_state(wrapper_skill_id)
                                .with_buff_mgr(&ctx.managers.buff_mgr),
                        ),
                    ) {
                        maybe_inject_wrapped_bloodpool_gain(
                            ctx,
                            &mut effects,
                            wrapper_skill_id,
                            holder_uid,
                            team_type,
                        );
                        maybe_embed_nested_trigger_steps(
                            ctx,
                            collected,
                            &mut effects,
                            wrapper_skill_id,
                        );
                        if !effects.is_empty() {
                            holder_effects.push(wrap_step(effect_container_step(
                                holder_uid,
                                holder_uid,
                                instance.buff_id,
                                effects,
                            )));
                        }
                    }
                }
            }

            if !holder_effects.is_empty() {
                out.push(build_effect_step(holder_effects));
            }
        }

        out
    }
}

fn maybe_embed_nested_trigger_steps(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    effects: &mut [ActEffect],
    wrapper_skill_id: i32,
) {
    let Some(skill_step) = find_skill_step_mut(effects, wrapper_skill_id) else {
        return;
    };
    let skill_event = event_from_step(
        ctx.fight,
        skill_step.from_id.unwrap_or(0),
        skill_step.to_id.unwrap_or(0),
        skill_step.act_id.unwrap_or(0),
        &skill_step.act_effect,
    );
    let trigger_steps = fire_combat_triggers(ctx, collected, &skill_event);
    if trigger_steps.is_empty() {
        return;
    }
    let Some(skill_step) = find_skill_step_mut(effects, wrapper_skill_id) else {
        return;
    };
    for ts in trigger_steps {
        let embedded = trigger_embed::trigger_step_to_embedded_effect(ts);
        skill_step.act_effect.push(embedded);
    }
}

fn maybe_inject_wrapped_bloodpool_gain(
    ctx: &mut FightContext<'_>,
    effects: &mut [ActEffect],
    wrapper_skill_id: i32,
    holder_uid: i64,
    team_type: i32,
) {
    if wrapper_skill_id != 31200192 {
        return;
    }

    let Some(step) = find_skill_step_mut(effects, wrapper_skill_id) else {
        return;
    };
    let already_present = step.act_effect.iter().any(|effect| {
        effect.effect_type == Some(EffectType::BloodPoolValueChange as i32)
            && effect.target_id == Some(holder_uid)
            && effect.effect_num == Some(team_type)
            && effect.effect_num1 == Some(2)
    });
    if already_present {
        return;
    }

    let insert_at = step
        .act_effect
        .iter()
        .position(|effect| effect.effect_type == Some(EffectType::BuffAdd as i32))
        .unwrap_or(step.act_effect.len());
    step.act_effect.insert(
        insert_at,
        ActEffect {
            effect_type: Some(EffectType::BloodPoolValueChange as i32),
            target_id: Some(holder_uid),
            effect_num: Some(team_type),
            effect_num1: Some(2),
            ..Default::default()
        },
    );

    // TODO(event-queue): Phase 3 - route this BloodPoolValueChange emission
    // through EventQueue drain instead of inline mutation + effect insert.
    let current = ctx.mechanics.bloodtithe.get_value(team_type).max(0);
    let next = current + 2;
    if next > ctx.mechanics.bloodtithe.get_max(team_type) {
        ctx.mechanics.bloodtithe.set_max(team_type, next);
    }
    ctx.mechanics.bloodtithe.set_value(team_type, next);
}

fn find_skill_step_mut(
    effects: &mut [ActEffect],
    act_id: i32,
) -> Option<&mut sonettobuf::FightStep> {
    for effect in effects {
        if let Some(step) = effect.fight_step.as_mut() {
            if step.act_id == Some(act_id) {
                return Some(step);
            }
            if let Some(found) = find_skill_step_mut(&mut step.act_effect, act_id) {
                return Some(found);
            }
        }
    }
    None
}
