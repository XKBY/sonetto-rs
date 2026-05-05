use anyhow::Result;
use sonettobuf::{ActEffect, Fight, FightHurtInfo, fight_hurt_info::DamageFromType};

use super::super::damage::calculate_damage;
use super::super::targets::get_entity;
use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::heroes::{nautika, rubuska, semmelweis};
use crate::state::battle::{
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    manager::{buff_mgr::BuffMgr as EventBuffMgr, ex_point_mgr::ExPointMgr as EventExPointMgr},
};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// BloodPool action — handler for the two `BehaviorType` variants
/// that mutate the bloodtithe pool directly via the
/// `BloodtitheState::pending_effects` queue:
/// `BloodPoolMaxChange { amount }` and `BloodPoolValueChange { amount }`.
/// Both delegate to the existing `pool_max_change` / `pool_value_change`
/// helpers in this module — the trait wrapper is purely the dispatch
/// route.
pub(super) struct BloodPool;

impl BehaviorAction for BloodPool {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::BloodPoolMaxChange { amount } => Some(Ok(pool_max_change(
                fight,
                &mut ctx.mechanics.bloodtithe,
                ctx.target,
                *amount,
            ))),
            BehaviorType::BloodPoolValueChange { amount } => Some(Ok(pool_value_change(
                fight,
                &mut ctx.mechanics.bloodtithe,
                ctx.target,
                *amount,
            ))),
            _ => None,
        }
    }
}
use crate::state::battle::manager::buff_mgr::BuffMgr;
use crate::state::battle::mechanics::bloodtithe::{
    BloodtitheState, bloodtithe_add_to_pool, bloodtithe_value_change,
};
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::apply_real_hurt_fix;

fn attr_value(entity: &sonettobuf::FightEntityInfo, attr_id: i32) -> i32 {
    let attr = entity.attr.as_ref();
    match attr_id {
        100 => entity.current_hp.unwrap_or(0),
        101 => attr.and_then(|a| a.hp).unwrap_or(0),
        102 => attr.and_then(|a| a.attack).unwrap_or(0),
        103 => attr.and_then(|a| a.defense).unwrap_or(0),
        _ => attr.and_then(|a| a.attack).unwrap_or(0),
    }
}

