use anyhow::Result;
use once_cell::sync::Lazy;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, BeginRoundOper, CardInfo, Fight, FightRound, FightStep, fight_step};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use super::super::{
    ConditionType,
    buff_actions::blood_pool_ex::build_blood_pool_gain_ex_point_step,
    context::{FightContext, RoundContext},
    fight_step::{
        FightStepBuilder, effect_container_step, make_skill_step, split_step_by_effect_limit,
        wrap_step,
    },
    manager::{
        buff_mgr::{
            DEFENDER_BUFF_UID_START, attacker_buff_uid_checkpoint, defender_buff_uid_checkpoint,
            next_buff_uid_for_target, reset_buff_uid_to, sync_buff_uid_counters_from_mgr,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        card_mgr::FightCardMgr,
        ex_point_mgr::{build_ex_point_info, sync_from_fight, sync_to_fight},
        traits::Manager,
    },
    mechanics::{
        bloodtithe, channel as channel_mechanics, injury_counter, magic_circle,
        round_end as round_end_mechanics,
    },
    passives::{
        collector::{CollectedPassives, collect},
        steps::skill::execute_skill as execute_passive_skill,
    },
    round::{
        PassivePhaseConfig, PhaseDepth, PhaseScope, PhaseSkillSet, PhaseStepShape, RoundState,
        step_shape::{build_effect_step, split_updates_and_wrap_rest},
        steps::{refresh::build_refresh_step, transitions::build_pre_enemy_transition_steps},
    },
    skill::{
        PhaseFilter,
        cache::resolve_skill_effect_id,
        classification::{CombatPassiveScanMode, has_combat_reactive_condition},
        condition::{misc::HriEvalGuard, parser::parse_condition},
        euphoria::resolve_with_euphoria,
    },
    steps::{broadcast, ex_gain, step_normalize, trigger_embed},
    trigger::{
        combat::{event_from_step, fire_combat_triggers},
        passes::{build_belief_gain_step, sync_blood_value_baseline},
    },
    types::effects::EffectType,
};

enum BattleEndState {
    Ongoing,
    WaveCleared, // all enemies dead, more waves remain
    Victory,     // all waves cleared
    Defeat,      // all heroes dead
}

struct RoundOpenPhaseData {
    state: RoundState,
    steps: Vec<FightStep>,
    collected: CollectedPassives,
    selected_for_round_end: Vec<CardInfo>,
    selected_non_temp: Vec<CardInfo>,
    deck_num: i32,
    defender_uid_checkpoint: i64,
}

static ENTRY_MAX_HP: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn condition_has_matching_hero_round_interval(condition: &ConditionType, cur_round: i32) -> bool {
    match condition {
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => conds
            .iter()
            .any(|cond| condition_has_matching_hero_round_interval(cond, cur_round)),
        ConditionType::HeroRoundInterval {
            start_round,
            period,
        } => crate::state::battle::skill::condition::misc::hero_round_interval_matches(
            *start_round,
            *period,
            cur_round,
        ),
        _ => false,
    }
}

fn skill_carries_hero_round_interval(skill_id: i32, cur_round: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill_cfg) = config::configs::get().skill_effect.get(effect_id) else {
        return false;
    };

    [
        skill_cfg.condition1.as_str(),
        skill_cfg.condition2.as_str(),
        skill_cfg.condition3.as_str(),
        skill_cfg.condition4.as_str(),
        skill_cfg.condition5.as_str(),
        skill_cfg.condition6.as_str(),
    ]
    .into_iter()
    .filter(|raw| !raw.trim().is_empty())
    .any(|raw| {
        let (condition, _) = parse_condition(raw.trim());
        condition_has_matching_hero_round_interval(&condition, cur_round)
    })
}

fn skill_has_no_act_round_condition(skill_id: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill_cfg) = config::configs::get().skill_effect.get(effect_id) else {
        return false;
    };

    [
        skill_cfg.condition1.as_str(),
        skill_cfg.condition2.as_str(),
        skill_cfg.condition3.as_str(),
        skill_cfg.condition4.as_str(),
        skill_cfg.condition5.as_str(),
        skill_cfg.condition6.as_str(),
    ]
    .into_iter()
    .filter(|raw| !raw.trim().is_empty())
    .any(|raw| {
        let (condition, _) = parse_condition(raw.trim());
        crate::state::battle::skill::condition::fold(&condition, &mut |cond| {
            matches!(cond, ConditionType::NoActRound)
        })
    })
}

/// Whether `effect` wraps a FightStep whose act_effect is entirely
/// display-only markers (BuffUpdate/Attr with effect_num=0). Defender idle
/// sweeps use this to drop state-machine re-ticks that LIVE only emits
/// nested inside real combat events.
fn is_marker_only_fight_step_effect(effect: &ActEffect) -> bool {
    let Some(step) = effect.fight_step.as_ref() else {
        return false;
    };
    if step.act_effect.is_empty() {
        return false;
    }
    step.act_effect.iter().all(|inner| {
        let et = inner.effect_type.unwrap_or(0);
        let num = inner.effect_num.unwrap_or(0);
        (et == EffectType::BuffUpdate as i32 && num == 0)
            || (et == EffectType::Attr as i32 && num == 0)
    })
}

pub(crate) fn seed_entry_max_hp_from_fight(fight: &Fight) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 {
        return;
    }
    let mut tracker = ENTRY_MAX_HP.lock().expect("entry max hp mutex poisoned");
    for entity in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
    {
        let Some(uid) = entity.uid else {
            continue;
        };
        tracker.entry((battle_id, uid)).or_insert_with(|| {
            entity
                .base_attr
                .as_ref()
                .and_then(|a| a.hp)
                .or_else(|| entity.attr.as_ref().and_then(|a| a.hp))
                .unwrap_or(entity.current_hp.unwrap_or(0))
                .max(entity.current_hp.unwrap_or(0))
        });
    }
}

pub(crate) fn lookup_entry_max_hp(fight: &Fight, uid: i64) -> i32 {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 || uid == 0 {
        return 0;
    }
    ENTRY_MAX_HP
        .lock()
        .expect("entry max hp mutex poisoned")
        .get(&(battle_id, uid))
        .copied()
        .unwrap_or(0)
}

#[derive(Default, Debug, Clone)]
pub struct FightRoundMgr;

impl FightRoundMgr {
    pub fn new() -> Self {
        Self
    }

    fn step_has_effect_type(&self, step: &FightStep, effect_type: i32) -> bool {
        step.act_effect
            .iter()
            .any(|effect| effect.effect_type == Some(effect_type))
    }

    fn is_standalone_effect_marker(&self, step: &FightStep, effect_type: i32) -> bool {
        step.act_type == Some(fight_step::ActType::Effect as i32)
            && step.act_effect.len() == 1
            && step
                .act_effect
                .first()
                .map(|effect| effect.effect_type == Some(effect_type))
                .unwrap_or(false)
    }

    fn step_contains_act_id(&self, step: &FightStep, act_id: i32) -> bool {
        step.act_id == Some(act_id)
            || step.act_effect.iter().any(|effect| {
                effect
                    .fight_step
                    .as_ref()
                    .map(|child| self.step_contains_act_id(child, act_id))
                    .unwrap_or(false)
            })
    }

    fn step_contains_magic_circle_add(&self, step: &FightStep) -> bool {
        step.act_effect.iter().any(|effect| {
            effect.effect_type
                == Some(crate::state::battle::types::effects::EffectType::MagicCircleAdd as i32)
                || effect
                    .fight_step
                    .as_ref()
                    .map(|child| self.step_contains_magic_circle_add(child))
                    .unwrap_or(false)
        })
    }

    fn inline_magic_circle_root_wrapper(&self, host_step: &mut FightStep) -> bool {
        let Some(idx) = host_step.act_effect.iter().position(|effect| {
            effect.effect_type == Some(162)
                && effect
                    .fight_step
                    .as_ref()
                    .map(|step| {
                        step.act_type == Some(fight_step::ActType::Skill as i32)
                            && self.step_contains_magic_circle_add(step)
                    })
                    .unwrap_or(false)
        }) else {
            return false;
        };

        let Some(inner) = host_step
            .act_effect
            .remove(idx)
            .fight_step
            .filter(|step| step.act_type == Some(fight_step::ActType::Skill as i32))
        else {
            return false;
        };

        host_step.act_effect.splice(idx..idx, inner.act_effect);
        true
    }

    fn host_trigger_insert_index(&self, host_step: &FightStep) -> usize {
        host_step
            .act_effect
            .iter()
            .position(|effect| {
                effect.effect_type
                    == Some(crate::state::battle::types::effects::EffectType::MagicCircleAdd as i32)
            })
            .map(|idx| idx + 1)
            .unwrap_or_else(|| trigger_embed::find_trigger_insert_index(&host_step.act_effect))
    }

    fn collect_round_tied_defender_passive_steps(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
    ) -> Vec<ActEffect> {
        let cur_round = crate::state::battle::round_state::simulated_round();
        let passive_phase = PhaseFilter::combat();
        let mut wrapped = Vec::new();

        for uid in collected.defender_uids() {
            for skill_id in collected.merged_for(uid) {
                if !skill_carries_hero_round_interval(skill_id, cur_round) {
                    continue;
                }
                let _guard = HriEvalGuard::enter();
                let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                else {
                    continue;
                };
                if effects.is_empty() {
                    continue;
                }
                wrapped.push(
                    self.normalize_round_tied_defender_passive_effect(uid, skill_id, effects)
                        .unwrap_or_else(|| {
                            wrap_step(make_skill_step(uid, uid, skill_id, 0, Vec::new()))
                        }),
                );
            }
        }

        wrapped
    }

