//! AddTargetBuffByPoison — prioritize poison applications onto the
//! enemy side's current poison carriers instead of blindly fanning out.
//!
//! Willow's poison passives use behavior `60112`. LIVE applies these
//! stacks one at a time, re-checking which enemy currently carries the
//! most poison after each application. When targets tie, the action
//! prefers the current hostile focus (`behavior_ctx.target_uid`), which
//! matches battle3 r2 where both extra Willow stacks stay on `-1`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::{
    skill::targets::{alive_enemies_by_position, get_entity},
    types::{behavior::BehaviorType, condition::ConditionType},
};

pub(super) struct PoisonPriority;

impl BehaviorAction for PoisonPriority {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let BehaviorType::AddTargetBuffByPoison {
            stack_count,
            duration: _duration,
            buff_id,
            max_targets,
        } = behavior
        else {
            return None;
        };

        let total_stacks = (*stack_count).max(0) as usize;
        if total_stacks == 0 || *buff_id == 0 {
            return Some(Ok(vec![]));
        }

        let fight = ctx.behavior_ctx.fight;
        let has_bloodpool = ctx.mechanics.bloodtithe.has_bloodpool();
        let preferred_uid = ctx.behavior_ctx.target_uid;
        let mut pool = rank_enemies(ctx, alive_enemies_by_position(fight, ctx.caster_uid), preferred_uid);
        pool.truncate((*max_targets).max(1) as usize);

        if pool.is_empty() {
            return Some(Ok(vec![]));
        }

        let mut effects = Vec::new();
        let has_existing_poison = pool
            .iter()
            .any(|&uid| poison_instance_count(ctx, uid) > 0);

        if has_existing_poison {
            for _ in 0..total_stacks {
                let target_uid = rank_enemies(ctx, pool.clone(), preferred_uid)
                    .into_iter()
                    .next()
                    .unwrap_or(pool[0]);
                effects.extend(buff::apply(
                    ctx.executor,
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    target_uid,
                    *buff_id,
                    1,
                    has_bloodpool,
                    ctx.skill_id,
                    ctx.condition_id,
                    condition,
                ));
            }
        } else {
            for idx in 0..total_stacks {
                let target_uid = pool[idx % pool.len()];
                effects.extend(buff::apply(
                    ctx.executor,
                    fight,
                    ctx.managers,
                    ctx.mechanics,
                    ctx.caster_uid,
                    target_uid,
                    *buff_id,
                    1,
                    has_bloodpool,
                    ctx.skill_id,
                    ctx.condition_id,
                    condition,
                ));
            }
        }

        Some(Ok(effects))
    }
}

fn rank_enemies(ctx: &ActionCtx<'_, '_>, mut uids: Vec<i64>, preferred_uid: i64) -> Vec<i64> {
    let fight = ctx.behavior_ctx.fight;
    uids.sort_by(|&a, &b| {
        let poison_a = poison_instance_count(ctx, a);
        let poison_b = poison_instance_count(ctx, b);
        poison_b
            .cmp(&poison_a)
            .then_with(|| (b == preferred_uid).cmp(&(a == preferred_uid)))
            .then_with(|| enemy_position(fight, a).cmp(&enemy_position(fight, b)))
            .then_with(|| a.cmp(&b))
    });
    uids
}

fn enemy_position(fight: &sonettobuf::Fight, uid: i64) -> i32 {
    get_entity(fight, uid)
        .and_then(|entity| entity.position)
        .unwrap_or(99)
}

fn poison_instance_count(ctx: &ActionCtx<'_, '_>, uid: i64) -> i32 {
    ctx.managers
        .buff_mgr
        .get(uid)
        .iter()
        .filter(|instance| is_poison_family(instance.buff_id))
        .map(|instance| instance.layer.max(1))
        .sum()
}

fn is_poison_family(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|buff| buff.id == buff_id) else {
        return false;
    };
    buff_cfg.features.split('|').any(|entry| {
        entry
            .split('#')
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .is_some_and(|act_id| matches!(act_id, 803 | 844))
    })
}
