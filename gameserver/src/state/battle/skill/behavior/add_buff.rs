//! AddBuff action — handler for every behavior variant that applies
//! a buff to the target through the `buff::apply` pipeline.
//!
//! Variants owned:
//! * `AddBuff { buff_id, count }` — the simple "apply buff `buff_id`
//!   with `count` stacks" path (`AddBuff`, `AddBuffRound`,
//!   `AddBuffRound2`, `CreateAdditionalDamageAddBuff` all collapse to
//!   this variant in the parser).
//! * `ConsumeBloodAddBuff { consume, buff_id, count }` and
//!   `ConsumeBloodAddBuff2 { ... }` — same as `AddBuff` but first
//!   consumes `consume` from the attacker bloodpool and emits a
//!   `Bloodpoolvaluechange` side-effect. Returns empty if the pool
//!   has fewer than `consume` available; otherwise debits the pool
//!   then runs `buff::apply`.
//! * `AddBuffRanId { pool_buff_id, count }` — pick `count` buffs from
//!   the meta-buff pool keyed by `pool_buff_id`, biased toward buffs
//!   the target does not already carry. Routes through
//!   `random::add_buff_ran_id`.

use anyhow::Result;
use sonettobuf::{ActEffect, effect_type_enum::EffectType};

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use super::random;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// AddBuff action — routes from the four buff-application variants.
pub(super) struct AddBuff;

impl BehaviorAction for AddBuff {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::AddBuff { buff_id, count } => Ok(buff::apply(
                ctx.executor,
                fight,
                ctx.managers,
                ctx.mechanics,
                ctx.caster_uid,
                ctx.target,
                *buff_id,
                *count,
                ctx.mechanics.bloodtithe.has_bloodpool(),
                ctx.skill_id,
                ctx.condition_id,
                condition,
            )),

            BehaviorType::ConsumeBloodAddBuff {
                consume,
                buff_id,
                count,
            }
            | BehaviorType::ConsumeBloodAddBuff2 {
                consume,
                buff_id,
                count,
            } => {
                let current = ctx.mechanics.bloodtithe.get_value(1);
                if current < *consume {
                    return Ok(vec![]);
                }
                ctx.mechanics.bloodtithe.set_value(1, current - consume);

                // Emit BloodPoolValueChange as a side-effect sibling of the skill 162,
                // not inline inside the skill act_effect payload.
                ctx.executor.side_effects.push(ActEffect {
                    effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
                    target_id: Some(ctx.target),
                    effect_num: Some(1), // team_type = attacker side
                    effect_num1: Some(-consume),
                    ..Default::default()
                });

                Ok(buff::apply(
                    ctx.executor,
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                    *buff_id,
                    *count,
                    ctx.mechanics.bloodtithe.has_bloodpool(),
                    ctx.skill_id,
                    ctx.condition_id,
                    condition,
                ))
            }

            BehaviorType::AddBuffRanId {
                pool_buff_id,
                count,
            } => random::add_buff_ran_id(
                ctx.executor,
                ctx.rng,
                fight,
                ctx.managers,
                ctx.mechanics,
                ctx.caster_uid,
                ctx.target,
                *pool_buff_id,
                *count,
            ),

            _ => Ok(vec![]),
        }
    }
}
