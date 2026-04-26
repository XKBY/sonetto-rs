//! Damage cluster — handlers for the six skill_behavior types that all
//! collapse into `BehaviorType::Damage { rate }`:
//! `Damage`, `Damage2`, `Detonate`, `Detonate2`, `OriginDamage`, `OriginDamage2`.
//!
//! Each runs the same `lost_life::apply` core, then appends a preview
//! `Bloodpoolvaluechange` for any team whose bloodtithe pool has been
//! initialized. The preview is what mid-step lookups read before the
//! authoritative bloodtithe accumulator settles at round close.

use sonettobuf::{ActEffect, Fight, effect_type_enum::EffectType};

use super::super::executor::SkillExecutor;
use super::super::targets::get_entity;
use crate::state::battle::buff_actions::{EffectContext, lost_life};
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::mechanics::Mechanics;

/// Apply a damage behavior to a single target. Returns the lost-life
/// effects emitted by the damage handler plus any preview bloodtithe
/// gain effects produced by HP loss on a team with an initialized pool.
pub fn execute(
    executor: &mut SkillExecutor,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    rate: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target_uid);
    let mut effects =
        lost_life::apply(&mut ctx, Some(&executor.pending_attr_bonus), rate, skill_id);
    append_preview_bloodtithe_gain_effects(
        executor,
        mechanics,
        fight,
        caster_uid,
        target_uid,
        &mut effects,
    );
    effects
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
            effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
            target_id: Some(target_uid),
            effect_num: Some(team_type),
            effect_num1: Some(gained),
            ..Default::default()
        });
    }
}
