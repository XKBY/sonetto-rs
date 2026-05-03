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
use crate::state::battle::skill::targets::get_entity;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::utils::attr_update;

pub(super) struct AttrModify;

impl BehaviorAction for AttrModify {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::AttrModify { attr_id, amount }
            | BehaviorType::AttrFix { attr_id, amount } => {
                ctx.executor
                    .add_attr_bonus(ctx.caster_uid, *attr_id, *amount);
                Some(Ok(vec![attr_update(ctx.caster_uid)]))
            }
            BehaviorType::AttrFixByLoseHp {
                step_permille,
                attr_id,
                bonus_per_stack,
                max_stacks,
            } => {
                if *step_permille <= 0 || *bonus_per_stack <= 0 || *max_stacks <= 0 {
                    return Some(Ok(vec![]));
                }
                let entity = get_entity(fight, ctx.caster_uid);
                let max_hp = entity
                    .and_then(|e| e.attr.as_ref())
                    .and_then(|a| a.hp)
                    .unwrap_or(0);
                if max_hp <= 0 {
                    return Some(Ok(vec![]));
                }
                let cur_hp = entity.and_then(|e| e.current_hp).unwrap_or(0);
                let missing = (max_hp - cur_hp).max(0) as i64;
                let missing_permille = (missing * 1000 / max_hp as i64) as i32;
                let stacks = (missing_permille / *step_permille).min(*max_stacks);
                if stacks <= 0 {
                    return Some(Ok(vec![]));
                }
                let bonus = stacks.saturating_mul(*bonus_per_stack);
                ctx.executor.add_attr_bonus(ctx.caster_uid, *attr_id, bonus);
                Some(Ok(vec![attr_update(ctx.caster_uid)]))
            }
            BehaviorType::RaspberryAddCount { attr_id, rate } => {
                let mut effect_ctx = EffectContext::new(
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    ctx.target,
                );
                Some(raspberry::add_count(
                    &mut effect_ctx,
                    ctx.executor,
                    *attr_id,
                    *rate,
                ))
            }
            _ => None,
        }
    }
}
