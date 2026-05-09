//! Damage action — handler for the plain damage family plus the
//! Sotheby-scoped `Detonate2` special-case.
//!
//! Each runs the same `lost_life::apply` core, then appends a preview
//! `Bloodpoolvaluechange` for any team whose bloodtithe pool has been
//! initialized. The preview is what mid-step lookups read before the
//! authoritative bloodtithe accumulator settles at round close.

use anyhow::Result;
use sonettobuf::{ActEffect, Fight};

use super::super::executor::SkillExecutor;
use super::super::targets::{get_ally_uids, get_entity};
use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::buff_actions::{EffectContext, lost_life};
use crate::state::battle::event_queue::{BattleEvent, serialize_leaf_event};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::manager::buff_mgr::BuffInstance;
use crate::state::battle::mechanics::Mechanics;
use crate::state::battle::mechanics::dot::parse_dot_features;
use crate::state::battle::skill::condition::buff::target_count_buffs_in_group;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::apply_real_hurt_fix;
use std::collections::HashMap;

/// Damage action — the single struct routed to from
/// `BehaviorType::Damage` in the dispatcher.
pub(super) struct Damage;

impl BehaviorAction for Damage {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::Damage { rate } => {
                let mut effect_ctx = EffectContext::new(
                    ctx.behavior_ctx.fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                );
                let mut effects = lost_life::apply(
                    &mut effect_ctx,
                    Some(&ctx.executor.pending_attr_bonus),
                    *rate,
                    ctx.skill_id,
                );
                append_preview_bloodtithe_gain_effects(
                    ctx.executor,
                    ctx.mechanics,
                    ctx.behavior_ctx.fight,
                    ctx.caster_uid,
                    ctx.target,
                    &mut effects,
                );
                Some(Ok(effects))
            }
            BehaviorType::Detonate2 {
                rate,
                granted_buff_id,
            } => {
                let effects = if is_sotheby_detonate2(ctx, *granted_buff_id) {
                    execute_sotheby_detonate2(ctx, *rate, *granted_buff_id)
                } else {
                    let mut effect_ctx = EffectContext::new(
                        ctx.behavior_ctx.fight,
                        ctx.managers,
                        ctx.mechanics,
                        ctx.caster_uid,
                        ctx.target,
                    );
                    lost_life::apply(
                        &mut effect_ctx,
                        Some(&ctx.executor.pending_attr_bonus),
                        *rate,
                        ctx.skill_id,
                    )
                };
                let mut effects = effects;
                append_preview_bloodtithe_gain_effects(
                    ctx.executor,
                    ctx.mechanics,
                    ctx.behavior_ctx.fight,
                    ctx.caster_uid,
                    ctx.target,
                    &mut effects,
                );
                Some(Ok(effects))
            }
            BehaviorType::OriginDamageByAttrAndBuffGroupSize {
                mode: _,
                attr_id,
                permille,
                group_id,
            } => Some(Ok(execute_origin_damage_by_attr_and_buff_group_size(
                ctx, *attr_id, *permille, *group_id,
            ))),
            _ => None,
        }
    }
}

const SOTHEBY_DETONATE2_SKILL_ID: i32 = 300901321;
pub(crate) const DUALITY_POTION_BUFF_ID: i32 = 30091120;
pub(crate) const CURE_TYPE_ID: i32 = 30091111;
const POISON_INSTANCE_BUFF_ID: i32 = 300901412;
const SOTHEBY_DETONATE2_RATE_NUMERATOR: i32 = 2;
const SOTHEBY_DETONATE2_RATE_DENOMINATOR: i32 = 3;

fn is_sotheby_detonate2(ctx: &ActionCtx<'_, '_>, granted_buff_id: i32) -> bool {
    ctx.skill_id == SOTHEBY_DETONATE2_SKILL_ID && granted_buff_id == CURE_TYPE_ID
}