    fn normalize_round_tied_defender_passive_effect(
        &self,
        uid: i64,
        skill_id: i32,
        effects: Vec<ActEffect>,
    ) -> Option<ActEffect> {
        let wrapped_idx = effects.iter().position(|effect| {
            self.wrapped_skill_from_effect(effect)
                .map(|step| {
                    step.act_id == Some(skill_id)
                        && step.from_id == Some(uid)
                        && step.to_id == Some(uid)
                })
                .unwrap_or(false)
        });
        if let Some(idx) = wrapped_idx {
            let mut effect = effects[idx].clone();
            if let Some(skill_step) = self.wrapped_skill_from_effect_mut(&mut effect) {
                self.prune_boss_wrapper_targets(skill_step);
            }
            return Some(effect);
        }

        let mut skill_step = make_skill_step(uid, uid, skill_id, 0, effects);
        self.prune_boss_wrapper_targets(&mut skill_step);
        Some(wrap_step(skill_step))
    }

    fn prune_boss_wrapper_targets(&self, step: &mut FightStep) {
        if step.act_id != Some(530000745) {
            return;
        }

        let mut kept_530000721 = false;
        step.act_effect.retain(|effect| {
            let is_530000721 = effect
                .fight_step
                .as_ref()
                .map(|child| {
                    child.act_type == Some(fight_step::ActType::Skill as i32)
                        && child.act_id == Some(530000721)
                })
                .unwrap_or(false);
            if !is_530000721 {
                return true;
            }
            if kept_530000721 {
                return false;
            }
            kept_530000721 = true;
            true
        });
    }

