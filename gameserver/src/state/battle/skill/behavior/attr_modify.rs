//! AttrModify action — handler for the behavior variants that
//! register a per-attr bonus on the executor (or feed a raspberry
//! accumulator):
//!
//! * `AttrModify { attr_id, amount }` and `AttrFix { attr_id, amount }`
//!   — register an attr bonus on the caster and emit a single
//!   `attr_update` ActEffect.
//! * `RaspberryAddCount { attr_id, rate }` — add `rate × attr_value`
//!   to the raspberry accumulator (which feeds into shadow_cloak).
//!   Returns the raspberry-side ActEffects through
//!   `buff_actions::raspberry::add_count`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::buff_actions::{EffectContext, raspberry};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::utils::attr_update;

pub(super) struct AttrModify;

impl BehaviorAction for AttrModify {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::AttrModify { attr_id, amount }
            | BehaviorType::AttrFix { attr_id, amount } => {
                ctx.executor
                    .add_attr_bonus(ctx.caster_uid, *attr_id, *amount);
                Ok(vec![attr_update(ctx.caster_uid)])
            }
            BehaviorType::RaspberryAddCount { attr_id, rate } => {
                let mut effect_ctx = EffectContext::new(
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                );
                raspberry::add_count(&mut effect_ctx, ctx.executor, *attr_id, *rate)
            }
            _ => Ok(vec![]),
        }
    }
}