fn burn_params(buff_id: i32) -> Option<(i32, i32, i32)> {
    if buff_id <= 0 {
        return None;
    }
    let cfg = config::configs::get();
    let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for entry in buff_cfg.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
            continue;
        };
        if cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "Burn")
            .unwrap_or(false)
        {
            let rate = parts
                .get(1)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let attr_id = parts
                .get(2)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            return Some((act_id, rate, attr_id));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn lost_life(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    bloodtithe: &mut BloodtitheState,
    caster_uid: i64,
    target: i64,
    mode: i32,
    _attr_id: i32,
    permille: i32,
    behavior_id: i32,
    skill_id: i32,
    floor_permille: i32,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();

    let entity = get_entity(fight, target);
    let max_hp = entity
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let current_hp = entity.and_then(|e| e.current_hp).unwrap_or(0);
    let burn = (mode == 1).then(|| burn_params(skill_id)).flatten();
    let loss = if let Some((_, burn_rate, burn_attr_id)) = burn {
        let burn_rate = if burn_rate > 0 { burn_rate } else { permille };

        let from_damage = calculate_damage(
            fight, buff_mgr, None, caster_uid, target, burn_rate, skill_id, false,
        )
        .into_iter()
        .find(|e| {
            matches!(
                e.effect_type,
                Some(t)
                    if t == EffectType::Damage as i32
                        || t == EffectType::Crit as i32
                        || t == EffectType::OriginDamage as i32
                        || t == EffectType::OriginCrit as i32
            )
        })
        .and_then(|e| e.effect_num)
        .unwrap_or(0);

        if from_damage > 0 {
            from_damage
        } else {
            let source = get_entity(fight, caster_uid)
                .map(|e| attr_value(e, burn_attr_id))
                .unwrap_or(0);
            source * permille / 1000
        }
    } else if rubuska::is_basic_self_loss(skill_id) && target == caster_uid && _attr_id == 100 {
        // Rubuska's 31250111 family self-loss tracks the entry HP snapshot used by
        // Shadow Cloak, not the legacy 1%-of-current-max fallback.
        rubuska::basic_self_loss_amount(fight, target, permille)
    } else if mode == 1 {
        current_hp * permille / 1000
    } else {
        // Default LostLife lane uses basis-point style scaling
        // (e.g. value 800 -> 8% of max HP).
        max_hp * permille / 10000
    };

    let min_hp = if floor_permille > 0 {
        (max_hp * floor_permille / 1000).max(1)
    } else {
        0
    };
    let actual_loss = loss.min((current_hp - min_hp).max(0));

    if actual_loss == 0 {
        return effects;
    }

    if let Some((act_id, _, _)) = burn {
        let buff_uid = buff_mgr
            .get(target)
            .iter()
            .find(|b| b.buff_id == skill_id || b.type_id == skill_id)
            .map(|b| b.uid)
            .unwrap_or(0);

        effects.push(ActEffect {
            effect_type: Some(EffectType::Burn as i32),
            target_id: Some(target),
            effect_num: Some(skill_id),
            buff_act_id: (act_id > 0).then_some(act_id),
            ..Default::default()
        });

        effects.push(ActEffect {
            effect_type: Some(EffectType::OriginDamage as i32),
            target_id: Some(target),
            effect_num: Some(apply_real_hurt_fix(buff_mgr, target, actual_loss)),
            buff_act_id: (act_id > 0).then_some(act_id),
            hurt_info: Some(FightHurtInfo {
                damage: Some(apply_real_hurt_fix(buff_mgr, target, actual_loss)),
                reduce_hp: Some(0),
                reduce_shield: Some(0),
                career_restraint: Some(false),
                critical: Some(false),
                assassinate: Some(false),
                hurt_effect: Some(EffectType::OriginDamage as i32),
                damage_from_type: Some(DamageFromType::Buff as i32),
                config_effect: Some(0),
                buff_act_id: (act_id > 0).then_some(act_id),
                buff_uid: (buff_uid > 0).then_some(buff_uid as i32),
                effect_id: Some(0),
                skill_id: Some(0),
                from_uid: Some(caster_uid),
            }),
            ..Default::default()
        });
    } else {
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::Damage {
            target,
            amount: actual_loss,
            is_crit: false,
            hurt_info: FightHurtInfo {
                damage: Some(actual_loss),
                reduce_hp: Some(0),
                hurt_effect: Some(EffectType::Damage as i32),
                damage_from_type: Some(DamageFromType::SkillEffect as i32),
                config_effect: Some(behavior_id),
                effect_id: Some(skill_id),
                skill_id: Some(skill_id),
                from_uid: Some(caster_uid),
                ..Default::default()
            },
            from: caster_uid,
            skill_id: Some(skill_id),
        });

        let mut synthetic_fight = Fight::default();
        let mut synthetic_buff_mgr = EventBuffMgr::new();
        let mut synthetic_ex_point_mgr = EventExPointMgr::new();
        let drained = {
            let mut event_ctx = EventContext {
                fight: &mut synthetic_fight,
                buff_mgr: &mut synthetic_buff_mgr,
                ex_point_mgr: &mut synthetic_ex_point_mgr,
                bloodtithe,
            };
            drain_to_fight_steps(queue.drain(), &mut event_ctx)
        };
        effects.extend(drained);
    }

    let team_type = get_entity(fight, target).and_then(|e| e.team_type);

    let model_id = get_entity(fight, target).and_then(|e| e.model_id);

    // Battle2 parity: Semmelweis Ultimate body emits four visible 335 packets even when
    // our replay-seeded bloodpool cap is already saturated. Mirror the live packet lane
    // here and let replay-time 335 application advance the authoritative pool value.
    if skill_id == *semmelweis::TIER_IV_ULT_SKILL_ID {
        let manual_gain = semmelweis::ult_manual_gain(model_id);
        if manual_gain > 0 {
            effects.push(bloodtithe_add_to_pool(target, manual_gain));
            return effects;
        }
    }
    let preview_gain = team_type.and_then(|team_type| {
        let mut preview = bloodtithe.clone();
        preview.on_hp_lost(target, team_type, actual_loss)
    });
    if let Some(new_value) = preview_gain {
        if nautika::is_nautika(model_id) {
            effects.push(nautika::faith_gain_one(target));
        }
        effects.push(bloodtithe_add_to_pool(target, new_value));
    }

    effects
}

pub fn pool_max_change(
    _fight: &Fight,
    bloodtithe: &mut BloodtitheState,
    _target: i64,
    amount: i32,
) -> Vec<ActEffect> {
    let mut queue = EventQueue::new();
    queue.push(BattleEvent::BloodpoolMaxChange {
        team_type: 1,
        max: amount,
    });
    let mut synthetic_fight = Fight::default();
    let mut synthetic_buff_mgr = EventBuffMgr::new();
    let mut synthetic_ex_point_mgr = EventExPointMgr::new();
    let drained = {
        let mut event_ctx = EventContext {
            fight: &mut synthetic_fight,
            buff_mgr: &mut synthetic_buff_mgr,
            ex_point_mgr: &mut synthetic_ex_point_mgr,
            bloodtithe,
        };
        drain_to_fight_steps(queue.drain(), &mut event_ctx)
    };
    bloodtithe.pending_effects.extend(drained);
    vec![]
}

pub fn pool_value_change(
    fight: &Fight,
    bloodtithe: &mut BloodtitheState,
    target: i64,
    amount: i32,
) -> Vec<ActEffect> {
    bloodtithe.add_initial_gain(1, amount);

    let model_id = get_entity(fight, target).and_then(|e| e.model_id);
    if nautika::is_nautika(model_id) {
        bloodtithe
            .pending_effects
            .push(nautika::faith_gain_one(target));
    }
    bloodtithe
        .pending_effects
        .push(bloodtithe_value_change(target, amount, 1));
    vec![]
}
