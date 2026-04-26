mod action;
mod add_buff;
mod attr_modify;
mod bloodtithe;
mod buff;
mod buff_helper;
mod damage;
mod direct_skill;
mod disperse;
mod ex_point;
mod heal;
mod lost_life;
mod magic_circle;
mod misc;
mod nuodika_damage;
pub(crate) mod precast;
mod random;
mod skill_rate;
mod stats;

use self::action::{ActionCtx, BehaviorAction};

pub mod parser;

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::ActEffect;

use super::cache::resolve_skill_effect_id;
use super::executor::SkillExecutor;
use crate::state::battle::{
    context::behavior_context::BehaviorContext,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::cache::SKILL_CACHE,
    skill::condition::parser::parse_condition,
    types::{behavior::BehaviorType, condition::ConditionType},
};

pub struct BehaviorExec<'a, 'ctx> {
    executor: &'a mut SkillExecutor,
    rng: &'a mut StdRng,
    managers: &'a mut Managers,
    mechanics: &'a mut Mechanics,
    behavior_ctx: &'a BehaviorContext<'ctx>,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    condition_id: i32,
}

impl<'a, 'ctx> BehaviorExec<'a, 'ctx> {
    pub fn new(
        executor: &'a mut SkillExecutor,
        rng: &'a mut StdRng,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
        behavior_ctx: &'a BehaviorContext<'ctx>,
    ) -> Self {
        Self {
            executor,
            rng,
            managers,
            mechanics,
            behavior_ctx,
            caster_uid: behavior_ctx.caster_uid,
            target_uid: behavior_ctx.target_uid,
            skill_id: behavior_ctx.skill_id,
            condition_id: 0,
        }
    }

    pub fn caster(mut self, caster_uid: i64) -> Self {
        self.caster_uid = caster_uid;
        self
    }

    pub fn for_target(mut self, target_uid: i64) -> Self {
        self.target_uid = target_uid;
        self
    }

    pub fn skill(mut self, skill_id: i32) -> Self {
        self.skill_id = skill_id;
        self
    }

    pub fn condition_uid(mut self, condition_id: i32) -> Self {
        self.condition_id = condition_id;
        self
    }

