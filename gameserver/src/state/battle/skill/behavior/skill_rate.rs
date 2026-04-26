//! SkillRate action — handler for the three behavior variants that
//! register a per-skill rate bonus on the executor (the bonus is
//! consumed later when the host skill computes damage):
//!
//! * `SkillRateUp { rate }` — unconditional bonus of `rate` permille
//!   from caster to current target.
//! * `SkillRateUpBySelfBuffType { buff_type_id, rate }` — bonus of
//!   `rate × stacks` where `stacks` is the count of buffs on the
//!   caster matching `buff_type_id`. Returns empty if the caster
//!   has no matching buffs or `rate == 0`.
//! * `SkillRateUpByBuffType { rate, buff_types }` — bonus of `rate`
//!   if the target carries any buff matching one of the listed
//!   types. Returns empty if no match or `rate == 0`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

pub(super) struct SkillRate;

impl BehaviorAction for SkillRate {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::SkillRateUp { rate } => {
                ctx.executor
                    .add_skill_rate_bonus(ctx.caster_uid, ctx.target, *rate);
                Some(Ok(vec![]))
            }
            BehaviorType::SkillRateUpBySelfBuffType { buff_type_id, rate } => {
                let stacks =
                    buff::sum_stacks_by_type(fight, ctx.managers, ctx.caster_uid, *buff_type_id);
                if stacks <= 0 || *rate == 0 {
                    return Some(Ok(vec![]));
                }
                ctx.executor.add_skill_rate_bonus(
                    ctx.caster_uid,
                    ctx.target,
                    rate.saturating_mul(stacks),
                );
                Some(Ok(vec![]))
            }
            BehaviorType::SkillRateUpByBuffType { rate, buff_types } => {
                if *rate == 0 || buff_types.is_empty() {
                    return Some(Ok(vec![]));
                }
                let has_matching_type =
                    buff::has_any_type(fight, ctx.managers, ctx.target, buff_types);
                if has_matching_type {
                    ctx.executor
                        .add_skill_rate_bonus(ctx.caster_uid, ctx.target, *rate);
                }
                Some(Ok(vec![]))
            }
            _ => None,
        }
    }
}