fn execute_sotheby_detonate2(
    ctx: &mut ActionCtx<'_, '_>,
    rate: i32,
    granted_buff_id: i32,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();

    let scaled_rate =
        rate.saturating_mul(SOTHEBY_DETONATE2_RATE_NUMERATOR) / SOTHEBY_DETONATE2_RATE_DENOMINATOR;
    let mut effect_ctx = EffectContext::new(
        ctx.behavior_ctx.fight,
        ctx.managers,
        ctx.mechanics,
        ctx.caster_uid,
        ctx.target,
    );
    let damage_effects = lost_life::apply(
        &mut effect_ctx,
        Some(&ctx.executor.pending_attr_bonus),
        scaled_rate.max(0),
        ctx.skill_id,
    );
    let target_died = target_would_die(ctx.behavior_ctx.fight, ctx.target, &damage_effects);
    effects.extend(damage_effects);

    if target_died {
        effects.push(ActEffectBuilder::dead(ctx.target));
    } else if let Some(poison_damage) = detonate_target_poison_damage(ctx) {
        effects.push(ActEffectBuilder::origin_crit(
            ctx.target,
            poison_damage,
            None,
        ));
    }

    let duality = ctx
        .managers
        .buff_mgr
        .find_instance_by_buff_id(ctx.caster_uid, DUALITY_POTION_BUFF_ID)
        .cloned();
    if let Some(duality) = duality.as_ref() {
        effects.extend(build_sotheby_holder_consume_steps(
            ctx.behavior_ctx.fight,
            ctx.caster_uid,
            &[ctx.target],
            granted_buff_id,
            duality,
            1,
            target_died,
        ));
    }

    for ally_uid in get_ally_uids(ctx.behavior_ctx.fight, ctx.caster_uid) {
        let cure_instances = ctx.managers.buff_mgr.get(ally_uid).to_vec();
        for instance in cure_instances {
            if instance.type_id != CURE_TYPE_ID {
                continue;
            }
            let Some(permille) = advanced_cure_permille(instance.buff_id) else {
                continue;
            };
            let Some(caster) = get_entity(ctx.behavior_ctx.fight, ctx.caster_uid) else {
                continue;
            };
            let attack = caster
                .attr
                .as_ref()
                .and_then(|attr| attr.attack)
                .unwrap_or(0);
            let heal = attack.saturating_mul(permille) / 1000;
            if heal > 0 {
                effects.push(serialize_leaf_event(BattleEvent::Heal {
                    target: ally_uid,
                    amount: heal,
                    from: instance.from_uid,
                }));
            }
            effects.push(
                crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                    ally_uid,
                    instance.uid,
                    instance.buff_id,
                    instance.from_uid,
                ),
            );
        }
    }

    effects
}

pub(crate) fn build_sotheby_holder_consume_steps(
    fight: &Fight,
    caster_uid: i64,
    target_uids: &[i64],
    granted_buff_id: i32,
    duality: &BuffInstance,
    stack_count: i32,
    suppress_cure: bool,
) -> Vec<ActEffect> {
    if stack_count <= 0 || target_uids.is_empty() {
        return Vec::new();
    }

    // Per `_30091120_design.md` §3 (verified directly from LIVE
    // battle3 r5 fanout):
    // - Poison: each consumed stack = fresh `BuffAdd 300901412` per
    //   hostile target. LIVE uses Add for every stack with a fresh uid.
    // - Cure: first consumed stack = `BuffAdd 30091111` per ally with
    //   a fresh uid; subsequent stacks = `BuffUpdate(7)` on the SAME
    //   uid with layer climbing 1→N. The Cure instance is shared
    //   across stacks within one cast, layered up.
    //
    // Reading back the allocated uid from the BuffAdd ActEffect lets
    // the BuffUpdate packets reference the same uid without exposing
    // any new builder. The replay path at
    // `manager/calculate_mgr.rs::play_effect_add_buff` honors the
    // explicit uid via `buff_mgr.add_with_uid`, and
    // `play_effect_update_buff` finds and updates that instance — so
    // runtime BuffMgr ends with one Cure instance per ally at
    // layer=stack_count, matching LIVE r5.
    let mut add_effects = Vec::new();
    let mut cure_uid_by_ally: HashMap<i64, i64> = HashMap::new();
    for stack_idx in 0..stack_count {
        for &target_uid in target_uids {
            add_effects.push(
                crate::state::battle::fight_step::ActEffectBuilder::buff_add(
                    target_uid,
                    caster_uid,
                    POISON_INSTANCE_BUFF_ID,
                    0,
                ),
            );
            add_effects.push(
                ActEffectBuilder::new(EffectType::Poison as i32, target_uid)
                    .effect_num(0)
                    .build(),
            );
        }
        if !suppress_cure {
            for ally_uid in get_ally_uids(fight, caster_uid) {
                if stack_idx == 0 {
                    let cure_add = crate::state::battle::fight_step::ActEffectBuilder::buff_add(
                        ally_uid,
                        caster_uid,
                        granted_buff_id,
                        1,
                    );
                    let cure_uid = cure_add.buff.as_ref().and_then(|b| b.uid).unwrap_or(0);
                    if cure_uid != 0 {
                        cure_uid_by_ally.insert(ally_uid, cure_uid);
                    }
                    add_effects.push(cure_add);
                    add_effects.push(
                        crate::state::battle::fight_step::ActEffectBuilder::effect_none(ally_uid),
                    );
                } else if let Some(&cure_uid) = cure_uid_by_ally.get(&ally_uid) {
                    let new_layer = stack_idx + 1;
                    add_effects.push(
                        crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                            ally_uid,
                            caster_uid,
                            granted_buff_id,
                            cure_uid,
                            0,
                            new_layer,
                        ),
                    );
                    add_effects.push(
                        crate::state::battle::fight_step::ActEffectBuilder::effect_none(ally_uid),
                    );
                }
            }
        }
    }

    vec![
        ActEffectBuilder::skill_wrapper(crate::state::battle::fight_step::effect_container_step(
            caster_uid,
            caster_uid,
            DUALITY_POTION_BUFF_ID,
            add_effects,
        )),
        ActEffectBuilder::skill_wrapper(crate::state::battle::fight_step::effect_container_step(
            caster_uid,
            caster_uid,
            DUALITY_POTION_BUFF_ID,
            vec![
                crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                    caster_uid,
                    duality.uid,
                    duality.buff_id,
                    duality.from_uid,
                ),
            ],
        )),
    ]
}

