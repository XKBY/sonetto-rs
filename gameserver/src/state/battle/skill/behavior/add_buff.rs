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
use sonettobuf::{ActEffect, Fight};

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use super::random;
use crate::state::battle::event_queue::{
    BattleEvent, EventContext, EventQueue, drain_to_fight_steps,
};
use crate::state::battle::manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// AddBuff action — routes from the four buff-application variants.
pub(super) struct AddBuff;

impl BehaviorAction for AddBuff {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let fight = ctx.behavior_ctx.fight;
        match behavior {
            BehaviorType::AddBuff { buff_id, count } => Some(Ok(buff::apply(
                buff::BuffApplySpec::new(*buff_id)
                    .caster(ctx.caster_uid)
                    .target(ctx.target)
                    .count(*count)
                    .bloodpool(ctx.mechanics.bloodtithe.has_bloodpool())
                    .skill(ctx.skill_id)
                    .condition(ctx.condition_id, condition),
                ctx.executor,
                fight,
                ctx.managers,
                ctx.mechanics,
            ))),

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
                    return Some(Ok(vec![]));
                }
                let mut queue = EventQueue::new();
                queue.push(BattleEvent::BloodpoolValueChange {
                    team_type: 1,
                    target: ctx.target,
                    delta: -*consume,
                });
                let mut synthetic_fight = Fight::default();
                let mut synthetic_buff_mgr = BuffMgr::new();
                let mut synthetic_entity_mgr = EntityMgr::default();
                let mut event_ctx = EventContext {
                    fight: &mut synthetic_fight,
                    buff_mgr: &mut synthetic_buff_mgr,
                    entity_mgr: &mut synthetic_entity_mgr,
                    bloodtithe: &mut ctx.mechanics.bloodtithe,
                };
                // Emit BloodPoolValueChange as a side-effect sibling of the skill 162.
                ctx.executor
                    .side_effects
                    .extend(drain_to_fight_steps(queue.drain(), &mut event_ctx));

                Some(Ok(buff::apply(
                    buff::BuffApplySpec::new(*buff_id)
                        .caster(ctx.caster_uid)
                        .target(ctx.target)
                        .count(*count)
                        .bloodpool(ctx.mechanics.bloodtithe.has_bloodpool())
                        .skill(ctx.skill_id)
                        .condition(ctx.condition_id, condition),
                    ctx.executor,
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                )))
            }

            BehaviorType::AddBuffRanId {
                pool_buff_id,
                count,
            } => Some(random::add_buff_ran_id(
                ctx.executor,
                ctx.rng,
                fight,
                ctx.managers,
                ctx.mechanics,
                ctx.caster_uid,
                ctx.target,
                *pool_buff_id,
                *count,
            )),

            _ => None,
        }
    }
}
