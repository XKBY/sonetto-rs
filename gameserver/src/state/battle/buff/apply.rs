use sonettobuf::{ActEffect, Fight};

use super::super::{
    buff_actions::{self, EffectContext},
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
};

#[allow(clippy::too_many_arguments)]
pub fn apply_buff_effects(
    executor: &mut SkillExecutor,
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    caster_uid: i64,
    target: i64,
    buff_id: i32,
    has_bloodpool: bool,
) -> Vec<ActEffect> {
    let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
    buff_actions::apply_after_buff_add_features(&mut ctx, executor, buff_id, has_bloodpool)
}

#[allow(clippy::too_many_arguments)]
pub fn pre_buff_effects(
    executor: &mut SkillExecutor,
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    caster_uid: i64,
    target: i64,
    buff_id: i32,
    condition_id: i32,
) -> Vec<ActEffect> {
    let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
    buff_actions::apply_before_buff_add_features(&mut ctx, executor, buff_id, condition_id)
}
