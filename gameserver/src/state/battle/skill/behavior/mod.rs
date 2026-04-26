mod action;
mod add_buff;
mod bloodtithe;
mod buff;
mod buff_helper;
mod damage;
mod direct_skill;
mod disperse;
mod ex_point;
mod heal;
mod lost_life;
mod misc;
pub(crate) mod precast;
mod random;
mod skill;
mod stats;

use self::action::{ActionCtx, BehaviorAction};

pub mod parser;

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, effect_type_enum::EffectType};

use super::cache::resolve_skill_effect_id;
use super::executor::SkillExecutor;
use crate::state::battle::{
    buff_actions::{
        EffectContext, attr_replace::buff_get_attr_replace_permille, raspberry,
    },
    context::behavior_context::BehaviorContext,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::cache::SKILL_CACHE,
    skill::condition::parser::parse_condition,
    skill::targets::{TargetResolver, get_entity},
    types::{behavior::BehaviorType, condition::ConditionType},
    utils::damage_with_hurt,
};

pub(crate) fn execute_nuo_di_ka_damage_for_target(
    caster_uid: i64,
    target_uid: i64,
    damage: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    if target_uid == 0 || target_uid == caster_uid {
        return vec![];
    }
    vec![damage_with_hurt(
        target_uid, damage, -1, skill_id, caster_uid,
    )]
}

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
    let fight = behavior_ctx.fight;
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
        BehaviorType::BloodPoolMaxChange { amount } => Ok(bloodtithe::pool_max_change(
            fight,
            &mut mechanics.bloodtithe,
            target,
            *amount,
        )),
        BehaviorType::BloodPoolValueChange { amount } => Ok(bloodtithe::pool_value_change(
            fight,
            &mut mechanics.bloodtithe,
            target,
            *amount,
        )),

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
        BehaviorType::AddMagicCircle { circle_id } => {
            misc::add_magic_circle(fight, caster_uid, *circle_id)
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
        BehaviorType::SkillRateUpBySelfBuffType { buff_type_id, rate } => {
            let stacks = buff::sum_stacks_by_type(fight, managers, caster_uid, *buff_type_id);
            if stacks <= 0 || *rate == 0 {
                Ok(vec![])
            } else {
                executor.add_skill_rate_bonus(caster_uid, target, rate.saturating_mul(stacks));
                Ok(vec![])
            }
        }
        BehaviorType::SkillRateUpByBuffType { rate, buff_types } => {
            if *rate == 0 || buff_types.is_empty() {
                return Ok(vec![]);
            }
            let has_matching_type = buff::has_any_type(fight, managers, target, buff_types);
            if has_matching_type {
                executor.add_skill_rate_bonus(caster_uid, target, *rate);
            }
            Ok(vec![])
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
        BehaviorType::Summon { .. } => skill::summon(),
        BehaviorType::Kill => skill::kill(),
        BehaviorType::MonsterChange => skill::monster_change(),

        // --- misc ---
        BehaviorType::AttrModify { attr_id, amount }
        | BehaviorType::AttrFix { attr_id, amount } => {
            executor.add_attr_bonus(caster_uid, *attr_id, *amount);
            Ok(vec![crate::state::battle::utils::attr_update(caster_uid)])
        }
        BehaviorType::BeAttackedAssassinate { .. } => misc::be_attacked_assassinate(),
        BehaviorType::SkillRateUp { rate } => {
            executor.add_skill_rate_bonus(caster_uid, target, *rate);
            Ok(vec![])
        }
        BehaviorType::RaspberryAddCount { attr_id, rate } => {
            let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
            raspberry::add_count(&mut ctx, executor, *attr_id, *rate)
        }
        BehaviorType::MagicCircleAttr { .. } => misc::magic_circle_attr(),
        BehaviorType::CrystalAddCard => misc::crystal_add_card(),
        BehaviorType::IgnoreSkillConfigDamageRate => Ok(vec![]),
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
        BehaviorType::NuoDiKaDamage {
            primary_buff_id,
            primary_rate,
            secondary_buff_id,
            secondary_rate,
            self_loss_param,
        } => {
            let Some(caster) = get_entity(fight, caster_uid) else {
                return Ok(vec![]);
            };
            let current_hp = caster.current_hp.unwrap_or(0);
            let max_hp = caster
                .attr
                .as_ref()
                .and_then(|a| a.hp)
                .unwrap_or(current_hp)
                .max(current_hp)
                .max(0);
            let primary_permille = buff_get_attr_replace_permille(*primary_buff_id).unwrap_or(0);
            let secondary_permille =
                buff_get_attr_replace_permille(*secondary_buff_id).unwrap_or(0);
            let total_permille = (primary_permille.saturating_mul(*primary_rate) / 1000)
                .saturating_add(secondary_permille.saturating_mul(*secondary_rate) / 1000)
                .max(0);
            let self_loss_percent = (*self_loss_param / 5).max(0);
            let self_loss = current_hp.saturating_mul(self_loss_percent) / 100;

            let cfg = config::configs::get();
            let logic_target = cfg
                .skill_effect
                .iter()
                .find(|s| s.id == resolve_skill_effect_id(skill_id))
                .and_then(|s| s.logic_target.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let damage_targets = TargetResolver::new(fight, caster_uid, target)
                .behavior(logic_target)
                .resolve();

            let mut out = Vec::new();
            if self_loss > 0 {
                out.push(damage_with_hurt(
                    caster_uid, self_loss, 30006, skill_id, caster_uid,
                ));
            }
            if total_permille <= 0 {
                return Ok(out);
            }
            let damage = (max_hp.saturating_mul(total_permille) / 1000).max(1);
            for damage_target in damage_targets {
                out.extend(execute_nuo_di_ka_damage_for_target(
                    caster_uid,
                    damage_target,
                    damage,
                    skill_id,
                ));
            }
            Ok(out)
        }
        BehaviorType::ShellUseSkill { .. } => skill::shell_use_skill(),
        BehaviorType::ShellAssign { .. } => skill::shell_assign(),
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

        BehaviorType::Unknown { raw } => {
            tracing::warn!("Skipping unknown behavior: {}", raw);
            Ok(vec![])
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

