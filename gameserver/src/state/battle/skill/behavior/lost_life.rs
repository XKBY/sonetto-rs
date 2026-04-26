//! Lost-life action — handler for the variants that emit HP loss
//! through a custom path (not the normal damage pipeline).
//!
//! Variants owned:
//! * `LostLife { mode, attr_id, permille, behavior_id }` — the
//!   bloodtithe-aware self-life loss path. Routed through
//!   `bloodtithe::lost_life` so the pool gain bookkeeping stays
//!   coupled with the HP delta. Mirrors damage emissions to
//!   `ex_point_mgr` and `shadow_cloak` (both runtime mechanics) and,
//!   in non-combat phases, also mirrors emitted ExPointAdd effects
//!   to `ex_point_mgr` because `play_step_data` doesn't replay them
//!   off the combat path.
//! * `LostAllLifeByAttr { caster_attr, caster_amount, target_attr,
//!    target_amount }` — drop the target to a configured permille of
//!    HP using the attr-difference handler in
//!    `buff_actions::lost_life`.
//! * `DamageRealLostLife { buff_id, duration: _, rate }` — apply real
//!   damage tagged against `buff_id` at `rate`.
//!
//! All three end with effects emitted by either
//! `mechanics::bloodtithe` or `buff_actions::lost_life`; this module
//! owns the dispatch wiring and the per-variant pre/post bookkeeping.

use anyhow::Result;
use sonettobuf::{ActEffect, effect_type_enum::EffectType};

use super::action::{ActionCtx, BehaviorAction};
use super::bloodtithe;
use super::buff;
use crate::state::battle::buff_actions::{EffectContext, lost_life as lost_life_handler};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Lost-life action — routes from `LostLife`, `LostAllLifeByAttr`,
/// and `DamageRealLostLife`.
pub(super) struct LostLife;

impl BehaviorAction for LostLife {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::LostLife {
                mode,
                attr_id,
                permille,
                behavior_id,
            } => {
                let floor_permille =
                    buff::ban_lost_life_floor_permille(fight, ctx.managers, ctx.target);
                let effects = bloodtithe::lost_life(
                    fight,
                    &ctx.managers.buff_mgr,
                    &mut ctx.mechanics.bloodtithe,
                    ctx.caster_uid,
                    ctx.target,
                    *mode,
                    *attr_id,
                    *permille,
                    *behavior_id,
                    ctx.skill_id,
                    floor_permille,
                );
                let damage = effects
                    .iter()
                    .find(|e| {
                        matches!(
                            e.effect_type,
                            Some(t)
                                if t == EffectType::Damage as i32
                                    || t == EffectType::Crit as i32
                                    || t == crate::state::battle::types::effects::EffectType::OriginDamage as i32
                                    || t == crate::state::battle::types::effects::EffectType::OriginCrit as i32
                        )
                    })
                    .and_then(|e| e.effect_num)
                    .unwrap_or(0);
                tracing::warn!("LostLife: target={} damage={}", ctx.target, damage);
                if damage > 0 {
                    ctx.managers.ex_point_mgr.apply_damage(ctx.target, damage);
                    ctx.mechanics.shadow_cloak.add(ctx.target, damage);
                }
                // In combat phases the emitted 111 effects are replayed later by
                // calculate_mgr::play_effect_add_ex_point via play_step_data, so
                // mutating ex_point_mgr here would double-apply. Battle-start /
                // non-combat passive phases never hit play_step_data for LostLife,
                // so we still need the direct mirror there.
                if !ctx.behavior_ctx.phase.is_combat() {
                    for e in &effects {
                        if e.effect_type == Some(111)
                            && let Some(uid) = e.target_id
                        {
                            ctx.managers
                                .ex_point_mgr
                                .add_ex_point(uid, e.effect_num.unwrap_or(0));
                        }
                    }
                }
                Ok(effects)
            }

            BehaviorType::LostAllLifeByAttr {
                caster_attr,
                caster_amount,
                target_attr,
                target_amount,
            } => {
                let mut effect_ctx = EffectContext::new(
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                );
                Ok(lost_life_handler::lost_all_life_by_attr(
                    &mut effect_ctx,
                    *caster_attr,
                    *caster_amount,
                    *target_attr,
                    *target_amount,
                    ctx.skill_id,
                ))
            }

            BehaviorType::DamageRealLostLife {
                buff_id,
                duration: _,
                rate,
            } => {
                let mut effect_ctx = EffectContext::new(
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                );
                Ok(lost_life_handler::damage_real_lost_life(
                    &mut effect_ctx,
                    *buff_id,
                    *rate,
                    ctx.skill_id,
                ))
            }

            _ => Ok(vec![]),
        }
    }
}