    pub fn run(self, behavior: &BehaviorType, condition: &ConditionType) -> Result<Vec<ActEffect>> {
        dispatch_impl(
            self.executor,
            self.rng,
            self.managers,
            self.mechanics,
            self.behavior_ctx,
            self.caster_uid,
            self.target_uid,
            behavior,
            self.skill_id,
            self.condition_id,
            condition,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_behavior(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    behavior_ctx: &BehaviorContext<'_>,
    behavior: &BehaviorType,
    condition_id: i32,
    condition: &ConditionType,
) -> Result<Vec<ActEffect>> {
    let caster_uid = behavior_ctx.caster_uid;
    let skill_id = behavior_ctx.skill_id;
    tracing::debug!(
        "execute_behavior slot={} skill={} caster={} target={}",
        behavior_ctx.slot,
        skill_id,
        caster_uid,
        behavior_ctx.target_uid
    );
    let self_targeted = matches!(
        behavior,
        BehaviorType::Bloodlust { .. }
            | BehaviorType::AddExPoint { .. }
            | BehaviorType::DirectUseBigSkill
    );

    let targets = behavior_ctx.resolve_targets(self_targeted, behavior);
    let mut effects = Vec::new();
    for target in targets {
        effects.extend(
            BehaviorExec::new(executor, rng, managers, mechanics, behavior_ctx)
                .caster(caster_uid)
                .for_target(target)
                .skill(skill_id)
                .condition_uid(condition_id)
                .run(behavior, condition)?,
        );
    }
    Ok(effects)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_impl(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    behavior_ctx: &BehaviorContext<'_>,
    caster_uid: i64,
    target: i64,
    behavior: &BehaviorType,
    skill_id: i32,
    condition_id: i32,
    condition: &ConditionType,
) -> Result<Vec<ActEffect>> {
    match behavior {
        // --- damage ---
        BehaviorType::Damage { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            damage::Damage::execute(behavior, &mut action_ctx, condition)
        }

        // --- healing ---
        BehaviorType::Heal { .. } | BehaviorType::HealByTwoAttr { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            heal::Heal::execute(behavior, &mut action_ctx, condition)
        }

        // --- buff ---
        BehaviorType::AddBuff { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            add_buff::AddBuff::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::Disperse
        | BehaviorType::DisperseForce { .. }
        | BehaviorType::Purify
        | BehaviorType::ReplaceBuff2 { .. }
        | BehaviorType::ConsumeBuffByTypeId { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            disperse::Disperse::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::ConsumeBloodAddBuff { .. } | BehaviorType::ConsumeBloodAddBuff2 { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            add_buff::AddBuff::execute(behavior, &mut action_ctx, condition)
        }

        // --- stats ---
        // NOTE: don't mutate ex_point_mgr directly here — the returned
        // ExPointChange (effectType 111) effect is later applied by
        // calculate_mgr::play_effect_add_ex_point during play_step_data.
        // Mutating here AND letting play_step_data also mutate caused
        // double-application (heroes starting with 2x expected EX).
        BehaviorType::AddExPoint { .. } | BehaviorType::AddExPointWithMax { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            ex_point::ExPoint::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::Bloodlust { .. }
        | BehaviorType::ChangePower { .. }
        | BehaviorType::AverageLife => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            stats::Stats::execute(behavior, &mut action_ctx, condition)
        }

        // --- bloodtithe ---
        BehaviorType::LostLife { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            lost_life::LostLife::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::BloodPoolMaxChange { .. } | BehaviorType::BloodPoolValueChange { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            bloodtithe::BloodPool::execute(behavior, &mut action_ctx, condition)
        }

        // --- random ---
        BehaviorType::AddBuffRanId { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            add_buff::AddBuff::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::AddMagicCircle { .. } | BehaviorType::MagicCircleAttr { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            magic_circle::MagicCircle::execute(behavior, &mut action_ctx, condition)
        }

        // --- skill triggers ---
        BehaviorType::DirectUseSkill { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            direct_skill::DirectSkill::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::DirectUseBigSkill => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            direct_skill::DirectSkill::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::ConsumeExPointAddAttr { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            ex_point::ExPoint::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::SkillRateUpBySelfBuffType { .. }
        | BehaviorType::SkillRateUpByBuffType { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            skill_rate::SkillRate::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::DirectUseGroupAndStarSkill { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            direct_skill::DirectSkill::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::ConsumePowerDirectUseSkill { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            direct_skill::DirectSkill::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::RandomUseSkill { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            direct_skill::DirectSkill::execute(behavior, &mut action_ctx, condition)
        }

        // --- misc skill ops (currently no-op placeholders) ---
        BehaviorType::Summon { .. }
        | BehaviorType::Kill
        | BehaviorType::MonsterChange
        | BehaviorType::BeAttackedAssassinate { .. }
        | BehaviorType::CrystalAddCard
        | BehaviorType::IgnoreSkillConfigDamageRate => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            misc::Misc::execute(behavior, &mut action_ctx, condition)
        }

        // --- attr modify ---
        BehaviorType::AttrModify { .. }
        | BehaviorType::AttrFix { .. }
        | BehaviorType::RaspberryAddCount { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            attr_modify::AttrModify::execute(behavior, &mut action_ctx, condition)
        }

        // --- skill rate ---
        BehaviorType::SkillRateUp { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            skill_rate::SkillRate::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::LostAllLifeByAttr { .. } | BehaviorType::DamageRealLostLife { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            lost_life::LostLife::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::NuoDiKaDamage { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            nuodika_damage::NuoDiKaDamage::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::ShellUseSkill { .. } | BehaviorType::ShellAssign { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            misc::Misc::execute(behavior, &mut action_ctx, condition)
        }
        BehaviorType::PurifyX { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            disperse::Disperse::execute(behavior, &mut action_ctx, condition)
        }

        BehaviorType::Unknown { .. } => {
            let mut action_ctx = ActionCtx {
                executor,
                rng,
                managers,
                mechanics,
                behavior_ctx,
                caster_uid,
                target,
                skill_id,
                condition_id,
            };
            misc::Misc::execute(behavior, &mut action_ctx, condition)
        }
    }
}

fn infer_enter_fight_seed_layer(skill_id: i32, buff_or_type_id: i32) -> Option<i32> {
    let effect_id = resolve_skill_effect_id(skill_id);
    let rows = SKILL_CACHE.get(&effect_id)?;

    rows.iter().find_map(|row| {
        let BehaviorType::AddBuff { buff_id, count } = row.behavior else {
            return None;
        };
        if buff_id != buff_or_type_id {
            return None;
        }
        let is_enter_fight_seed = matches!(
            row.condition,
            ConditionType::EnterFight { .. }
                | ConditionType::EnterFightAnd(_)
                | ConditionType::EnterFightOr(_)
        );
        if !is_enter_fight_seed {
            return None;
        }
        Some(count.max(1))
    })
}

pub(super) fn has_active_use_trigger_condition(skill_id: i32) -> bool {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(row) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return false;
    };
    let conditions = [
        row.condition1.as_str(),
        row.condition2.as_str(),
        row.condition3.as_str(),
        row.condition4.as_str(),
        row.condition5.as_str(),
        row.condition6.as_str(),
        row.condition7.as_str(),
        row.condition8.as_str(),
        row.condition9.as_str(),
        row.condition10.as_str(),
    ];
    conditions.iter().any(|raw| {
        let raw = raw.trim();
        if raw.is_empty() {
            return false;
        }
        let (cond, _) = parse_condition(raw);
        matches!(
            cond,
            ConditionType::ActiveUseSkill | ConditionType::ActiveUseSkillId { .. }
        )
    })
}

