mod bloodtithe;
mod buff;
mod buff_helper;
mod misc;
pub(crate) mod precast;
mod random;
mod skill;
mod stats;

pub mod parser;

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, Fight, effect_type_enum::EffectType};

use self::precast::{collect_precast_skills_for_caster, infer_precast_per_decr_seed_cap};
use super::cache::resolve_skill_effect_id;
use super::executor::SkillExecutor;
use crate::state::battle::{
    buff_actions::{
        EffectContext, attr_replace::buff_get_attr_replace_permille, heal, heal_by_two_attr,
        lost_life, raspberry,
    },
    context::behavior_context::BehaviorContext,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::cache::SKILL_CACHE,
    skill::condition::parser::parse_condition,
    skill::targets::{TargetResolver, alive_enemies, get_entity},
    types::{behavior::BehaviorType, condition::ConditionType},
    utils::damage_with_hurt,
};

fn is_damage_effect_type(effect_type: Option<i32>) -> bool {
    matches!(
        effect_type,
        Some(t)
            if t == EffectType::Damage as i32
                || t == EffectType::Crit as i32
                || t == crate::state::battle::types::effects::EffectType::OriginDamage as i32
                || t == crate::state::battle::types::effects::EffectType::OriginCrit as i32
                || t == crate::state::battle::types::effects::EffectType::AdditionalDamage as i32
                || t == crate::state::battle::types::effects::EffectType::AdditionalDamageCrit as i32
    )
}

fn append_preview_bloodtithe_gain_effects(
    executor: &mut SkillExecutor,
    mechanics: &Mechanics,
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    effects: &mut Vec<ActEffect>,
) {
    if !mechanics.bloodtithe.initialized || target_uid == 0 {
        return;
    }
    let Some(target_entity) = get_entity(fight, target_uid) else {
        return;
    };
    let Some(team_type) = target_entity.team_type else {
        return;
    };

    let raw_damage = effects
        .iter()
        .filter(|effect| {
            effect.target_id == Some(target_uid) && is_damage_effect_type(effect.effect_type)
        })
        .map(|effect| effect.effect_num.unwrap_or(0).max(0))
        .sum::<i32>();
    if raw_damage <= 0 {
        return;
    }

    // Nautika profile: HP lost from being attacked only contributes at 30% efficiency.
    let converted_damage = if caster_uid != 0 && caster_uid.signum() != target_uid.signum() {
        raw_damage * 300 / 1000
    } else {
        raw_damage
    };
    if converted_damage <= 0 {
        return;
    }

    let max_value = mechanics.bloodtithe.get_max(team_type).max(0);
    let (preview_value, preview_acc) = executor
        .pending_bloodtithe_preview
        .entry(team_type)
        .or_insert_with(|| {
            (
                mechanics.bloodtithe.get_value(team_type).max(0),
                mechanics.bloodtithe.get_acc(team_type).max(0),
            )
        });
    *preview_acc += converted_damage;

    let mut gained = 0;
    while *preview_acc >= 3000 && *preview_value < max_value {
        *preview_acc -= 3000;
        *preview_value += 1;
        gained += 1;
    }

    if gained > 0 {
        effects.push(ActEffect {
            effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
            target_id: Some(target_uid),
            effect_num: Some(team_type),
            effect_num1: Some(gained),
            ..Default::default()
        });
    }
}

