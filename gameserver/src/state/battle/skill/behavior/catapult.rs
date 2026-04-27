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

        effects.extend(buff::apply(
            ctx.executor,
            fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.target,
            *buff_id,
            *primary_stacks,
            has_bloodpool,
            ctx.skill_id,
            ctx.condition_id,
            condition,
        ));

        let mut other_enemies: Vec<i64> = alive_enemies(fight, ctx.caster_uid)
            .into_iter()
            .filter(|uid| *uid != ctx.target)
            .collect();
        other_enemies.shuffle(ctx.rng);

        for enemy_uid in other_enemies
            .into_iter()
            .take(catapult_cap.max(&0).to_owned() as usize)
        {
            effects.extend(buff::apply(
                ctx.executor,
                fight,
                ctx.managers,
                ctx.mechanics,
                ctx.caster_uid,
                enemy_uid,
                *buff_id,
                *catapult_stacks,
                has_bloodpool,
                ctx.skill_id,
                ctx.condition_id,
                condition,
            ));
        }

        Some(Ok(effects))
    }
}