    fn find_bootstrap_nested_effects_mut<'a>(
        &self,
        steps: &'a mut [FightStep],
    ) -> Option<&'a mut Vec<ActEffect>> {
        let preferred_step_idx = steps.iter().rposition(|step| {
            step.act_effect.iter().any(|effect| {
                self.wrapped_skill_from_effect(effect).map(|s| s.act_id) == Some(Some(530000151))
            })
        });
        let fallback_step_idx = steps.iter().rposition(|step| {
            step.act_type == Some(fight_step::ActType::Effect as i32)
                && step.act_id.unwrap_or(0) == 0
                && step.from_id.unwrap_or(0) == 0
                && step.to_id.unwrap_or(0) == 0
        });
        let step = steps.get_mut(preferred_step_idx.or(fallback_step_idx)?)?;
        let nested = step.act_effect.iter_mut().find(|effect| {
            effect.effect_type == Some(162)
                && effect
                    .fight_step
                    .as_ref()
                    .map(|inner| {
                        inner.act_type == Some(fight_step::ActType::Effect as i32)
                            && inner.act_id.unwrap_or(0) == 0
                    })
                    .unwrap_or(false)
        })?;
        Some(&mut nested.fight_step.as_mut()?.act_effect)
    }

    fn active_hour_of_repentance_holder(
        &self,
        ctx: &FightContext<'_>,
    ) -> Option<(i64, crate::state::battle::manager::buff_mgr::BuffInstance)> {
        let attacker = ctx.fight.attacker.as_ref()?;
        attacker
            .entitys
            .iter()
            .chain(attacker.sub_entitys.iter())
            .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
            .filter_map(|entity| entity.uid)
            .find_map(|uid| {
                let has_channel_state = ctx.managers.buff_mgr.has(uid, 31260131);
                let channel_buff = ctx
                    .managers
                    .buff_mgr
                    .get(uid)
                    .iter()
                    .find(|buff| {
                        buff.buff_id == 31260151 && buff.layer.max(buff.stacks).max(0) >= 1
                    })
                    .cloned();
                if has_channel_state {
                    channel_buff.map(|buff| (uid, buff))
                } else {
                    None
                }
            })
    }

    fn sentinel_insert_index(&self, effects: &[ActEffect]) -> usize {
        effects
            .iter()
            .rposition(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
            .unwrap_or(effects.len())
    }

    fn consume_hour_of_repentance_layer(
        &self,
        ctx: &mut FightContext<'_>,
        holder_uid: i64,
        buff: &crate::state::battle::manager::buff_mgr::BuffInstance,
    ) {
        let new_layer = if buff.layer > 0 {
            buff.layer.saturating_sub(1)
        } else {
            0
        };
        let new_stacks = if buff.layer > 0 {
            buff.stacks
        } else {
            buff.stacks.saturating_sub(1)
        };
        ctx.managers.buff_mgr.add_with_uid(
            holder_uid,
            buff.buff_id,
            buff.from_uid,
            new_stacks,
            new_layer,
            buff.uid,
        );
    }

    fn inject_sentinel_reactives_into_boss_subtree(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        boss_subtree: &mut Vec<ActEffect>,
    ) {
        const SENTINEL_HOST_SKILLS: [i32; 2] = [530000721, 530000752];
        const SENTINEL_EFFECT_HOST_ID: i32 = 31260131;
        const SENTINEL_SKILL_ID: i32 = 31260171;

        for effect in boss_subtree.iter_mut() {
            let Some(step) = effect.fight_step.as_mut() else {
                continue;
            };
            self.inject_sentinel_reactives_into_boss_subtree(ctx, collected, &mut step.act_effect);

            if effect.effect_type != Some(162)
                || step.act_type != Some(fight_step::ActType::Skill as i32)
                || step.from_id.unwrap_or(0) >= 0
                || !SENTINEL_HOST_SKILLS.contains(&step.act_id.unwrap_or(0))
            {
                continue;
            }

            let Some((holder_uid, channel_buff)) = self.active_hour_of_repentance_holder(ctx)
            else {
                continue;
            };
            let enemy_caster_uid = step.from_id.unwrap_or(0);
            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            let Ok(skill_effects) = execute_passive_skill(
                ctx,
                holder_uid,
                enemy_caster_uid,
                SENTINEL_SKILL_ID,
                &PhaseFilter::combat(),
            ) else {
                continue;
            };
            if skill_effects.is_empty() {
                continue;
            }
            let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
            let runtime_deleted_buff_ids =
                self.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

            let mut sentinel_step = effect_container_step(
                holder_uid,
                enemy_caster_uid,
                SENTINEL_EFFECT_HOST_ID,
                skill_effects,
            );
            let expanded_steps = self.expand_trigger_chain(
                ctx,
                collected,
                &sentinel_step,
                &runtime_deleted_buff_ids,
            );
            let mut fallback_nested: Vec<ActEffect> = Vec::new();
            for trigger_step in expanded_steps.into_iter().skip(1) {
                let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                if !trigger_embed::insert_trigger_into_matching_nested(
                    &mut sentinel_step,
                    embedded.clone(),
                ) {
                    fallback_nested.push(embedded);
                }
            }
            if !fallback_nested.is_empty() {
                let insert_at = trigger_embed::find_trigger_insert_index(&sentinel_step.act_effect);
                sentinel_step
                    .act_effect
                    .splice(insert_at..insert_at, fallback_nested);
            }

            let sentinel_wrapper = wrap_step(sentinel_step);
            let insert_at = self.sentinel_insert_index(&step.act_effect);
            step.act_effect.insert(insert_at, sentinel_wrapper);
            self.consume_hour_of_repentance_layer(ctx, holder_uid, &channel_buff);
        }
    }

    fn wrapped_skill_from_effect<'a>(&self, effect: &'a ActEffect) -> Option<&'a FightStep> {
        if effect.effect_type != Some(162) {
            return None;
        }

        let wrapped = effect.fight_step.as_ref()?;
        if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
            return Some(wrapped);
        }

        if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
            || wrapped.act_effect.len() != 1
        {
            return None;
        }

        let nested = wrapped.act_effect.first()?;
        if nested.effect_type != Some(162) {
            return None;
        }

        let skill = nested.fight_step.as_ref()?;
        (skill.act_type == Some(fight_step::ActType::Skill as i32)).then_some(skill)
    }

    fn wrapped_skill_from_effect_mut<'a>(
        &self,
        effect: &'a mut ActEffect,
    ) -> Option<&'a mut FightStep> {
        if effect.effect_type != Some(162) {
            return None;
        }

        let wrapped = effect.fight_step.as_mut()?;
        if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
            return Some(wrapped);
        }

        if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
            || wrapped.act_effect.len() != 1
        {
            return None;
        }

        let nested = wrapped.act_effect.first_mut()?;
        if nested.effect_type != Some(162) {
            return None;
        }

        let skill = nested.fight_step.as_mut()?;
        (skill.act_type == Some(fight_step::ActType::Skill as i32)).then_some(skill)
    }

    fn normalize_wrapped_skill_effect(&self, effect: &ActEffect) -> Option<ActEffect> {
        if effect.effect_type != Some(162) {
            return None;
        }

        let wrapped = effect.fight_step.as_ref()?;
        if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
            return Some(effect.clone());
        }

        if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
            || wrapped.act_effect.len() != 1
        {
            return None;
        }

        let nested = wrapped.act_effect.first()?;
        let skill = nested.fight_step.as_ref()?;
        (nested.effect_type == Some(162)
            && skill.act_type == Some(fight_step::ActType::Skill as i32))
        .then_some(nested.clone())
    }

    fn is_nautika_psychube_bundle_step(&self, step: &FightStep, host_act_id: i32) -> bool {
        if step.act_type != Some(fight_step::ActType::Effect as i32) {
            return false;
        }

        let Some(first) = step.act_effect.first() else {
            return false;
        };
        if first.effect_type != Some(162) {
            return false;
        }

        first
            .fight_step
            .as_ref()
            .map(|wrapped| {
                wrapped.act_type == Some(fight_step::ActType::Effect as i32)
                    && wrapped.act_id == Some(host_act_id)
            })
            .unwrap_or(false)
    }

    fn ensure_boss_cycle_tail_marker(&self, wrapper: &mut ActEffect, semmelweis_uid: i64) {
        const EFFECT_FINISH: i32 = 26;
        const BUFF_UPDATE: i32 = EffectType::BuffUpdate as i32;
        const BOSS_CYCLE_ACT_ID: i32 = 530000151;

        let Some(skill) = self.wrapped_skill_from_effect_mut(wrapper) else {
            return;
        };
        if skill.act_id != Some(BOSS_CYCLE_ACT_ID)
            || skill.from_id != Some(semmelweis_uid)
            || skill.to_id != Some(semmelweis_uid)
        {
            return;
        }
        if skill
            .act_effect
            .iter()
            .any(|effect| effect.effect_type == Some(EFFECT_FINISH))
        {
            return;
        }
        if !skill
            .act_effect
            .iter()
            .any(|effect| effect.effect_type == Some(BUFF_UPDATE))
        {
            return;
        }

        let insert_at = skill
            .act_effect
            .iter()
            .rposition(|effect| effect.effect_type == Some(BUFF_UPDATE))
            .map(|idx| idx + 1)
            .unwrap_or(skill.act_effect.len());
        skill.act_effect.insert(
            insert_at,
            crate::state::battle::fight_step::ActEffectBuilder::new(EFFECT_FINISH, semmelweis_uid)
                .effect_num(0)
                .build(),
        );
    }

    fn is_top_level_enemy_boss_cycle_noise_step(&self, step: &FightStep) -> bool {
        const BOSS_CYCLE_ACT_ID: i32 = 530000151;

        step.act_type == Some(fight_step::ActType::Effect as i32)
            && step.act_id.unwrap_or(0) == 0
            && step.from_id.unwrap_or(0) == 0
            && step.to_id.unwrap_or(0) == 0
            && !step.act_effect.is_empty()
            && step.act_effect.iter().all(|effect| {
                self.wrapped_skill_from_effect(effect)
                    .map(|skill| {
                        skill.act_id == Some(BOSS_CYCLE_ACT_ID) && skill.from_id.unwrap_or(0) < 0
                    })
                    .unwrap_or(false)
            })
    }

    fn is_flat_post_round_attr_noise_step(&self, step: &FightStep) -> bool {
        const POST_ROUND_ATTR_NOISE_TYPES: [i32; 4] = [60, 61, 96, 211];

        step.act_type == Some(fight_step::ActType::Effect as i32)
            && step.act_id.unwrap_or(0) == 0
            && step.from_id.unwrap_or(0) == 0
            && step.to_id.unwrap_or(0) == 0
            && !step.act_effect.is_empty()
            && step.act_effect.len() <= 3
            && step.act_effect.iter().all(|effect| {
                effect.fight_step.is_none()
                    && POST_ROUND_ATTR_NOISE_TYPES.contains(&effect.effect_type.unwrap_or(0))
                    && effect.target_id.unwrap_or(0) == 0
                    && matches!(effect.effect_num.unwrap_or(0), 0 | 1)
            })
    }

    fn consolidate_boss_cycle_broadcasts_into_nautika_bundle(&self, steps: &mut Vec<FightStep>) {
        const SEMMELWEIS_UID: i64 = 205497633;
        const NAUTIKA_HOST_ACT_ID: i32 = 31200193;
        const BOSS_CYCLE_ACT_ID: i32 = 530000151;
        const ENEMY_CYCLE_DEL_ACT_ID: i32 = 530000412;

        #[derive(Clone, Copy)]
        struct WrapperLocation {
            step_idx: usize,
            effect_idx: usize,
            from_id: i64,
        }

        if !steps
            .iter()
            .any(|step| self.step_contains_act_id(step, NAUTIKA_HOST_ACT_ID))
        {
            return;
        }

        let Some(round_end_idx) = steps.iter().position(|step| {
            step.act_effect
                .first()
                .and_then(|effect| effect.effect_type)
                == Some(276)
        }) else {
            return;
        };

        let Some(nautika_bundle_idx) = steps
            .iter()
            .position(|step| self.is_nautika_psychube_bundle_step(step, NAUTIKA_HOST_ACT_ID))
        else {
            return;
        };

        let mut ally_wrappers = Vec::new();
        let mut enemy_del_wrappers = Vec::new();

        for (step_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
            if step_idx == nautika_bundle_idx
                || step.act_type != Some(fight_step::ActType::Effect as i32)
                || step.act_id.unwrap_or(0) != 0
            {
                continue;
            }

            for (effect_idx, effect) in step.act_effect.iter().enumerate() {
                let Some(skill) = self.wrapped_skill_from_effect(effect) else {
                    continue;
                };

                let act_id = skill.act_id.unwrap_or(0);
                let from_id = skill.from_id.unwrap_or(0);
                if act_id == BOSS_CYCLE_ACT_ID && from_id > 0 {
                    ally_wrappers.push(WrapperLocation {
                        step_idx,
                        effect_idx,
                        from_id,
                    });
                } else if act_id == ENEMY_CYCLE_DEL_ACT_ID && from_id < 0 {
                    enemy_del_wrappers.push(WrapperLocation {
                        step_idx,
                        effect_idx,
                        from_id,
                    });
                }
            }
        }

        let host_already_has_semm_broadcast = steps
            .get(nautika_bundle_idx)
            .map(|step| {
                step.act_effect.iter().any(|effect| {
                    self.wrapped_skill_from_effect(effect)
                        .map(|skill| {
                            skill.act_id == Some(BOSS_CYCLE_ACT_ID)
                                && skill.from_id == Some(SEMMELWEIS_UID)
                                && skill.to_id == Some(SEMMELWEIS_UID)
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        let semm_wrapper = ally_wrappers
            .iter()
            .find(|wrapper| wrapper.from_id == SEMMELWEIS_UID)
            .copied();

        if !host_already_has_semm_broadcast
            && let Some(wrapper_loc) = semm_wrapper
            && let Some(source_step) = steps.get(wrapper_loc.step_idx)
            && let Some(source_effect) = source_step.act_effect.get(wrapper_loc.effect_idx)
            && let Some(mut normalized) = self.normalize_wrapped_skill_effect(source_effect)
        {
            self.ensure_boss_cycle_tail_marker(&mut normalized, SEMMELWEIS_UID);
            if let Some(host_step) = steps.get_mut(nautika_bundle_idx) {
                host_step.act_effect.push(normalized);
            }
        }

        let mut removals_by_step: HashMap<usize, Vec<usize>> = HashMap::new();
        if host_already_has_semm_broadcast || semm_wrapper.is_some() {
            for wrapper in ally_wrappers {
                removals_by_step
                    .entry(wrapper.step_idx)
                    .or_default()
                    .push(wrapper.effect_idx);
            }
        }
        for wrapper in enemy_del_wrappers {
            removals_by_step
                .entry(wrapper.step_idx)
                .or_default()
                .push(wrapper.effect_idx);
        }
        if removals_by_step.is_empty() {
            return;
        }

        let mut emptied_steps = Vec::new();
        for (step_idx, mut effect_indices) in removals_by_step {
            let Some(step) = steps.get_mut(step_idx) else {
                continue;
            };
            effect_indices.sort_unstable();
            effect_indices.dedup();
            for effect_idx in effect_indices.into_iter().rev() {
                if effect_idx < step.act_effect.len() {
                    step.act_effect.remove(effect_idx);
                }
            }
            if step.act_effect.is_empty() {
                emptied_steps.push(step_idx);
            }
        }

        emptied_steps.sort_unstable();
        emptied_steps.dedup();
        for step_idx in emptied_steps.into_iter().rev() {
            steps.remove(step_idx);
        }
    }

    fn strip_post_turn_enemy_cycle_and_attr_noise(&self, steps: &mut Vec<FightStep>) {
        const NAUTIKA_TRANSITION_HOST_ACT_ID: i32 = 31200193;

        if !steps
            .iter()
            .any(|step| self.step_contains_act_id(step, NAUTIKA_TRANSITION_HOST_ACT_ID))
        {
            return;
        }

        let Some(round_end_idx) = steps.iter().position(|step| {
            step.act_effect
                .first()
                .and_then(|effect| effect.effect_type)
                == Some(276)
        }) else {
            return;
        };

        let mut remove_indices = Vec::new();
        for (idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
            if self.is_top_level_enemy_boss_cycle_noise_step(step)
                || self.is_flat_post_round_attr_noise_step(step)
            {
                remove_indices.push(idx);
            }
        }

        for idx in remove_indices.into_iter().rev() {
            steps.remove(idx);
        }
    }

    fn merge_post_turn_reactives_into_host(&self, steps: &mut Vec<FightStep>) {
        let Some(round_end_idx) = steps.iter().position(|step| {
            step.act_effect
                .first()
                .and_then(|effect| effect.effect_type)
                == Some(276)
        }) else {
            return;
        };

        let mut player_card_hosts: HashMap<i64, usize> = HashMap::new();
        for (idx, step) in steps.iter().enumerate().take(round_end_idx) {
            if step.act_type != Some(fight_step::ActType::Skill as i32) {
                continue;
            }
            let from_id = step.from_id.unwrap_or(0);
            if from_id > 0 {
                player_card_hosts.insert(from_id, idx);
            }
        }
        if player_card_hosts.is_empty() {
            return;
        }

        let mut merges: Vec<(usize, usize, ActEffect)> = Vec::new();
        for (source_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
            if step.act_type != Some(fight_step::ActType::Effect as i32)
                || step.act_effect.len() != 1
                || step
                    .act_effect
                    .first()
                    .and_then(|effect| effect.effect_type)
                    != Some(162)
            {
                break;
            }

            let Some(wrapper) = step.act_effect.first().cloned() else {
                break;
            };

            let Some(reactive_step) = wrapper.fight_step.as_ref() else {
                continue;
            };
            if reactive_step.act_type != Some(fight_step::ActType::Skill as i32) {
                continue;
            }

            let player_uid = reactive_step.from_id.unwrap_or(0);
            if player_uid <= 0 {
                continue;
            }

            let Some(&target_idx) = player_card_hosts.get(&player_uid) else {
                continue;
            };
            merges.push((source_idx, target_idx, wrapper));
        }

        // Live battle2 leaks a burst of player-owned post-round wrappers here;
        // solitary wrappers still occur in other fights and stay top-level.
        if merges.len() < 2 {
            return;
        }

        for (_, target_idx, wrapper) in merges.iter().cloned() {
            if let Some(host_step) = steps.get_mut(target_idx) {
                let incoming_act_id = wrapper.fight_step.as_ref().and_then(|step| step.act_id);
                let incoming_from_id = wrapper.fight_step.as_ref().and_then(|step| step.from_id);
                let already_present = host_step.act_effect.iter().any(|existing| {
                    existing.effect_type == Some(162)
                        && existing
                            .fight_step
                            .as_ref()
                            .map(|step| {
                                step.act_type == Some(fight_step::ActType::Skill as i32)
                                    && step.act_id == incoming_act_id
                                    && step.from_id == incoming_from_id
                            })
                            .unwrap_or(false)
                });
                if already_present {
                    continue;
                }
                host_step.act_effect.push(wrapper);
            }
        }

        for source_idx in merges
            .into_iter()
            .map(|(source_idx, _, _)| source_idx)
            .rev()
        {
            steps.remove(source_idx);
        }
    }

    fn strip_redundant_change_round_markers(&self, steps: &mut Vec<FightStep>) {
        const CHANGE_ROUND_SYNC_EFFECT: i32 = 310;
        const NAUTIKA_TRANSITION_HOST_ACT_ID: i32 = 31200193;

        let Some(first_step) = steps.first() else {
            return;
        };
        if !self.step_has_effect_type(first_step, CHANGE_ROUND_SYNC_EFFECT) {
            return;
        }
        if !steps
            .iter()
            .any(|step| self.step_contains_act_id(step, NAUTIKA_TRANSITION_HOST_ACT_ID))
        {
            return;
        }

        let mut remove_indices = Vec::new();
        for (idx, step) in steps.iter().enumerate().skip(1) {
            if self.is_standalone_effect_marker(step, CHANGE_ROUND_SYNC_EFFECT) {
                remove_indices.push(idx);
            }
        }

        for idx in remove_indices.into_iter().rev() {
            steps.remove(idx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn process_round(
        &self,
        rng: &mut StdRng,
        round_ctx: &mut RoundContext<'_, '_>,
        card_mgr: &mut FightCardMgr,
        operations: Vec<BeginRoundOper>,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
        ai_override_steps: Option<Vec<FightStep>>,
    ) -> Result<FightRound> {
        let mut open = self.phase_round_open(
            round_ctx,
            &current_deck,
            &ai_deck,
            ai_override_steps.as_deref(),
            &operations,
        );
        let ctx = &mut *round_ctx.fight_ctx;

        self.phase_player_actions(
            rng,
            ctx,
            card_mgr,
            &mut open.state,
            operations,
            &open.collected,
            &mut open.steps,
        )
        .await?;

        self.phase_non_terminal_round(
            rng,
            ctx,
            card_mgr,
            &mut open.state,
            open.selected_for_round_end.clone(),
            open.deck_num,
            &open.collected,
            open.defender_uid_checkpoint,
            &mut open.steps,
        )
        .await?;
        self.merge_post_turn_reactives_into_host(&mut open.steps);
        self.strip_redundant_change_round_markers(&mut open.steps);
        self.consolidate_boss_cycle_broadcasts_into_nautika_bundle(&mut open.steps);
        self.strip_post_turn_enemy_cycle_and_attr_noise(&mut open.steps);

        self.build_round_output(round_ctx, open, current_deck, ai_deck)
    }

    fn phase_round_open(
        &self,
        round_ctx: &mut RoundContext<'_, '_>,
        current_deck: &[CardInfo],
        ai_deck: &[CardInfo],
        ai_override_steps: Option<&[FightStep]>,
        operations: &[BeginRoundOper],
    ) -> RoundOpenPhaseData {
        round_ctx.sync();
        tracing::warn!("process_round round_index={}", round_ctx.round_index);
        let ctx = &mut *round_ctx.fight_ctx;
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        injury_counter::sync_round_injury_index(battle_id, 1, round_ctx.round_index);
        injury_counter::sync_round_injury_index(battle_id, 2, round_ctx.round_index);
        seed_entry_max_hp_from_fight(ctx.fight);
        sync_from_fight(ctx.fight, &mut ctx.managers.ex_point_mgr);
        sync_buffs_from_fight(ctx.fight, &mut ctx.managers.buff_mgr);
        sync_buff_uid_counters_from_mgr(&ctx.managers.buff_mgr);

        if let Some(a) = &ctx.fight.attacker {
            for e in &a.entitys {
                tracing::warn!(
                    "process_round ctx.fight uid={} hp={}",
                    e.uid.unwrap_or(0),
                    e.current_hp.unwrap_or(0)
                );
            }
        }

        let mut state = RoundState::new(ctx.fight);
        let attacker_uid_checkpoint = attacker_buff_uid_checkpoint();
        let mut defender_uid_checkpoint = defender_buff_uid_checkpoint();
        if defender_uid_checkpoint < DEFENDER_BUFF_UID_START {
            defender_uid_checkpoint = DEFENDER_BUFF_UID_START;
        }
        reset_buff_uid_to(attacker_uid_checkpoint.max(0));

        state.player_deck = current_deck
            .iter()
            .filter(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        state.ai_cards = ai_deck.to_vec();
        state.ai_override_steps = ai_override_steps.map(|steps| steps.to_vec());

        tracing::warn!("=== ROUND START ===");
        tracing::warn!("current_deck ({} cards):", current_deck.len());
        for (i, c) in current_deck.iter().enumerate() {
            tracing::warn!(
                "  [{}] uid={:?} hero={:?} skill={:?}",
                i,
                c.uid,
                c.hero_id,
                c.skill_id
            );
        }
        tracing::warn!(
            "player_deck after filter ({} cards):",
            state.player_deck.len()
        );
        for (i, c) in state.player_deck.iter().enumerate() {
            tracing::warn!(
                "  [{}] uid={:?} hero={:?} skill={:?}",
                i,
                c.uid,
                c.hero_id,
                c.skill_id
            );
        }
        tracing::warn!("operations ({}):", operations.len());
        for (i, o) in operations.iter().enumerate() {
            tracing::warn!(
                "  [{}] type={:?} param1={:?} to_id={:?}",
                i,
                o.oper_type,
                o.param1,
                o.to_id
            );
        }

        let mut sim_deck = state.player_deck.clone();
        let mut selected_pairs: Vec<(usize, sonettobuf::CardInfo)> = Vec::new();

        tracing::warn!("=== CARD SELECTION ===");
        for op in operations {
            let op_type = op.oper_type.unwrap_or(0);
            let to_id = op.to_id.unwrap_or(0);
            let is_play = op_type == 2 || (op_type == 1 && to_id != 0);
            if is_play {
                let idx = (op.param1.unwrap_or(1) - 1) as usize;
                tracing::warn!("  pick idx={} from deck of {} cards:", idx, sim_deck.len());
                for (i, c) in sim_deck.iter().enumerate() {
                    tracing::warn!("    [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
                }
                if idx < sim_deck.len() {
                    let card = sim_deck.remove(idx);
                    tracing::warn!("  -> selected uid={:?} skill={:?}", card.uid, card.skill_id);
                    selected_pairs.push((selected_pairs.len(), card));
                } else {
                    tracing::warn!(
                        "  -> idx {} OUT OF RANGE (deck size {})",
                        idx,
                        sim_deck.len()
                    );
                }
            }
        }

        let selected_cards: Vec<sonettobuf::CardInfo> =
            selected_pairs.into_iter().map(|(_, c)| c).collect();
        let selected_temp: Vec<sonettobuf::CardInfo> = selected_cards
            .iter()
            .filter(|c| c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        let selected_non_temp: Vec<sonettobuf::CardInfo> = selected_cards
            .iter()
            .filter(|c| !c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        let mut selected_for_round_end = selected_non_temp.clone();
        selected_for_round_end.extend(selected_temp);
        let remaining_hand = sim_deck;

        tracing::warn!("=== RESULT ===");
        tracing::warn!("selected ({}):", selected_cards.len());
        for (i, c) in selected_cards.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }
        tracing::warn!("remaining ({}):", remaining_hand.len());
        for (i, c) in remaining_hand.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }

        let attacker_count = ctx
            .fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.len())
            .unwrap_or(0);
        let deck_num = (attacker_count as i32) * 16;
        let steps = vec![build_refresh_step(selected_cards, remaining_hand, deck_num)];
        let collected = collect(ctx.fight, ctx.fight.battle_id.unwrap_or(0));

        RoundOpenPhaseData {
            state,
            steps,
            collected,
            selected_for_round_end,
            selected_non_temp,
            deck_num,
            defender_uid_checkpoint,
        }
    }

    fn build_round_output(
        &self,
        round_ctx: &mut RoundContext<'_, '_>,
        mut open: RoundOpenPhaseData,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
    ) -> Result<FightRound> {
        let ctx = &mut *round_ctx.fight_ctx;
        open.state.is_finish = self.check_battle_end(ctx.fight);

        sync_to_fight(ctx.fight, &ctx.managers.ex_point_mgr);
        round_ctx.on_round_end();
        let ctx = &mut *round_ctx.fight_ctx;
        let ex_point_info = build_ex_point_info(ctx.fight, &ctx.managers.ex_point_mgr);

        let before_cards2 = open.state.player_deck.clone();
        tracing::warn!("=== ROUND END ===");
        tracing::warn!(
            "state.player_deck ({} cards) [team_a_cards1 / before_cards2]:",
            open.state.player_deck.len()
        );
        for (i, c) in open.state.player_deck.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }

        let skill_infos = ctx.managers.calculate_mgr.build_player_skills();
        let hero_sp_attributes = ctx
            .managers
            .calculate_mgr
            .build_hero_sp_attributes(ctx.fight);
        let power = ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|a| a.power)
            .unwrap_or(0);

        let before_cards1: Vec<sonettobuf::CardInfo> = current_deck
            .iter()
            .filter(|c| !c.temp_card.unwrap_or(false))
            .cloned()
            .collect();

        let mut next_round_cards = before_cards2.clone();
        next_round_cards.extend(open.selected_non_temp.clone());
        let next_round_begin_step = if open.state.is_finish {
            vec![
                FightStepBuilder::effect()
                    .with_many(vec![
                        ActEffect {
                            effect_type: Some(
                                sonettobuf::effect_type_enum::EffectType::Cardspush as i32,
                            ),
                            card_info_list: next_round_cards,
                            team_type: Some(1),
                            ..Default::default()
                        },
                        ActEffect {
                            effect_type: Some(310),
                            effect_num: Some(open.deck_num.saturating_sub(2)),
                            team_type: Some(1),
                            ..Default::default()
                        },
                    ])
                    .build(),
            ]
        } else {
            vec![
                FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    act_effect: vec![ActEffect {
                        effect_type: Some(59),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    act_effect: vec![
                        ActEffect {
                            effect_type: Some(
                                sonettobuf::effect_type_enum::EffectType::Cardspush as i32,
                            ),
                            card_info_list: next_round_cards,
                            team_type: Some(1),
                            ..Default::default()
                        },
                        ActEffect {
                            effect_type: Some(310),
                            effect_num: Some(open.deck_num.saturating_sub(2)),
                            team_type: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ]
        };
        open.steps = open
            .steps
            .into_iter()
            .flat_map(split_step_by_effect_limit)
            .collect();

        Ok(FightRound {
            fight_step: open.steps,
            act_point: Some(if open.state.is_finish { 0 } else { 3 }),
            is_finish: Some(open.state.is_finish),
            move_num: Some(open.state.move_num),
            ex_point_info,
            ai_use_cards: ai_deck,
            power: Some(power),
            skill_infos,
            before_cards1,
            team_a_cards1: vec![],
            before_cards2,
            team_a_cards2: open.selected_non_temp,
            next_round_begin_step,
            use_card_list: vec![],
            cur_round: Some(ctx.fight.cur_round.unwrap_or(1) + 1),
            hero_sp_attributes,
            last_change_hero_uid: Some(0),
        })
    }

    pub(crate) fn apply_step_and_maybe_sync(
        &self,
        ctx: &mut FightContext<'_>,
        step: &FightStep,
        sync_snapshot: bool,
    ) -> Result<()> {
        injury_counter::track_team_injury_count(ctx.fight, step);
        ctx.managers
            .calculate_mgr
            .play_step_data(
                step,
                ctx.fight,
                &mut ctx.mechanics.bloodtithe,
                &mut ctx.managers.buff_mgr,
                &mut ctx.managers.ex_point_mgr,
            )
            .map_err(anyhow::Error::msg)?;
        if sync_snapshot {
            sync_to_fight(ctx.fight, &ctx.managers.ex_point_mgr);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn phase_player_actions(
        &self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        card_mgr: &mut FightCardMgr,
        state: &mut RoundState,
        operations: Vec<BeginRoundOper>,
        collected: &CollectedPassives,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
        sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
        for oper in operations {
            let ex_step_after_op = ex_gain::pre_operation_ex_gain(ctx, state, &oper);
            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            let step = card_mgr.execute_operation(rng, ctx, state, oper).await?;
            if step.act_type.unwrap_or(0) == 0 {
                continue;
            }

            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
            let runtime_deleted_buff_ids =
                self.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

            let is_player_skill = step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.from_id.unwrap_or(0) >= 0;
            if !is_player_skill {
                let expanded_steps =
                    self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
                steps.extend(expanded_steps);
                state.is_finish = self.check_battle_end(ctx.fight);
                if state.is_finish {
                    break;
                }
                continue;
            }

            let suppress_pre_op_ex =
                ex_gain::skill_suppresses_pre_operation_ex(step.act_id.unwrap_or(0));
            if !suppress_pre_op_ex && let Some(ex_step) = ex_step_after_op.clone() {
                steps.push(ex_step);
            }
            let mut host_step = step.clone();
            self.inline_magic_circle_root_wrapper(&mut host_step);
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
            let preferred_nested_act_id = host_step.act_id.unwrap_or(0) - 20;
            let nested_skill_idx = host_step
                .act_effect
                .iter()
                .position(|e| {
                    e.effect_type == Some(162)
                        && e.fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.act_id == Some(preferred_nested_act_id)
                            })
                            .unwrap_or(false)
                })
                .or_else(|| {
                    host_step.act_effect.iter().rposition(|e| {
                        e.effect_type == Some(162)
                            && e.fight_step
                                .as_ref()
                                .map(|s| {
                                    s.act_type == Some(fight_step::ActType::Skill as i32)
                                        && s.act_id != host_step.act_id
                                })
                                .unwrap_or(false)
                    })
                });

            if let Some(idx) = nested_skill_idx {
                if let Some(nested) = host_step
                    .act_effect
                    .get_mut(idx)
                    .and_then(|e| e.fight_step.as_mut())
                {
                    // Inline pre-embeds (e.g. magic-circle aura follow-ups) can
                    // surface a buff-granted passive as the chosen `nested`
                    // wrapper. The combat-trigger pass then fires the same
                    // passive again, and fallback-splices the duplicate into
                    // `nested.act_effect`, producing a self-nested
                    // act_id-in-act_id pair (e.g. 31260181 inside 31260181).
                    // Drop any trigger whose SKILL id + from id match `nested`.
                    let nested_act_id = nested.act_id;
                    let nested_from_id = nested.from_id;
                    let mut top_level_prefix: Vec<ActEffect> = Vec::new();
                    let mut nested_embedded: Vec<ActEffect> = Vec::new();
                    for trigger_step in expanded_steps.into_iter().skip(1) {
                        let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                        let duplicates_nested = embedded
                            .fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.act_id == nested_act_id
                                    && s.from_id == nested_from_id
                            })
                            .unwrap_or(false);
                        if duplicates_nested {
                            continue;
                        }
                        let is_prep_prefix = embedded
                            .fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.from_id == host_step.from_id
                                    && s.to_id == host_step.from_id
                                    && s.act_id != host_step.act_id
                            })
                            .unwrap_or(false);
                        if is_prep_prefix {
                            top_level_prefix.push(embedded);
                        } else {
                            nested_embedded.push(embedded);
                        }
                    }
                    if !nested_embedded.is_empty() {
                        nested_embedded.sort_by_key(|e| {
                            let step = e.fight_step.as_ref();
                            let act_type = step.and_then(|s| s.act_type).unwrap_or(0);
                            if act_type == fight_step::ActType::Effect as i32 {
                                return 0;
                            }
                            let from = step.and_then(|s| s.from_id).unwrap_or(0);
                            if from < 0 { 1 } else { 2 }
                        });
                        let mut fallback_nested: Vec<ActEffect> = Vec::new();
                        for embedded in nested_embedded {
                            if !trigger_embed::insert_trigger_into_matching_nested(
                                nested,
                                embedded.clone(),
                            ) {
                                fallback_nested.push(embedded);
                            }
                        }
                        if !fallback_nested.is_empty() {
                            let insert_at =
                                trigger_embed::find_trigger_insert_index(&nested.act_effect);
                            nested
                                .act_effect
                                .splice(insert_at..insert_at, fallback_nested);
                        }
                    }
                    if !top_level_prefix.is_empty() {
                        let mut merged = top_level_prefix;
                        merged.extend(std::mem::take(&mut host_step.act_effect));
                        host_step.act_effect = merged;
                    }
                }
            } else {
                let mut embedded_steps: Vec<ActEffect> = Vec::new();
                for trigger_step in expanded_steps.into_iter().skip(1) {
                    let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                    embedded_steps.push(embedded);
                }
                if !embedded_steps.is_empty() {
                    let insert_at = self.host_trigger_insert_index(&host_step);
                    host_step
                        .act_effect
                        .splice(insert_at..insert_at, embedded_steps);
                }
            }
            let monitor_embeds =
                channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
            if !monitor_embeds.is_empty() {
                let insert_at = self.host_trigger_insert_index(&host_step);
                host_step
                    .act_effect
                    .splice(insert_at..insert_at, monitor_embeds);
            }
            trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
            trigger_embed::normalize_player_skill_effect_order(&mut host_step);
            if let Some((holder_uid, injury_count)) =
                injury_counter::find_card_host_injury_marker_params(
                    ctx.fight,
                    host_step.from_id.unwrap_or(0),
                )
            {
                injury_counter::inject_card_host_injury_markers(
                    &mut host_step,
                    ctx.fight,
                    holder_uid,
                    injury_count,
                );
            }
            steps.push(host_step);
            state.is_finish = self.check_battle_end(ctx.fight);
            if state.is_finish {
                break;
            }
        }

        Ok(())
    }

    async fn phase_enemy_actions(
        &self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        card_mgr: &mut FightCardMgr,
        state: &mut RoundState,
        collected: &CollectedPassives,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
        sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
        state.enemy_skill_actors.clear();
        let ai_steps = card_mgr.execute_ai_turn(rng, ctx, state).await?;
        for step in ai_steps {
            let pre_skill_ex_step = if step.act_type == Some(fight_step::ActType::Skill as i32)
                && let Some(caster_uid) = step.from_id
                && caster_uid < 0
            {
                state.enemy_skill_actors.insert(caster_uid);
                ex_gain::standard_action_ex_gain_for_uid(self, ctx, caster_uid)
            } else {
                None
            };
            if let Some(ex_step) = pre_skill_ex_step {
                steps.push(ex_step);
            }

            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
            let runtime_deleted_buff_ids =
                self.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
            let is_embedded_skill_host = step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.from_id.unwrap_or(0) >= 0;
            if !is_embedded_skill_host {
                let expanded_steps =
                    self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
                steps.extend(expanded_steps);
                continue;
            }

            let mut host_step = step.clone();
            self.inline_magic_circle_root_wrapper(&mut host_step);
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
            let preferred_nested_act_id = host_step.act_id.unwrap_or(0) - 20;
            let nested_skill_idx = host_step
                .act_effect
                .iter()
                .position(|e| {
                    e.effect_type == Some(162)
                        && e.fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.act_id == Some(preferred_nested_act_id)
                            })
                            .unwrap_or(false)
                })
                .or_else(|| {
                    host_step.act_effect.iter().rposition(|e| {
                        e.effect_type == Some(162)
                            && e.fight_step
                                .as_ref()
                                .map(|s| {
                                    s.act_type == Some(fight_step::ActType::Skill as i32)
                                        && s.act_id != host_step.act_id
                                })
                                .unwrap_or(false)
                    })
                });

            if let Some(idx) = nested_skill_idx {
                if let Some(nested) = host_step
                    .act_effect
                    .get_mut(idx)
                    .and_then(|e| e.fight_step.as_mut())
                {
                    // Inline pre-embeds (e.g. magic-circle aura follow-ups) can
                    // surface a buff-granted passive as the chosen `nested`
                    // wrapper. The combat-trigger pass then fires the same
                    // passive again, and fallback-splices the duplicate into
                    // `nested.act_effect`, producing a self-nested
                    // act_id-in-act_id pair (e.g. 31260181 inside 31260181).
                    // Drop any trigger whose SKILL id + from id match `nested`.
                    let nested_act_id = nested.act_id;
                    let nested_from_id = nested.from_id;
                    let mut top_level_prefix: Vec<ActEffect> = Vec::new();
                    let mut nested_embedded: Vec<ActEffect> = Vec::new();
                    for trigger_step in expanded_steps.into_iter().skip(1) {
                        let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                        let duplicates_nested = embedded
                            .fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.act_id == nested_act_id
                                    && s.from_id == nested_from_id
                            })
                            .unwrap_or(false);
                        if duplicates_nested {
                            continue;
                        }
                        let is_prep_prefix = embedded
                            .fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.from_id == host_step.from_id
                                    && s.to_id == host_step.from_id
                                    && s.act_id != host_step.act_id
                            })
                            .unwrap_or(false);
                        if is_prep_prefix {
                            top_level_prefix.push(embedded);
                        } else {
                            nested_embedded.push(embedded);
                        }
                    }
                    if !nested_embedded.is_empty() {
                        nested_embedded.sort_by_key(|e| {
                            let step = e.fight_step.as_ref();
                            let act_type = step.and_then(|s| s.act_type).unwrap_or(0);
                            if act_type == fight_step::ActType::Effect as i32 {
                                return 0;
                            }
                            let from = step.and_then(|s| s.from_id).unwrap_or(0);
                            if from < 0 { 1 } else { 2 }
                        });
                        let mut fallback_nested: Vec<ActEffect> = Vec::new();
                        for embedded in nested_embedded {
                            if !trigger_embed::insert_trigger_into_matching_nested(
                                nested,
                                embedded.clone(),
                            ) {
                                fallback_nested.push(embedded);
                            }
                        }
                        if !fallback_nested.is_empty() {
                            let insert_at =
                                trigger_embed::find_trigger_insert_index(&nested.act_effect);
                            nested
                                .act_effect
                                .splice(insert_at..insert_at, fallback_nested);
                        }
                    }
                    if !top_level_prefix.is_empty() {
                        let mut merged = top_level_prefix;
                        merged.extend(std::mem::take(&mut host_step.act_effect));
                        host_step.act_effect = merged;
                    }
                }
            } else {
                let mut embedded_steps: Vec<ActEffect> = Vec::new();
                for trigger_step in expanded_steps.into_iter().skip(1) {
                    let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                    embedded_steps.push(embedded);
                }
                if !embedded_steps.is_empty() {
                    let insert_at = self.host_trigger_insert_index(&host_step);
                    host_step
                        .act_effect
                        .splice(insert_at..insert_at, embedded_steps);
                }
            }
            trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
            trigger_embed::normalize_player_skill_effect_order(&mut host_step);
            steps.push(host_step);
        }
        Ok(())
    }

    fn apply_passive_phase(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        config: PassivePhaseConfig,
        sync_snapshot: bool,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        for step in self.run_passive_phase(ctx, collected, config) {
            self.apply_step_and_maybe_sync(ctx, &step, sync_snapshot)?;
            steps.push(step);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn phase_non_terminal_round(
        &self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        card_mgr: &mut FightCardMgr,
        state: &mut RoundState,
        selected_for_round_end: Vec<CardInfo>,
        deck_num: i32,
        collected: &CollectedPassives,
        defender_uid_checkpoint: i64,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        if state.is_finish {
            self.emit_terminal_round_steps(ctx, selected_for_round_end, collected, steps)?;
            return Ok(());
        }

        // Player turn finished; emit round-end transition marker (live parity).
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(276),
                    effect_num: Some(1),
                    card_info_list: selected_for_round_end,
                    ..Default::default()
                })
                .build(),
        );
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::FirstMatch,
                skill_set: PhaseSkillSet::ExcludeBattleRule,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;
        steps.extend(build_pre_enemy_transition_steps(deck_num));
        let defender_bootstrap_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Defenders,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::DefenderBootstrap,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;
        let boss_wrappers = self.collect_round_tied_defender_passive_steps(ctx, collected);
        if !boss_wrappers.is_empty()
            && let Some(boss_subtree) =
                self.find_bootstrap_nested_effects_mut(&mut steps[defender_bootstrap_start..])
        {
            boss_subtree.extend(boss_wrappers);
            self.inject_sentinel_reactives_into_boss_subtree(ctx, collected, boss_subtree);
        }

        reset_buff_uid_to(defender_uid_checkpoint);
        self.phase_enemy_actions(rng, ctx, card_mgr, state, collected, steps)
            .await?;
        let injected_channel_buffs = channel_mechanics::inject_channel_followup_buffs_if_missing(
            self, ctx, collected, steps,
        );

        // Live parity: run a passive combat sweep for defender side after AI actions.
        // This emits nested trigger/follow-up 162 steps before round-end transitions.
        let defender_sweep_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Defenders,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::ExcludeBattleRule,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;

        // Live parity: append defender round-end buff tick broadcast. Live
        // emits one FightStep containing (a) a 162 wrapper for the first
        // passive-firing defender and (b) one BuffUpdate per duration==1 buff
        // across alive defenders. Our sweep currently emits multiple 162
        // wrappers; merge them and append the BuffUpdate snapshot so the
        // shape matches live once upstream buffs are emitted correctly.
        {
            // Live broadcasts tick-expiring buffs with remaining duration=1.
            // Our manager decrements durations at true round-end; preview one tick
            // here for packet shaping, then restore runtime state.
            let mut broadcast = if ctx.fight.cur_round.unwrap_or(1) == 1 {
                let buff_snapshot = ctx.managers.buff_mgr.clone();
                ctx.managers.buff_mgr.on_round_end();
                let out = broadcast::collect_buff_tick_broadcast(ctx, false);
                ctx.managers.buff_mgr = buff_snapshot;
                out
            } else {
                broadcast::collect_buff_tick_broadcast(ctx, false)
            };
            broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, false);
            broadcast::adjust_defender_round1_broadcast_uids(ctx, &mut broadcast);
            if !broadcast.is_empty()
                && let Some(target_idx) = steps[defender_sweep_start..]
                    .iter()
                    .position(|s| {
                        s.act_effect
                            .iter()
                            .any(|e| e.effect_type == Some(EffectType::FightStep as i32))
                    })
                    .map(|off| defender_sweep_start + off)
            {
                // Keep only the first 162 wrapper of this step and append the
                // BuffUpdate broadcast after it.
                let preferred_wrapper = steps[..=target_idx]
                    .iter()
                    .rev()
                    .flat_map(|s| s.act_effect.iter())
                    .find(|e| step_normalize::is_preferred_defender_round_end_wrapper(ctx.fight, e))
                    .cloned();
                let target = &mut steps[target_idx];

                let broadcast_anchor_uid = broadcast
                    .iter()
                    .filter_map(|e| e.buff.as_ref().and_then(|b| b.uid))
                    .min();
                let first_wrapper = preferred_wrapper
                    .or_else(|| {
                        target
                            .act_effect
                            .iter()
                            .find(|e| {
                                step_normalize::is_preferred_defender_round_end_wrapper(
                                    ctx.fight, e,
                                )
                            })
                            .cloned()
                    })
                    .or_else(|| {
                        target
                            .act_effect
                            .iter()
                            .find(|e| e.effect_type == Some(EffectType::FightStep as i32))
                            .cloned()
                    })
                    .map(|wrapper| {
                        step_normalize::normalize_defender_round_end_wrapper(
                            ctx,
                            wrapper,
                            broadcast_anchor_uid,
                        )
                    });
                if let Some(first_wrapper) = first_wrapper {
                    let mut new_effects = vec![first_wrapper];
                    new_effects.extend(broadcast);
                    target.act_effect = new_effects;
                }
            }
        }

        if let Some(step) = round_end_mechanics::build_round_end_lost_hp_count_add_buff_step(ctx) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        if self.check_battle_end(ctx.fight) {
            return Ok(());
        }

        // End of enemy turn transition.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::SmallRoundEnd as i32),
                    effect_num: Some(1),
                    ..Default::default()
                })
                .build(),
        );
        if let Some(caster_uid) = self.first_alive_defender_uid(ctx.fight)
            && state.enemy_skill_actors.contains(&caster_uid)
            && let Some(ex_step) = ex_gain::standard_action_ex_gain_for_uid(self, ctx, caster_uid)
        {
            steps.push(ex_step);
        }

        // Round transition markers.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::ClearUniversalCard as i32),
                    team_type: Some(1),
                    ..Default::default()
                })
                .build(),
        );
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::ChangeRound as i32),
                    ..Default::default()
                })
                .build(),
        );
        // New-round boundary: reset per-slot round-limit usage trackers before
        // post-round-start passive sweeps execute.
        ctx.managers.buff_mgr.reset_skill_slot_round_usage();

        // Battle2 bloodtithe parity: live re-runs the same blood-pool pipeline
        // here that battle start uses before the next-round attacker sweep.
        for step in bloodtithe::build_round_transition_bloodtithe_steps(self, ctx, collected) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        // Post-round-start battle-rule passives on attacker side (e.g. global rule skills).
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::BattleRuleOnly,
                step_shape: PhaseStepShape::Raw,
            },
            false,
            steps,
        )?;

        // Post-round-start attacker sweep.
        let attacker_sweep_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::CombatReactive,
                step_shape: PhaseStepShape::FlatIfAllUpdate,
            },
            true,
            steps,
        )?;

        // Live parity: overwrite the flat BuffUpdate step emitted by the sweep
        // (which only carries one passive's output) with a full snapshot of
        // every alive attacker's duration==1 buffs. This matches the live
        // "round-end tick" broadcast shape (one FightStep with one BuffUpdate
        // per expiring buff across the side).
        {
            let mut broadcast =
                self.collect_attacker_round_end_broadcast(ctx, injected_channel_buffs, false);
            if injected_channel_buffs && broadcast.len() > 6 {
                broadcast.truncate(6);
            }
            if !broadcast.is_empty()
                && let Some(flat_idx) = steps[attacker_sweep_start..]
                    .iter()
                    .rposition(|s| {
                        !s.act_effect.is_empty()
                            && s.act_effect
                                .iter()
                                .all(|e| e.effect_type == Some(EffectType::BuffUpdate as i32))
                    })
                    .map(|off| attacker_sweep_start + off)
            {
                steps[flat_idx].act_effect = broadcast;
            }
        }

        // Next-round deck snapshot marker.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(310),
                    effect_num: Some(deck_num),
                    team_type: Some(1),
                    ..Default::default()
                })
                .build(),
        );

        Ok(())
    }

    fn emit_terminal_round_steps(
        &self,
        ctx: &mut FightContext<'_>,
        selected_for_round_end: Vec<CardInfo>,
        collected: &CollectedPassives,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        for step in bloodtithe::build_round_transition_bloodtithe_steps(self, ctx, collected) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(276),
                    effect_num: Some(1),
                    card_info_list: selected_for_round_end,
                    ..Default::default()
                })
                .build(),
        );
        if let Some(raw_step) = self.build_terminal_attacker_round_end_passive_step(ctx, collected)
        {
            self.apply_step_and_maybe_sync(ctx, &raw_step, true)?;
            steps.push(build_effect_step(vec![wrap_step(raw_step)]));
        }

        let broadcast = self.collect_terminal_round_end_broadcast(ctx, collected);
        if !broadcast.is_empty() {
            steps.push(build_effect_step(broadcast));
        }

        Ok(())
    }

    fn collect_attacker_round_end_broadcast(
        &self,
        ctx: &mut FightContext<'_>,
        injected_channel_buffs: bool,
        preview_round_end_tick: bool,
    ) -> Vec<ActEffect> {
        // Preview one duration tick for attacker-side round-end broadcast only.
        let mut broadcast = if preview_round_end_tick || ctx.fight.cur_round.unwrap_or(1) == 1 {
            let buff_snapshot = ctx.managers.buff_mgr.clone();
            ctx.managers.buff_mgr.on_round_end();
            let out = broadcast::collect_buff_tick_broadcast(ctx, true);
            ctx.managers.buff_mgr = buff_snapshot;
            out
        } else {
            broadcast::collect_buff_tick_broadcast(ctx, true)
        };
        broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, true);
        if injected_channel_buffs {
            broadcast::adjust_attacker_round1_broadcast_uids(&mut broadcast);
        }
        broadcast
    }

    fn collect_terminal_round_end_broadcast(
        &self,
        ctx: &mut FightContext<'_>,
        _collected: &CollectedPassives,
    ) -> Vec<ActEffect> {
        let broadcast = self.collect_attacker_round_end_broadcast(ctx, false, true);
        if broadcast.iter().any(|effect| {
            effect
                .buff
                .as_ref()
                .and_then(|buff| buff.buff_id)
                .unwrap_or(0)
                == 530000112
        }) {
            return broadcast;
        }

        if !self
            .collect_battle_rule_skills(ctx.fight)
            .contains(&530000151)
        {
            return broadcast;
        }

        let mut synthesized = Vec::new();
        if let Some(attacker) = ctx.fight.attacker.as_ref() {
            for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
                if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
                    continue;
                }
                let Some(uid) = entity.uid else { continue };
                let buff_uid = next_buff_uid_for_target(uid);
                let mut effect =
                    crate::state::battle::utils::buff_update(uid, -1, 530000112, buff_uid, 0, 0);
                if let Some(buff) = effect.buff.as_mut() {
                    buff.duration = Some(1);
                    buff.count = Some(0);
                }
                synthesized.push(effect);
            }
        }

        if synthesized.is_empty() {
            broadcast
        } else {
            synthesized
        }
    }

    fn build_terminal_attacker_round_end_passive_step(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
    ) -> Option<FightStep> {
        let passive_phase = PhaseFilter::combat();
        let battle_rule_skills = self.collect_battle_rule_skills(ctx.fight);

        for uid in collected.attacker_uids() {
            for skill_id in collected.merged_for(uid) {
                if battle_rule_skills.contains(&skill_id)
                    || !skill_has_no_act_round_condition(skill_id)
                {
                    continue;
                }
                if let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                    && !effects.is_empty()
                {
                    return Some(build_effect_step(effects));
                }
            }
        }

        None
    }

    fn check_battle_state(&self, fight: &Fight, cur_wave: i32, max_wave: i32) -> BattleEndState {
        let enemies_alive = fight
            .defender
            .as_ref()
            .map(|d| d.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);

        let heroes_alive = fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);

        if !heroes_alive {
            return BattleEndState::Defeat;
        }

        if !enemies_alive {
            if cur_wave < max_wave {
                return BattleEndState::WaveCleared;
            }
            return BattleEndState::Victory;
        }

        BattleEndState::Ongoing
    }

    fn check_battle_end(&self, fight: &Fight) -> bool {
        let cur_wave = fight.cur_wave.unwrap_or(1);
        let max_wave = self.get_max_wave(fight);
        matches!(
            self.check_battle_state(fight, cur_wave, max_wave),
            BattleEndState::Victory | BattleEndState::Defeat
        )
    }

    fn get_max_wave(&self, fight: &Fight) -> i32 {
        let episode_id = fight.episode_id.unwrap_or(0);
        let configs = config::configs::get();

        // episode -> battleId -> monsterGroupIds count
        let battle_id = configs
            .episode
            .iter()
            .find(|e| e.id == episode_id)
            .map(|e| e.battle_id)
            .unwrap_or(0);

        configs
            .battle
            .iter()
            .find(|b| b.id == battle_id)
            .map(|b| {
                if b.monster_group_ids.is_empty() {
                    1
                } else {
                    b.monster_group_ids.split('#').count() as i32
                }
            })
            .unwrap_or(1)
    }

    pub(crate) fn expand_trigger_chain(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        root_step: &FightStep,
        runtime_deleted_buff_ids: &[i32],
    ) -> Vec<FightStep> {
        let mut out = vec![root_step.clone()];
        let mut event = event_from_step(
            ctx.fight,
            root_step.from_id.unwrap_or(0),
            root_step.to_id.unwrap_or(0),
            root_step.act_id.unwrap_or(0),
            &root_step.act_effect,
        );
        for buff_id in runtime_deleted_buff_ids {
            if *buff_id > 0 && !event.deleted_buff_ids.contains(buff_id) {
                event.deleted_buff_ids.push(*buff_id);
            }
        }
        let trigger_steps = fire_combat_triggers(ctx, collected, &event);
        for ts in trigger_steps {
            ctx.managers
                .calculate_mgr
                .play_step_data(
                    &ts,
                    ctx.fight,
                    &mut ctx.mechanics.bloodtithe,
                    &mut ctx.managers.buff_mgr,
                    &mut ctx.managers.ex_point_mgr,
                )
                .map_err(anyhow::Error::msg)
                .ok();
            let ts_event = event_from_step(
                ctx.fight,
                ts.from_id.unwrap_or(0),
                ts.to_id.unwrap_or(0),
                ts.act_id.unwrap_or(0),
                &ts.act_effect,
            );
            out.push(ts);
            if root_step.act_type == Some(fight_step::ActType::Effect.into()) {
                for &(team_type, gain) in &ts_event.bloodpool_gain_packets_by_team {
                    if let Some(sync_step) = build_belief_gain_step(ctx.fight, team_type, gain) {
                        ctx.managers
                            .calculate_mgr
                            .play_step_data(
                                &sync_step,
                                ctx.fight,
                                &mut ctx.mechanics.bloodtithe,
                                &mut ctx.managers.buff_mgr,
                                &mut ctx.managers.ex_point_mgr,
                            )
                            .map_err(anyhow::Error::msg)
                            .ok();
                        out.push(sync_step);
                    }
                }
            }
            let gains = [
                (1, ts_event.bloodpool_gain(1)),
                (2, ts_event.bloodpool_gain(2)),
            ];
            if let Some(sync_step) = build_blood_pool_gain_ex_point_step(
                &ctx.mechanics.bloodtithe,
                ctx.fight,
                &ctx.managers.buff_mgr,
                &mut ctx.managers.ex_point_mgr,
                &gains,
                &ts_event.bloodpool_gain_by_skill_team,
            ) {
                ctx.managers
                    .calculate_mgr
                    .play_step_data(
                        &sync_step,
                        ctx.fight,
                        &mut ctx.mechanics.bloodtithe,
                        &mut ctx.managers.buff_mgr,
                        &mut ctx.managers.ex_point_mgr,
                    )
                    .map_err(anyhow::Error::msg)
                    .ok();
                out.push(sync_step);
            }
        }
        out
    }

    pub(crate) fn deleted_buff_ids_from_delta(
        &self,
        before: &[(i64, super::buff_mgr::BuffInstance)],
        after: &[(i64, super::buff_mgr::BuffInstance)],
    ) -> Vec<i32> {
        let mut out = Vec::new();
        let after_keys: HashSet<(i64, i64)> = after.iter().map(|(uid, b)| (*uid, b.uid)).collect();
        for (uid, instance) in before {
            if !after_keys.contains(&(*uid, instance.uid)) {
                if instance.buff_id > 0 && !out.contains(&instance.buff_id) {
                    out.push(instance.buff_id);
                }
                if instance.type_id > 0 && !out.contains(&instance.type_id) {
                    out.push(instance.type_id);
                }
            }
        }
        out
    }

    fn run_passive_phase(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        config: PassivePhaseConfig,
    ) -> Vec<FightStep> {
        let scope_uids = match &config.scope {
            PhaseScope::Attackers => collected.attacker_uids(),
            PhaseScope::Defenders => collected.defender_uids(),
        };

        let mut steps = match config.skill_set {
            PhaseSkillSet::ExcludeBattleRule | PhaseSkillSet::CombatReactive => {
                let battle_rule_skills = self.collect_battle_rule_skills(ctx.fight);
                let stop_at_first = matches!(config.depth, PhaseDepth::FirstMatch);
                let passive_phase = PhaseFilter::combat();
                let is_defender_sweep = matches!(config.scope, PhaseScope::Defenders);
                let mut out = Vec::new();

                for &uid in &scope_uids {
                    let is_attacker_uid = self.uid_on_attacker_side(ctx.fight, uid);
                    let mut per_entity_effects: Vec<ActEffect> = Vec::new();
                    let mut skill_ids = collected.merged_for(uid);
                    self.extend_with_buff_granted_passives(ctx, uid, &mut skill_ids);
                    if !is_attacker_uid {
                        for sid in &battle_rule_skills {
                            if !skill_ids.contains(sid)
                                && !collected.battle_defender.contains(sid)
                                && !collected.battle_attacker.contains(sid)
                            {
                                skill_ids.push(*sid);
                            }
                        }
                    }
                    for skill_id in skill_ids {
                        if matches!(config.skill_set, PhaseSkillSet::CombatReactive)
                            && !has_combat_reactive_condition(
                                skill_id,
                                CombatPassiveScanMode::RoundSweep,
                            )
                        {
                            continue;
                        }
                        if is_attacker_uid && battle_rule_skills.contains(&skill_id) {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                            && !effects.is_empty()
                        {
                            // Defender-side idle sweeps in LIVE do not emit
                            // wrappers for state-machine passives whose only
                            // output is a BuffUpdate marker (e.g. 530000151
                            // cycling between 530000111/530000112 via
                            // NoBuffId gates). LIVE fires these nested inside
                            // actual combat events. Drop marker-only wrappers
                            // from defender sweeps so top-level OURS steps
                            // don't balloon with no-op state ticks.
                            let kept: Vec<ActEffect> = if is_defender_sweep
                                && has_combat_reactive_condition(
                                    skill_id,
                                    CombatPassiveScanMode::RoundSweep,
                                ) {
                                effects
                                    .into_iter()
                                    .filter(|e| !is_marker_only_fight_step_effect(e))
                                    .collect()
                            } else {
                                effects
                            };
                            if kept.is_empty() {
                                continue;
                            }
                            per_entity_effects.extend(kept);
                            if stop_at_first {
                                break;
                            }
                        }
                    }

                    if !per_entity_effects.is_empty() {
                        out.push(build_effect_step(per_entity_effects));
                    }
                }

                out
            }
            PhaseSkillSet::DefenderBootstrap => {
                let defender_uids = collected.defender_uids();
                let mut out = Vec::new();
                let defender_skill_set: std::collections::HashSet<i32> = defender_uids
                    .iter()
                    .flat_map(|uid| collected.merged_for(*uid))
                    .collect();
                let mut ordered_skills: Vec<i32> = Vec::new();

                let mut teammate_alive_skills: Vec<i32> = defender_skill_set
                    .iter()
                    .copied()
                    .filter(|sid| self.is_teammate_alive_self_addbuff(*sid))
                    .collect();
                teammate_alive_skills.sort_unstable();
                ordered_skills.extend(teammate_alive_skills);

                let mut battle_rule_skills: Vec<i32> = self
                    .collect_battle_rule_skills(ctx.fight)
                    .into_iter()
                    .filter(|sid| defender_skill_set.contains(sid))
                    .collect();
                battle_rule_skills.sort_unstable();
                for sid in battle_rule_skills {
                    if !ordered_skills.contains(&sid) {
                        ordered_skills.push(sid);
                    }
                }

                for skill_id in ordered_skills {
                    let mut wrapped = Vec::new();
                    for uid in &defender_uids {
                        let should_try = collected.merged_for(*uid).contains(&skill_id);
                        if !should_try {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &PhaseFilter::combat())
                            && !effects.is_empty()
                        {
                            let inner = build_effect_step(effects);
                            wrapped.push(wrap_step(inner));
                        }
                    }
                    if !wrapped.is_empty() {
                        out.push(build_effect_step(wrapped));
                    }
                }

                out
            }
            PhaseSkillSet::BattleRuleOnly => {
                let attacker_uids: Vec<i64> = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .map(|a| {
                        a.entitys
                            .iter()
                            .chain(a.sub_entitys.iter())
                            .filter(|e| {
                                e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0
                            })
                            .filter_map(|e| e.uid)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut out = Vec::new();
                let attacker_skill_set: std::collections::HashSet<i32> = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .map(|a| {
                        a.entitys
                            .iter()
                            .chain(a.sub_entitys.iter())
                            .filter(|e| {
                                e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0
                            })
                            .flat_map(|e| e.passive_skill.iter().copied())
                            .collect()
                    })
                    .unwrap_or_default();

                let mut battle_rule_skills: Vec<i32> = self
                    .collect_battle_rule_skills(ctx.fight)
                    .into_iter()
                    .filter(|sid| attacker_skill_set.contains(sid))
                    .filter(|sid| {
                        let effect_id = resolve_skill_effect_id(*sid);
                        let cfg = config::configs::get();
                        let cond = cfg
                            .skill_effect
                            .iter()
                            .find(|s| s.id == effect_id)
                            .map(|s| s.condition1.clone())
                            .unwrap_or_default();
                        let (parsed, _) = parse_condition(cond.trim());
                        !matches!(parsed, ConditionType::TargetCareer { .. })
                    })
                    .collect();
                battle_rule_skills.sort_unstable();

                for skill_id in battle_rule_skills {
                    let mut wrapped = Vec::new();
                    for uid in &attacker_uids {
                        let should_try = ctx
                            .fight
                            .attacker
                            .as_ref()
                            .map(|a| {
                                a.entitys
                                    .iter()
                                    .chain(a.sub_entitys.iter())
                                    .find(|e| e.uid == Some(*uid))
                                    .map(|e| e.passive_skill.contains(&skill_id))
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false);
                        if !should_try {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &PhaseFilter::combat())
                            && !effects.is_empty()
                        {
                            let inner = build_effect_step(effects);
                            wrapped.push(wrap_step(inner));
                        }
                    }
                    if !wrapped.is_empty() {
                        out.push(build_effect_step(wrapped));
                    }
                }

                out
            }
        };

        if matches!(config.step_shape, PhaseStepShape::FlatIfAllUpdate) {
            steps = split_updates_and_wrap_rest(steps);
        }

        steps
    }

    fn extend_with_buff_granted_passives(
        &self,
        ctx: &FightContext<'_>,
        uid: i64,
        skill_ids: &mut Vec<i32>,
    ) {
        for instance in ctx.managers.buff_mgr.get(uid) {
            crate::state::battle::utils::for_each_buff_feature_chain(
                instance.buff_id,
                |act_type, parts| {
                    let value_start_idx = match act_type {
                        "AddPassiveSkills" => 1,
                        "AddToTarget" | "AddToTargetNoLimit" | "UseDamageSkillAddToTarget" => 2,
                        _ => 0,
                    };
                    if value_start_idx == 0 {
                        return;
                    }
                    for raw in parts.iter().skip(value_start_idx) {
                        for piece in raw.split(',') {
                            if let Ok(skill_id) = piece.trim().parse::<i32>()
                                && skill_id > 0
                            {
                                let resolved_skill_id =
                                    resolve_with_euphoria(ctx.fight, uid, skill_id);
                                if !skill_ids.contains(&resolved_skill_id) {
                                    skill_ids.push(resolved_skill_id);
                                }
                            }
                        }
                    }
                },
            );
        }
    }

    fn uid_on_attacker_side(&self, fight: &Fight, uid: i64) -> bool {
        fight
            .attacker
            .as_ref()
            .map(|a| {
                a.entitys
                    .iter()
                    .chain(a.sub_entitys.iter())
                    .any(|e| e.uid == Some(uid))
            })
            .unwrap_or(false)
    }

    fn collect_battle_rule_skills(&self, fight: &Fight) -> std::collections::HashSet<i32> {
        let mut out = std::collections::HashSet::new();
        let episode_id = fight.episode_id.unwrap_or(0);
        let cfg = config::configs::get();
        let Some(battle_id) = cfg
            .episode
            .iter()
            .find(|e| e.id == episode_id)
            .map(|e| e.battle_id)
        else {
            return out;
        };
        let Some(battle) = cfg.battle.iter().find(|b| b.id == battle_id) else {
            return out;
        };
        if battle.addition_rule.is_empty() {
            return out;
        }

        for entry in battle.addition_rule.split('|') {
            let mut parts = entry.split('#');
            let Some(prefix) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            let Some(id) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            if !(1..=3).contains(&prefix) {
                continue;
            }
            let Some(rule) = cfg.rule.iter().find(|r| r.id == id) else {
                continue;
            };
            let sid = rule.effect.parse::<i32>().ok().unwrap_or(0);
            if sid != 0 {
                out.insert(sid);
            }
        }

        out
    }

    pub(crate) fn first_alive_defender_uid(&self, fight: &Fight) -> Option<i64> {
        fight
            .defender
            .as_ref()
            .and_then(|d| {
                d.entitys
                    .iter()
                    .chain(d.sub_entitys.iter())
                    .find(|e| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0)
            })
            .and_then(|e| e.uid)
    }

    fn is_teammate_alive_self_addbuff(&self, skill_id: i32) -> bool {
        let effect_id = resolve_skill_effect_id(skill_id);
        let cfg = config::configs::get();
        let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
            return false;
        };
        let (cond, _) = parse_condition(skill.condition1.trim());
        matches!(cond, ConditionType::TeammateAlive { .. })
            && skill.behavior_target1.trim() == "103"
            && skill.behavior1.trim().starts_with("1#")
    }
}