pub(crate) fn execute_damage_for_target(
    executor: &mut SkillExecutor,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    rate: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target_uid);
    let mut effects =
        lost_life::apply(&mut ctx, Some(&executor.pending_attr_bonus), rate, skill_id);
    append_preview_bloodtithe_gain_effects(
        executor,
        mechanics,
        fight,
        caster_uid,
        target_uid,
        &mut effects,
    );
    effects
}

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
        BehaviorType::Damage { rate } => Ok(execute_damage_for_target(
            executor, managers, mechanics, fight, caster_uid, target, *rate, skill_id,
        )),

        // --- healing ---
        BehaviorType::Heal { rate } => {
            let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
            Ok(heal(&mut ctx, *rate))
        }
        BehaviorType::HealByTwoAttr {
            missing_percent,
            caster_hp_percent,
        } => {
            let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
            Ok(heal_by_two_attr(
                &mut ctx,
                *missing_percent,
                *caster_hp_percent,
            ))
        }

        // --- buff ---
        BehaviorType::AddBuff { buff_id, count } => Ok(buff::apply(
            executor,
            fight,
            managers,
            mechanics,
            caster_uid,
            target,
            *buff_id,
            *count,
            mechanics.bloodtithe.has_bloodpool(),
            skill_id,
            condition_id,
            condition,
        )),
        BehaviorType::Disperse => Ok(buff::disperse(fight, managers, target)),
        BehaviorType::DisperseForce { buff_id } => {
            Ok(buff::disperse_force(fight, managers, target, *buff_id))
        }
        BehaviorType::Purify => Ok(buff::purify(fight, managers, target)),
        BehaviorType::ReplaceBuff2 {
            source_buff_ids,
            replacement_buff_id,
            duration,
            count,
        } => Ok(buff::replace_buff2(
            fight,
            managers,
            caster_uid,
            target,
            source_buff_ids,
            *replacement_buff_id,
            *duration,
            *count,
        )),
        BehaviorType::ConsumeBuffByTypeId { type_id, count } => Ok(buff::consume_by_type(
            fight, managers, target, *type_id, skill_id, *count,
        )),
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
            let current = mechanics.bloodtithe.get_value(1);
            if current < *consume {
                return Ok(vec![]);
            }
            mechanics.bloodtithe.set_value(1, current - consume);

            // Emit BloodPoolValueChange as a side-effect sibling of the skill 162,
            // not inline inside the skill act_effect payload.
            executor.side_effects.push(ActEffect {
                effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
                target_id: Some(target),
                effect_num: Some(1), // team_type = attacker side
                effect_num1: Some(-consume),
                ..Default::default()
            });

            Ok(buff::apply(
                executor,
                fight,
                managers,
                mechanics,
                caster_uid,
                target,
                *buff_id,
                *count,
                mechanics.bloodtithe.has_bloodpool(),
                skill_id,
                condition_id,
                condition,
            ))
        }

        // --- stats ---
        // NOTE: don't mutate ex_point_mgr directly here — the returned
        // ExPointChange (effectType 111) effect is later applied by
        // calculate_mgr::play_effect_add_ex_point during play_step_data.
        // Mutating here AND letting play_step_data also mutate caused
        // double-application (heroes starting with 2x expected EX).
        BehaviorType::AddExPoint { amount } => Ok(stats::add_ex_point(target, *amount)),
        BehaviorType::AddExPointWithMax { amount } => Ok(stats::add_ex_point(target, *amount)),
        BehaviorType::Bloodlust { amount } => Ok(stats::bloodlust(target, *amount)),
        BehaviorType::ChangePower { amount } => Ok(stats::change_power(target, *amount)),
        BehaviorType::AverageLife => Ok(stats::average_life(target)),

        // --- bloodtithe ---
        BehaviorType::LostLife {
            mode,
            attr_id,
            permille,
            behavior_id,
        } => {
            let floor_permille = buff::ban_lost_life_floor_permille(fight, managers, target);
            let effects = bloodtithe::lost_life(
                fight,
                &managers.buff_mgr,
                &mut mechanics.bloodtithe,
                caster_uid,
                target,
                *mode,
                *attr_id,
                *permille,
                *behavior_id,
                skill_id,
                floor_permille,
            );
            let damage = effects
                .iter()
                .find(|e| {
                    matches!(
                        e.effect_type,
                        Some(t)
                            if t == EffectType::Damage as i32
                                || t == EffectType::Crit as i32
                                || t
                                    == crate::state::battle::types::effects::EffectType::OriginDamage
                                        as i32
                                || t
                                    == crate::state::battle::types::effects::EffectType::OriginCrit
                                        as i32
                    )
                })
                .and_then(|e| e.effect_num)
                .unwrap_or(0);
            tracing::warn!("LostLife: target={} damage={}", target, damage);
            if damage > 0 {
                managers.ex_point_mgr.apply_damage(target, damage);
                mechanics.shadow_cloak.add(target, damage);
            }
            // In combat phases the emitted 111 effects are replayed later by
            // calculate_mgr::play_effect_add_ex_point via play_step_data, so
            // mutating ex_point_mgr here would double-apply. Battle-start /
            // non-combat passive phases never hit play_step_data for LostLife,
            // so we still need the direct mirror there.
            if !behavior_ctx.phase.is_combat() {
                for e in &effects {
                    if e.effect_type == Some(111)
                        && let Some(uid) = e.target_id
                    {
                        managers
                            .ex_point_mgr
                            .add_ex_point(uid, e.effect_num.unwrap_or(0));
                    }
                }
            }
            Ok(effects)
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
        BehaviorType::AddBuffRanId {
            pool_buff_id,
            count,
        } => random::add_buff_ran_id(
            executor,
            rng,
            fight,
            managers,
            mechanics,
            caster_uid,
            target,
            *pool_buff_id,
            *count,
        ),
        BehaviorType::AddMagicCircle { circle_id } => {
            misc::add_magic_circle(fight, caster_uid, *circle_id)
        }

        // --- skill triggers ---
        BehaviorType::DirectUseSkill { skill_id } => {
            if *skill_id <= 0 {
                return Ok(vec![]);
            }
            let out = executor.execute_skill(
                rng,
                fight,
                managers,
                mechanics,
                caster_uid,
                target,
                *skill_id,
                &crate::state::battle::skill::phase::PhaseFilter::combat_with(
                    crate::state::battle::skill::phase::TriggerState::on_active_use_skill(
                        *skill_id,
                    )
                    .with_buff_mgr(&managers.buff_mgr),
                ),
            )?;

            Ok(out)
        }
        BehaviorType::DirectUseBigSkill => {
            let mut out = Vec::new();

            let caster_team =
                crate::state::battle::skill::targets::get_team_type(fight, caster_uid);
            let wrapper_candidate = skill_id - 20;
            let wrapper_effect_id = resolve_skill_effect_id(wrapper_candidate);
            let ex_skill_id = if SKILL_CACHE.contains_key(&wrapper_effect_id) {
                wrapper_candidate
            } else {
                crate::state::battle::skill::targets::get_entity(fight, caster_uid)
                    .and_then(|e| e.ex_skill)
                    .unwrap_or(0)
            };
            if ex_skill_id == 0 {
                return Ok(out);
            }

            // Derive consume range from the big skill's behavior block when available.
            let (_min_consume, max_consume) = SKILL_CACHE
                .get(&ex_skill_id)
                .and_then(|rows| {
                    rows.iter().find_map(|r| {
                        if let BehaviorType::ConsumeExPointAddAttr {
                            min_consume,
                            max_consume,
                        } = r.behavior
                        {
                            Some((min_consume, max_consume))
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or((0, 0));
            let need_ex = config::configs::get()
                .skill_effect
                .iter()
                .find(|s| s.id == resolve_skill_effect_id(ex_skill_id))
                .map(|s| {
                    if s.need_ex_point > 0 {
                        s.need_ex_point
                    } else {
                        max_consume
                    }
                })
                .unwrap_or(max_consume)
                .max(0);
            let current_ex = managers.ex_point_mgr.get_ex_point(caster_uid).max(0);
            // Live wrapper semantics: consume from EX-skill cost lane when
            // present, but cap by current_ex so low-EX casts don't over-consume.
            // (Refund cap = need_ex when present.)
            let initial_consume = if need_ex > 0 {
                need_ex.min(current_ex)
            } else {
                current_ex
            };
            let prep_skill_ids = collect_precast_skills_for_caster(fight, managers, caster_uid);
            let seeded_cap =
                infer_precast_per_decr_seed_cap(fight, managers, caster_uid, &prep_skill_ids);
            let mut consume = seeded_cap
                .map(|cap| initial_consume.min(cap.max(0)))
                .unwrap_or(initial_consume)
                .max(0);
            let mut refund = if need_ex > 0 {
                consume.min(need_ex)
            } else {
                consume
            };
            managers
                .ex_point_mgr
                .set_recent_decr_ex_point(caster_uid, consume);

            // Some wrapper cards first fire a passive-side helper skill before
            // forcing the EX cast.
            for precast_id in prep_skill_ids {
                let mut pre = {
                    let phase = crate::state::battle::skill::phase::PhaseFilter::combat_with(
                        crate::state::battle::skill::phase::TriggerState::on_use_card()
                            .with_buff_mgr(&managers.buff_mgr),
                    );
                    executor.execute_skill(
                        rng, fight, managers, mechanics, caster_uid, caster_uid, precast_id, &phase,
                    )?
                };
                out.append(&mut pre);
            }

            // If prep emitted a self-buff with layer, use that layer as consume cap.
            // This keeps consume/refund aligned with config-driven prep state.
            let prep_layer_cap = out
                .iter()
                .filter_map(|e| e.fight_step.as_ref())
                .flat_map(|s| s.act_effect.iter())
                .find_map(|ae| {
                    if ae.effect_type != Some(EffectType::Buffadd as i32) {
                        return None;
                    }
                    if ae.target_id != Some(caster_uid) {
                        return None;
                    }
                    let buff = ae.buff.as_ref()?;
                    Some(buff.layer.unwrap_or(0).max(0))
                });

            if let Some(cap) = prep_layer_cap {
                consume = consume.min(cap.max(0)).max(0);
                refund = if need_ex > 0 {
                    consume.min(need_ex)
                } else {
                    consume
                };
            }
            if consume != initial_consume {
                managers
                    .ex_point_mgr
                    .set_recent_decr_ex_point(caster_uid, consume);
            }

            if consume > 0 {
                // Don't mutate ex_point_mgr directly — the ExPointChange effect
                // below is applied by calculate_mgr::play_effect_add_ex_point
                // during play_step_data. Direct mutation + replay = double-apply.
                out.push(ActEffect {
                    effect_type: Some(EffectType::Expointchange as i32),
                    target_id: Some(caster_uid),
                    effect_num: Some(-consume),
                    ..Default::default()
                });
                out.push(ActEffect {
                    effect_type: Some(327),
                    target_id: Some(caster_uid),
                    effect_num: Some(0),
                    ..Default::default()
                });
            }

            let ex_target_uid = if crate::state::battle::skill::targets::get_team_type(
                fight, target,
            ) != caster_team
                && crate::state::battle::skill::targets::get_entity(fight, target)
                    .map(|e| e.current_hp.unwrap_or(0) > 0)
                    .unwrap_or(false)
            {
                target
            } else {
                alive_enemies(fight, caster_uid)
                    .into_iter()
                    .next()
                    .unwrap_or(target)
            };

            let mut ex = {
                let phase = crate::state::battle::skill::phase::PhaseFilter::combat_with(
                    crate::state::battle::skill::phase::TriggerState::on_use_card()
                        .with_buff_mgr(&managers.buff_mgr),
                );
                executor.execute_skill(
                    rng,
                    fight,
                    managers,
                    mechanics,
                    caster_uid,
                    ex_target_uid,
                    ex_skill_id,
                    &phase,
                )?
            };
            // Some trigger chains can emit duplicate wrapper skill steps. If any same-act_id
            // step carries damage, drop empty/attr-only duplicates for that act_id.
            let is_ex_skill_step = |e: &ActEffect| {
                e.effect_type == Some(EffectType::Fightstep as i32)
                    && e.fight_step
                        .as_ref()
                        .map(|s| s.act_id == Some(ex_skill_id))
                        .unwrap_or(false)
            };
            let ex_skill_step_has_damage = |e: &ActEffect| {
                e.fight_step
                    .as_ref()
                    .map(|s| {
                        s.act_effect
                            .iter()
                            .any(|ae| is_damage_effect_type(ae.effect_type))
                    })
                    .unwrap_or(false)
            };
            let has_damage_ex_skill_step = ex
                .iter()
                .any(|e| is_ex_skill_step(e) && ex_skill_step_has_damage(e));
            let mut kept_non_damage_ex_skill_step = false;
            ex.retain(|e| {
                if !is_ex_skill_step(e) {
                    return true;
                }
                if ex_skill_step_has_damage(e) {
                    return true;
                }
                if has_damage_ex_skill_step {
                    return false;
                }
                if kept_non_damage_ex_skill_step {
                    return false;
                }
                kept_non_damage_ex_skill_step = true;
                true
            });
            if ex_skill_id == skill_id {
                // Avoid self-nesting: this behavior can execute the same act_id as the
                // current skill, and we only want one outward Skill step.
                let mut flattened = Vec::new();
                for mut effect in ex.drain(..) {
                    if is_ex_skill_step(&effect) {
                        if let Some(step) = effect.fight_step.take() {
                            flattened.extend(step.act_effect);
                        }
                    } else {
                        flattened.push(effect);
                    }
                }
                ex = flattened;
            }
            out.append(&mut ex);

            if refund > 0 {
                // Don't mutate ex_point_mgr directly — calculate_mgr replays
                // the ExPointChange below. See note above on consume.
                out.push(ActEffect {
                    effect_type: Some(EffectType::Expointchange as i32),
                    target_id: Some(caster_uid),
                    effect_num: Some(refund),
                    ..Default::default()
                });
            }
            managers.ex_point_mgr.clear_recent_decr_ex_point(caster_uid);

            Ok(out)
        }
        BehaviorType::ConsumeExPointAddAttr {
            min_consume,
            max_consume,
        } => {
            let consumed = managers
                .ex_point_mgr
                .get_recent_decr_ex_point(caster_uid)
                .max(0);
            let usable = consumed.clamp(*min_consume, *max_consume);
            if usable <= 0 {
                return Ok(vec![]);
            }
            // Encoding: 60174#attr_id#rate_per_point#min#max...
            // The parser keeps min/max; pull rate_per_point from the config behavior string.
            let mut rate_per_point = 0;
            let cfg = config::configs::get();
            let skill_effect_id =
                crate::state::battle::skill::cache::resolve_skill_effect_id(skill_id);
            if let Some(skill_row) = cfg.skill_effect.iter().find(|s| s.id == skill_effect_id) {
                let raw_behaviors = [
                    &skill_row.behavior1,
                    &skill_row.behavior2,
                    &skill_row.behavior3,
                    &skill_row.behavior4,
                    &skill_row.behavior5,
                    &skill_row.behavior6,
                    &skill_row.behavior7,
                    &skill_row.behavior8,
                    &skill_row.behavior9,
                    &skill_row.behavior10,
                ];
                for raw in raw_behaviors {
                    if raw.starts_with("60174#") {
                        rate_per_point = raw
                            .split('#')
                            .nth(2)
                            .and_then(|v| v.parse::<i32>().ok())
                            .unwrap_or(0);
                        if rate_per_point != 0 {
                            break;
                        }
                    }
                }
            }
            if rate_per_point == 0 {
                return Ok(vec![]);
            }
            executor.add_skill_rate_bonus(
                caster_uid,
                caster_uid,
                rate_per_point.saturating_mul(usable),
            );
            // Live payload does not emit an extra ATTR(26) step for this behavior.
            Ok(vec![])
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
        BehaviorType::DirectUseGroupAndStarSkill { group, rank } => {
            // Live gating: this derived cast lane should never fire from
            // non-combat passive phases (battle-start / unconditional).
            // It is only valid while running under combat-trigger contexts.
            if !matches!(
                behavior_ctx.phase,
                crate::state::battle::skill::PhaseFilter::Combat(_)
            ) {
                return Ok(vec![]);
            }

            let mut out = Vec::new();
            // Some configs encode buff-pool picks (20021#pool#count) through
            // this behavior path. Route the pool-pick through the generic
            // `random::add_buff_ran_id` so the partition-and-shuffle bias
            // (favor buffs the target does not already have) and the
            // simulator's seeded RNG apply uniformly across every random-pool
            // mechanic, instead of taking the first `rank` entries
            // deterministically. The `add_buff_ran_id` helper short-circuits
            // when the pool is empty (single-buff config or non-pool id), so
            // the derived-skill cast below still runs in that case.
            if *group >= 10000 {
                out.extend(random::add_buff_ran_id(
                    executor,
                    rng,
                    fight,
                    managers,
                    mechanics,
                    caster_uid,
                    target,
                    *group,
                    *rank,
                )?);
            }

            let chosen_skill_id = if *group >= 10000 {
                // Live-style derived skill lane: base skill id + (9 + rank).
                // Example: 20021#30630111#2 -> 30630122.
                group.saturating_add(9 + (*rank).max(1))
            } else {
                crate::state::battle::skill::get_entity(fight, caster_uid)
                    .and_then(|entity| {
                        let idx = (*rank).saturating_sub(1) as usize;
                        match *group {
                            1 => entity.skill_group1.get(idx).copied(),
                            2 => entity.skill_group2.get(idx).copied(),
                            _ => None,
                        }
                    })
                    .unwrap_or(0)
            };
            if chosen_skill_id <= 0 {
                return Ok(out);
            }

            let mut derived_effects = executor.execute_skill(
                rng,
                fight,
                managers,
                mechanics,
                caster_uid,
                target,
                chosen_skill_id,
                &crate::state::battle::skill::phase::PhaseFilter::combat_with(
                    crate::state::battle::skill::phase::TriggerState::on_active_use_skill(
                        chosen_skill_id,
                    )
                    .with_buff_mgr(&managers.buff_mgr),
                ),
            )?;
            if derived_effects.is_empty() {
                let synthetic = ActEffect {
                    effect_type: Some(EffectType::Fightstep as i32),
                    target_id: Some(0),
                    effect_num: Some(0),
                    fight_step: Some(sonettobuf::FightStep {
                        act_type: Some(sonettobuf::fight_step::ActType::Skill as i32),
                        from_id: Some(caster_uid),
                        to_id: Some(target),
                        act_id: Some(chosen_skill_id),
                        act_effect: vec![],
                        card_index: Some(0),
                        support_hero_id: Some(0),
                        fake_timeline: Some(false),
                        real_skill_type: Some(0),
                        real_skin_id: Some(0),
                    }),
                    ..Default::default()
                };
                derived_effects.push(synthetic);
            }
            out.extend(derived_effects);

            let passive_phase = crate::state::battle::skill::phase::PhaseFilter::combat_with(
                crate::state::battle::skill::phase::TriggerState::on_active_use_skill(
                    chosen_skill_id,
                )
                .with_buff_mgr(&managers.buff_mgr),
            );
            let passive_skills: Vec<i32> =
                crate::state::battle::skill::targets::get_entity(fight, caster_uid)
                    .map(|e| e.passive_skill.clone())
                    .unwrap_or_default();
            for passive_skill_id in passive_skills {
                if passive_skill_id <= 0
                    || passive_skill_id == skill_id
                    || passive_skill_id == chosen_skill_id
                    || !has_active_use_trigger_condition(passive_skill_id)
                {
                    continue;
                }
                let passive_effects = executor.execute_skill(
                    rng,
                    fight,
                    managers,
                    mechanics,
                    caster_uid,
                    target,
                    passive_skill_id,
                    &passive_phase,
                )?;
                if !passive_effects.is_empty() {
                    out.extend(passive_effects);
                }
            }
            Ok(out)
        }
        BehaviorType::ConsumePowerDirectUseSkill { .. } => skill::consume_power_direct_use_skill(),
        BehaviorType::RandomUseSkill { raw } => {
            // `60225#sid:weight&sid:weight&...` — pick one entry and
            // recursively execute it through the skill executor. Without
            // a synced LIVE RNG seed we can't reproduce LIVE's pick
            // exactly; pick the middle entry deterministically because
            // battle2 r1's boss wrapper picks `530000752` (middle of
            // `530000751:100&530000752:100&530000753:100`).
            let pool: Vec<i32> = raw
                .split('#')
                .nth(1)
                .map(|payload| {
                    payload
                        .split('&')
                        .filter_map(|entry| {
                            entry
                                .split(':')
                                .next()
                                .and_then(|s| s.trim().parse::<i32>().ok())
                                .filter(|sid| *sid > 0)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if pool.is_empty() {
                return Ok(vec![]);
            }
            let pick = pool[pool.len() / 2];
            let out = executor.execute_skill(
                rng,
                fight,
                managers,
                mechanics,
                caster_uid,
                target,
                pick,
                &crate::state::battle::skill::phase::PhaseFilter::combat_with(
                    crate::state::battle::skill::phase::TriggerState::on_active_use_skill(pick)
                        .with_buff_mgr(&managers.buff_mgr),
                ),
            )?;
            Ok(out)
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
        BehaviorType::LostAllLifeByAttr {
            caster_attr,
            caster_amount,
            target_attr,
            target_amount,
        } => {
            let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
            Ok(lost_life::lost_all_life_by_attr(
                &mut ctx,
                *caster_attr,
                *caster_amount,
                *target_attr,
                *target_amount,
                skill_id,
            ))
        }
        BehaviorType::DamageRealLostLife {
            buff_id,
            duration: _,
            rate,
        } => {
            let mut ctx = EffectContext::new(fight, managers, mechanics, caster_uid, target);
            Ok(lost_life::damage_real_lost_life(
                &mut ctx, *buff_id, *rate, skill_id,
            ))
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
        BehaviorType::PurifyX { .. } => Ok(buff::purify(fight, managers, target)), // TODO: filter by type_ids

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

fn has_active_use_trigger_condition(skill_id: i32) -> bool {
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

