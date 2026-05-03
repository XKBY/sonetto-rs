//! Damage action — handler for the six skill_behavior types that all
//! collapse into `BehaviorType::Damage { rate }`:
//! `Damage`, `Damage2`, `Detonate`, `Detonate2`, `OriginDamage`, `OriginDamage2`.
//!
//! Each runs the same `lost_life::apply` core, then appends a preview
//! `Bloodpoolvaluechange` for any team whose bloodtithe pool has been
//! initialized. The preview is what mid-step lookups read before the
//! authoritative bloodtithe accumulator settles at round close.

use anyhow::Result;
use sonettobuf::{ActEffect, Fight};

use super::super::executor::SkillExecutor;
use super::super::targets::get_entity;
use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::buff_actions::{EffectContext, lost_life};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::mechanics::Mechanics;
use crate::state::battle::skill::condition::buff::target_count_buffs_in_group;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::apply_real_hurt_fix;

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
    vec![
        ActEffectBuilder::new(EffectType::OriginDamage as i32, ctx.target)
            .effect_num(damage)
            .config_effect(60127)
            .build(),
    ]
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
