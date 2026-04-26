//! Disperse action — handler for the buff-removal / replacement
//! variants. All of them mutate the target's active buff set via
//! `buff::*` helpers and emit the corresponding `BuffDelete` /
//! `BuffAdd` ActEffects.
//!
//! Variants owned:
//! * `Disperse` — drop all buffs the target carries (no filter).
//! * `DisperseForce { buff_id }` — drop a specific buff_id.
//! * `Purify` — drop debuffs only (game-defined "purify" set).
//! * `PurifyX { .. }` — same shape as `Purify` today; the variant
//!   carries a `type_ids` filter slot that is not yet honored
//!   (TODO: thread the filter through `buff::purify`).
//! * `ConsumeBuffByTypeId { type_id, count }` — drop up to `count`
//!   buffs whose type matches `type_id`.
//! * `ReplaceBuff2 { source_buff_ids, replacement_buff_id, duration, count }`
//!   — replace each matching source buff with the replacement.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Disperse action — routes from every buff-removal / replacement
/// variant. Each delegates to a `buff::*` helper; the trait
/// implementation centralizes the dispatch arm boilerplate.
pub(super) struct Disperse;

impl BehaviorAction for Disperse {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::Disperse => Some(Ok(buff::disperse(fight, ctx.managers, ctx.target))),
            BehaviorType::DisperseForce { buff_id } => Some(Ok(buff::disperse_force(
                fight,
                ctx.managers,
                ctx.target,
                *buff_id,
            ))),
            BehaviorType::Purify | BehaviorType::PurifyX { .. } => {
                // TODO: PurifyX should filter by type_ids; today it
                // matches plain Purify behavior. Mirrored from the
                // pre-migration dispatcher.
                Some(Ok(buff::purify(fight, ctx.managers, ctx.target)))
            }
            BehaviorType::ConsumeBuffByTypeId { type_id, count } => {
                Some(Ok(buff::consume_by_type(
                    fight,
                    ctx.managers,
                    ctx.target,
                    *type_id,
                    ctx.skill_id,
                    *count,
                )))
            }
            BehaviorType::ReplaceBuff2 {
                source_buff_ids,
                replacement_buff_id,
                duration,
                count,
            } => Some(Ok(buff::replace_buff2(
                fight,
                ctx.managers,
                ctx.caster_uid,
                ctx.target,
                source_buff_ids,
                *replacement_buff_id,
                *duration,
                *count,
            ))),
            _ => None,
        }
    }
}
