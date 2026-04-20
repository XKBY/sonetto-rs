use anyhow::Result;
use rand::seq::SliceRandom;
use sonettobuf::{ActEffect, Fight};

use super::super::executor::SkillExecutor;
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::mechanics::Mechanics;
use crate::state::battle::types::condition::ConditionType;

#[allow(clippy::too_many_arguments)]
pub fn add_buff_ran_id(
    executor: &mut SkillExecutor,
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    caster_uid: i64,
    target: i64,
    pool_buff_id: i32,
    count: i32,
) -> Result<Vec<ActEffect>> {
    let cfg = config::configs::get();
    let pool: Vec<i32> = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == pool_buff_id)
        .map(|b| {
            b.features
                .split('#')
                .filter_map(|entry| entry.split(',').next()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();

    if pool.is_empty() {
        return Ok(vec![]);
    }

    let mut rng = rand::thread_rng();
    let mut chosen = pool;
    chosen.shuffle(&mut rng);
    chosen.truncate(count as usize);

    let has_bloodpool = mechanics.bloodtithe.has_bloodpool();
    let mut effects = Vec::new();
    for buff_id in chosen {
        effects.extend(super::buff::apply(
            executor,
            fight,
            managers,
            mechanics,
            caster_uid,
            target,
            buff_id,
            0,
            has_bloodpool,
            0,
            0,
            &ConditionType::None,
        ));
    }
    Ok(effects)
}