fn target_would_die(fight: &Fight, target_uid: i64, effects: &[ActEffect]) -> bool {
    let Some(entity) = get_entity(fight, target_uid) else {
        return false;
    };
    let mut hp = entity.current_hp.unwrap_or(0);
    let mut shield = entity.shield_value.unwrap_or(0);

    for effect in effects {
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if effect.target_id != Some(target_uid) || !is_damage_effect_type(Some(effect_type)) {
            continue;
        }
        let damage = effect.effect_num.unwrap_or(0).max(0);
        let absorbed = damage.min(shield);
        shield = shield.saturating_sub(absorbed);
        hp = hp.saturating_sub(damage.saturating_sub(absorbed));
        if hp <= 0 {
            return true;
        }
    }

    false
}

fn advanced_cure_permille(buff_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|row| row.id == buff_id)?;
    for entry in buff.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id = parts.first().and_then(|v| v.trim().parse::<i32>().ok())?;
        let act_type = cfg
            .buff_act
            .iter()
            .find(|row| row.id == act_id)
            .map(|row| row.r#type.as_str())?;
        if act_type == "AdvancedCure" {
            return parts.get(3).and_then(|v| v.trim().parse::<i32>().ok());
        }
    }
    None
}

fn detonate_target_poison_damage(ctx: &ActionCtx<'_, '_>) -> Option<i32> {
    let mut total = 0_i32;

    for instance in ctx.managers.buff_mgr.get(ctx.target).to_vec() {
        let Some((_, permille)) = parse_dot_features(instance.buff_id) else {
            continue;
        };
        let Some(source) = get_entity(ctx.behavior_ctx.fight, instance.from_uid) else {
            continue;
        };
        let source_attack = source
            .attr
            .as_ref()
            .and_then(|attr| attr.attack)
            .unwrap_or(0);
        if source_attack <= 0 {
            continue;
        }
        let stacks = instance.layer.max(1);
        let rounds = instance.duration.max(1);
        let base_damage = apply_real_hurt_fix(
            &ctx.managers.buff_mgr,
            ctx.target,
            source_attack.saturating_mul(permille) / 1000,
        );
        if base_damage <= 0 {
            continue;
        }
        let crit_damage = base_damage.saturating_mul(1390) / 1000;
        total = total.saturating_add(crit_damage.saturating_mul(stacks).saturating_mul(rounds));
    }

    (total > 0).then_some(total)
}

