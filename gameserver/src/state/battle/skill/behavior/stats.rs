//! Stats action — handler for the small "emit a single stat-change
//! ActEffect" behavior variants that don't fit a more specific action
//! module:
//!
//! * `Bloodlust { amount }` — `effectType = Bloodlust`
//! * `ChangePower { amount }` — `effectType = Powerchange` with
//!   `config_effect = 1`
//! * `AverageLife` — `effectType = Averagelife`
//!
//! ExPoint-shaped variants (`AddExPoint`, `AddExPointWithMax`,
//! `ConsumeExPointAddAttr`) live in `ex_point.rs` because they all
//! key off the per-entity ExPoint counter (Moxie / Faith). Skill-rate
//! buffs (`SkillRateUp*`) live inline in the dispatcher for now.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Stats action — routes from `Bloodlust`, `ChangePower`,
/// `AverageLife`. Each emits a single ActEffect with no contextual
/// side effects.
pub(super) struct Stats;

impl BehaviorAction for Stats {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let target = ctx.target;
        match behavior {
            BehaviorType::Bloodlust { amount } => Some(Ok(bloodlust(target, *amount))),
            BehaviorType::ChangePower { amount } => Some(Ok(change_power(target, *amount))),
            BehaviorType::AverageLife => Some(Ok(average_life(target))),
            _ => None,
        }
    }
}

pub fn bloodlust(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffectBuilder::bloodlust(target, amount)]
}

pub fn change_power(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffectBuilder::power_change(
        Some(target),
        amount,
        Some(1),
    )]
}

pub fn average_life(target: i64) -> Vec<ActEffect> {
    vec![ActEffectBuilder::average_life(target)]
}
