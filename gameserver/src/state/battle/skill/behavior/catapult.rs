//! CatapultBuff action — apply a buff to the primary target, then fan
//! out additional stacks to other random legal enemies.
//!
//! Tuesday's `The Horror's Delight` uses this behavior for Poison
//! spread. The runtime keeps the normal buff-application path so the
//! standard BuffAdd / BuffUpdate and marker emissions (e.g. Poison 213)
//! still come from `buff::apply`.

use anyhow::Result;
use rand::seq::SliceRandom;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::{
    skill::targets::alive_enemies,
    types::{behavior::BehaviorType, condition::ConditionType},
};

pub(super) struct Catapult;

impl BehaviorAction for Catapult {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let BehaviorType::CatapultBuff {
            primary_stacks,
            duration: _duration,
            buff_id,
            catapult_stacks,
            catapult_cap,
        } = behavior
        else {
            return None;
        };

        let fight = ctx.behavior_ctx.fight;
        let has_bloodpool = ctx.mechanics.bloodtithe.has_bloodpool();
        let mut effects = Vec::new();

        // LIVE emits one BuffAdd per stack (count=0 layer=0), not a
        // single BuffAdd with layer=N. Calling `buff::apply` once with
        // `count=N` would collapse N stacks into a single emission with
        // `layer=N`, which is the wrong shape — match LIVE by calling
        // `buff::apply` per stack with count=1.
        for _ in 0..(*primary_stacks).max(0) {
            effects.extend(buff::apply(
                buff::BuffApplySpec::new(*buff_id)
                    .caster(ctx.caster_uid)
                    .target(ctx.target)
                    .count(1)
                    .bloodpool(has_bloodpool)
                    .skill(ctx.skill_id)
                    .condition(ctx.condition_id, condition),
                ctx.executor,
                fight,
                ctx.managers,
                ctx.mechanics,
            ));
        }

        // The catapult spread can land on ANY alive enemy including the
        // primary target. Verified from battle3 r2 step[8] LIVE shape:
        // primary -1 gets 2 initial stacks, then catapult lands on -2
        // and bounces back to -1, producing 4 BuffAdds total (2 +
        // catapult_cap=2 spreads).
        let mut all_enemies: Vec<i64> = alive_enemies(fight, ctx.caster_uid);
        all_enemies.shuffle(ctx.rng);

        for enemy_uid in all_enemies
            .into_iter()
            .take(catapult_cap.max(&0).to_owned() as usize)
        {
            for _ in 0..(*catapult_stacks).max(0) {
                effects.extend(buff::apply(
                    buff::BuffApplySpec::new(*buff_id)
                        .caster(ctx.caster_uid)
                        .target(enemy_uid)
                        .count(1)
                        .bloodpool(has_bloodpool)
                        .skill(ctx.skill_id)
                        .condition(ctx.condition_id, condition),
                    ctx.executor,
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                ));
            }
        }

        Some(Ok(effects))
    }
}