/// `OriginDamageByAttrAndBuffGroupSize` (id 60127) emits one bonus
/// `OriginDamage` per dispatched target, valued at
/// `caster.attr[attr_id] × permille × stack_count_in_group / 1000`.
/// Tuesday's Lock-Sound mass attack is the fixture caller — see
/// `BehaviorType::OriginDamageByAttrAndBuffGroupSize` for the
/// full mechanic citation.
///
/// `attr_id` lookup currently covers the 100-range base stats
/// (CurrentHp, Hp/MaxHp, Attack, Defense). 200-range bonus stats
/// (Cri / AddDmg / etc.) live on the executor's pending-bonus map
/// and need a different read path; left as a TODO until a fixture
/// invocation needs one.
///
/// Emits `et=130 OriginDamage` (non-crit) tagged with
/// `config_effect=60127`. Tuesday's text says "can critically
/// hit" but our crit hybrid is currently DOT-only (see
/// `mechanics/dot.rs`); extending crit to this emission is the
/// natural follow-up once a faithful crit-roll source lands.
fn execute_origin_damage_by_attr_and_buff_group_size(
    ctx: &mut ActionCtx<'_, '_>,
    attr_id: i32,
    permille: i32,
    group_id: i32,
) -> Vec<ActEffect> {
    if ctx.target == 0 || permille <= 0 {
        return Vec::new();
    }
    let caster_entity = get_entity(ctx.behavior_ctx.fight, ctx.caster_uid);
    let caster_attr = caster_entity
        .and_then(|e| {
            let attr = e.attr.as_ref();
            match attr_id {
                100 => e.current_hp,
                101 => attr.and_then(|a| a.hp),
                102 => attr.and_then(|a| a.attack),
                103 => attr.and_then(|a| a.defense),
                _ => None,
            }
        })
        .unwrap_or(0);
    if caster_attr <= 0 {
        return Vec::new();
    }
    let stacks = target_count_buffs_in_group(&ctx.managers.buff_mgr, ctx.target, group_id);
    if stacks <= 0 {
        return Vec::new();
    }
    let raw_bonus = (caster_attr as i64)
        .saturating_mul(permille as i64)
        .saturating_mul(stacks as i64)
        / 1000;
    let bonus = raw_bonus.clamp(0, i32::MAX as i64) as i32;
    if bonus <= 0 {
        return Vec::new();
    }
    let damage = apply_real_hurt_fix(&ctx.managers.buff_mgr, ctx.target, bonus);
    if damage <= 0 {
        return Vec::new();
    }
    vec![ActEffectBuilder::origin_damage(
        ctx.target,
        damage,
        Some(60127),
    )]
}

/// Whether `effect_type` is one of the six damage emission types
/// (Damage, Crit, OriginDamage, OriginCrit, AdditionalDamage,
/// AdditionalDamageCrit). Public to the parent module so the dispatcher
/// can reuse it for ad-hoc damage filtering inside other behaviors.
pub(super) fn is_damage_effect_type(effect_type: Option<i32>) -> bool {
    matches!(
        effect_type,
        Some(t)
            if t == EffectType::Damage as i32
                || t == EffectType::Crit as i32
                || t == crate::state::battle::types::effects::EffectType::OriginDamage as i32
                || t == crate::state::battle::types::effects::EffectType::OriginCrit as i32
                || t == crate::state::battle::types::effects::EffectType::AdditionalDamage as i32
                || t == crate::state::battle::types::effects::EffectType::AdditionalDamageCrit as i32
    )
}

/// Walk the just-emitted damage effects and, for any HP loss on a team
/// with an initialized bloodtithe pool, append a preview
/// `Bloodpoolvaluechange` reflecting the projected pool gain at this
/// step. The authoritative value is settled later by the bloodtithe
/// mechanic; this preview is what concurrent lookups read in the same
/// step.
fn append_preview_bloodtithe_gain_effects(
    executor: &mut SkillExecutor,
    mechanics: &Mechanics,
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    effects: &mut Vec<ActEffect>,
) {
    if !mechanics.bloodtithe.initialized || target_uid == 0 {
        return;
    }
    let Some(target_entity) = get_entity(fight, target_uid) else {
        return;
    };
    let Some(team_type) = target_entity.team_type else {
        return;
    };

    let raw_damage = effects
        .iter()
        .filter(|effect| {
            effect.target_id == Some(target_uid) && is_damage_effect_type(effect.effect_type)
        })
        .map(|effect| effect.effect_num.unwrap_or(0).max(0))
        .sum::<i32>();
    if raw_damage <= 0 {
        return;
    }

    // Nautika profile: HP lost from being attacked only contributes at 30% efficiency.
    let converted_damage = if caster_uid != 0 && caster_uid.signum() != target_uid.signum() {
        raw_damage * 300 / 1000
    } else {
        raw_damage
    };
    if converted_damage <= 0 {
        return;
    }

    let max_value = mechanics.bloodtithe.get_max(team_type).max(0);
    let (preview_value, preview_acc) = executor
        .pending_bloodtithe_preview
        .entry(team_type)
        .or_insert_with(|| {
            (
                mechanics.bloodtithe.get_value(team_type).max(0),
                mechanics.bloodtithe.get_acc(team_type).max(0),
            )
        });
    *preview_acc += converted_damage;

    let mut gained = 0;
    while *preview_acc >= 3000 && *preview_value < max_value {
        *preview_acc -= 3000;
        *preview_value += 1;
        gained += 1;
    }

    if gained > 0 {
        effects.push(ActEffect {
            effect_type: Some(EffectType::BloodPoolValueChange as i32),
            target_id: Some(target_uid),
            effect_num: Some(team_type),
            effect_num1: Some(gained),
            ..Default::default()
        });
    }
}
