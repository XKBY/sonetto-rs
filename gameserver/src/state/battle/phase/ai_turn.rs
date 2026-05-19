use anyhow::Result;
use rand::{Rng, rngs::StdRng};
use std::collections::HashMap;

use sonettobuf::{ActEffect, Fight, FightEntityInfo, FightStep, fight_step};

use crate::state::battle::{
    buff_actions::ex_point_overflow_bank::buff_get_ex_point_overflow,
    context::FightContext,
    fight_step::make_skill_step,
    manager::{fight_data_mgr::Managers, wave_mgr::WaveMgr},
    mechanics::Mechanics,
    round::RoundState,
    skill::{
        SkillExecutor,
        cache::SKILL_CACHE,
        euphoria::resolve_with_euphoria,
    },
    types::effects::EffectType,
    utils::{find_entity, find_entity_mut, gains_standard_action_ex},
};

pub(crate) async fn execute_ai_turn(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    state: &mut RoundState,
) -> Result<Vec<FightStep>> {
    let mut preview_fight = ctx.fight.clone();
    let mut preview_managers = ctx.managers.clone();
    let mut preview_mechanics = ctx.mechanics.clone();

    if let Some(override_steps) = state.ai_override_steps.as_ref() {
        return execute_ai_turn_replay(
            executor, rng, ctx, state, override_steps.clone(),
            &mut preview_fight, &mut preview_managers, &mut preview_mechanics,
        ).await;
    }

    execute_ai_turn_live(
        executor, rng, ctx, state,
        &mut preview_fight, &mut preview_managers, &mut preview_mechanics,
    ).await
}

async fn execute_ai_turn_replay(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    state: &mut RoundState,
    override_steps: Vec<FightStep>,
    preview_fight: &mut Fight,
    preview_managers: &mut Managers,
    preview_mechanics: &mut Mechanics,
) -> Result<Vec<FightStep>> {
    let players: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .filter(|e| e.current_hp.unwrap_or(0) > 0)
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    let mut steps = Vec::new();
    for step in &override_steps {
        let caster_uid = step.from_id.unwrap_or(0);
        let skill_id = step.act_id.unwrap_or(0);
        if caster_uid >= 0 || skill_id == 0 {
            continue;
        }
        if let Some(target_wave) = WaveMgr::expected_wave_for_uid(caster_uid) {
            let current_wave = preview_fight.cur_wave.unwrap_or(1);
            let max_wave = WaveMgr::max_wave_for_fight(preview_fight);
            let replay_snapshot_covers_wave = state.replay_wave_snapshot_applied
                && state.replay_wave_snapshot_target_wave.unwrap_or(0) >= target_wave;
            if !replay_snapshot_covers_wave
                && target_wave > current_wave
                && target_wave <= max_wave
            {
                let mut wave_executor = SkillExecutor::new();
                let mut wave_ctx = FightContext::new(
                    preview_fight,
                    preview_managers,
                    preview_mechanics,
                );
                let mut wave_mgr = std::mem::take(&mut wave_ctx.managers.wave_mgr);
                wave_mgr.fast_forward_to_wave(
                    &mut wave_ctx,
                    &mut wave_executor,
                    target_wave,
                )?;
                wave_ctx.managers.wave_mgr = wave_mgr;
                drop(wave_ctx);
            }
        }
        let caster_alive = preview_fight
            .defender
            .as_ref()
            .map(|d| {
                d.entitys
                    .iter()
                    .find(|e| e.uid == Some(caster_uid))
                    .map(|e| e.current_hp.unwrap_or(0) > 0)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if caster_alive {
            let target_uid = match step.to_id.unwrap_or(0) {
                0 => {
                    if players.is_empty() {
                        continue;
                    }
                    players[rng.gen_range(0..players.len())]
                }
                t => t,
            };
            if !entity_exists(preview_fight, caster_uid) {
                tracing::warn!(
                    "ai_override skip step caster={} target={} skill={} reason=entity_missing",
                    caster_uid, target_uid, skill_id
                );
                continue;
            }
            if target_uid > 0 && !entity_exists(preview_fight, target_uid) {
                tracing::debug!(
                    target: "ai_replay_skip",
                    "ai_override skip step caster={} target={} skill={} reason=positive_target_missing",
                    caster_uid, target_uid, skill_id
                );
                continue;
            }
            preview_managers.buff_mgr.clear_step_deleted_buff_ids();
            let resolved_skill_id =
                resolve_with_euphoria(preview_fight, caster_uid, skill_id);
            let replay_damage_targets = replay_primary_damage_targets(&step.act_effect);
            if !replay_damage_targets.is_empty() {
                executor.set_override_damage_targets(replay_damage_targets);
            }
            let per_behavior = executor.execute_skill(
                rng,
                preview_fight,
                preview_managers,
                preview_mechanics,
                caster_uid,
                target_uid,
                resolved_skill_id,
                &ctx.combat_phase(),
            )?;
            let pending_summons = executor.take_pending_summons();
            SkillExecutor::apply_summon_batch(
                preview_fight,
                preview_managers,
                &pending_summons,
            )?;
            SkillExecutor::apply_summon_batch(ctx.fight, ctx.managers, &pending_summons)?;
            let mut op_effects = normalize_skill_effects_for_operation(
                per_behavior, caster_uid, resolved_skill_id,
            );
            clamp_ai_add_ex_with_max_effects(
                preview_fight, preview_managers, caster_uid, &mut op_effects,
            );
            if op_effects.is_empty() {
                continue;
            }
            let step =
                make_skill_step(caster_uid, target_uid, resolved_skill_id, 0, op_effects);
            advance_ai_preview_after_cast(preview_fight, preview_managers, caster_uid, &step);
            steps.push(step);
        }
    }
    Ok(steps)
}

async fn execute_ai_turn_live(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    state: &mut RoundState,
    preview_fight: &mut Fight,
    preview_managers: &mut Managers,
    preview_mechanics: &mut Mechanics,
) -> Result<Vec<FightStep>> {
    #[derive(Clone)]
    struct AiCast {
        idx: usize,
        caster_uid: i64,
        skill_id: i32,
        target_uid_opt: Option<i64>,
    }

    let mut steps = Vec::new();
    let players: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .filter(|e| e.current_hp.unwrap_or(0) > 0)
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    if players.is_empty() {
        return Ok(steps);
    }

    let mut candidates: Vec<AiCast> = Vec::new();
    for i in 0..state.ai_use_cards.len() {
        let (caster_uid, raw_skill_id, target_uid_opt) = {
            let card = &state.ai_use_cards[i];
            (
                card.uid.unwrap_or(0),
                card.skill_id.unwrap_or(0),
                card.target_uid,
            )
        };
        if caster_uid >= 0 || raw_skill_id == 0 {
            continue;
        }

        let caster_alive = ctx
            .fight
            .defender
            .as_ref()
            .map(|d| {
                d.entitys
                    .iter()
                    .find(|e| e.uid == Some(caster_uid))
                    .map(|e| e.current_hp.unwrap_or(0) > 0)
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if !caster_alive {
            continue;
        }
        let resolved_skill_id = {
            let has_behaviors = SKILL_CACHE
                .get(&raw_skill_id)
                .map(|b| !b.is_empty())
                .unwrap_or(false);
            if has_behaviors {
                raw_skill_id
            } else {
                let fallback = raw_skill_id - 1;
                let fallback_has_behaviors = SKILL_CACHE
                    .get(&fallback)
                    .map(|b| !b.is_empty())
                    .unwrap_or(false);
                if fallback_has_behaviors {
                    fallback
                } else {
                    raw_skill_id
                }
            }
        };

        let canonical_caster_uid =
            canonical_ai_caster_uid(ctx.fight, caster_uid, resolved_skill_id)
                .unwrap_or(caster_uid);

        candidates.push(AiCast {
            idx: i,
            caster_uid: canonical_caster_uid,
            skill_id: resolved_skill_id,
            target_uid_opt,
        });
    }

    let mut caster_base_choice: HashMap<(i64, i32), AiCast> = HashMap::new();
    for cast in candidates {
        let base = cast.skill_id / 10;
        caster_base_choice
            .entry((cast.caster_uid, base))
            .and_modify(|current| {
                if cast.skill_id < current.skill_id
                    || (cast.skill_id == current.skill_id && cast.idx < current.idx)
                {
                    *current = cast.clone();
                }
            })
            .or_insert(cast);
    }

    let mut chosen_by_skill: HashMap<i32, AiCast> = HashMap::new();
    for cast in caster_base_choice.into_values() {
        chosen_by_skill
            .entry(cast.skill_id)
            .and_modify(|current| {
                if cast.caster_uid > current.caster_uid
                    || (cast.caster_uid == current.caster_uid && cast.idx < current.idx)
                {
                    *current = cast.clone();
                }
            })
            .or_insert(cast);
    }

    let mut casts: Vec<AiCast> = chosen_by_skill.into_values().collect();
    casts.sort_by(|a, b| b.caster_uid.cmp(&a.caster_uid).then(a.idx.cmp(&b.idx)));

    for cast in casts {
        let i = cast.idx;
        let caster_uid = cast.caster_uid;
        let skill_id = cast.skill_id;
        let target_uid_opt = cast.target_uid_opt;

        preview_managers.buff_mgr.clear_step_deleted_buff_ids();

        let target_uid = match target_uid_opt {
            Some(t) if t != 0 => resolve_target_fallback(preview_fight, t),
            _ => {
                let t = players[rng.gen_range(0..players.len())];
                state.ai_use_cards[i].target_uid = Some(t);
                t
            }
        };

        let resolved_skill_id = resolve_with_euphoria(preview_fight, caster_uid, skill_id);
        let per_behavior = executor.execute_skill(
            rng,
            preview_fight,
            preview_managers,
            preview_mechanics,
            caster_uid,
            target_uid,
            resolved_skill_id,
            &ctx.combat_phase(),
        )?;
        let pending_summons = executor.take_pending_summons();
        SkillExecutor::apply_summon_batch(
            preview_fight,
            preview_managers,
            &pending_summons,
        )?;
        SkillExecutor::apply_summon_batch(ctx.fight, ctx.managers, &pending_summons)?;
        let mut op_effects =
            normalize_skill_effects_for_operation(per_behavior, caster_uid, resolved_skill_id);
        clamp_ai_add_ex_with_max_effects(
            preview_fight, preview_managers, caster_uid, &mut op_effects,
        );
        if op_effects.is_empty() {
            continue;
        }

        let step = make_skill_step(caster_uid, target_uid, resolved_skill_id, 0, op_effects);
        advance_ai_preview_after_cast(preview_fight, preview_managers, caster_uid, &step);
        steps.push(step);
    }
    Ok(steps)
}

fn resolve_target_fallback(fight: &Fight, requested_uid: i64) -> i64 {
    use crate::state::battle::skill::targets;
    targets::resolve_target_fallback(fight, requested_uid)
}

fn entity_exists(fight: &Fight, uid: i64) -> bool {
    find_entity(fight, uid).is_some()
}

fn replay_primary_damage_targets(effects: &[ActEffect]) -> Vec<i64> {
    let mut targets = Vec::new();
    for effect in effects {
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if !matches!(
            effect_type,
            x if x == EffectType::Damage as i32
                || x == EffectType::Crit as i32
                || x == EffectType::DamageExtra as i32
        ) {
            continue;
        }
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        if target_uid <= 0 || targets.contains(&target_uid) {
            continue;
        }
        targets.push(target_uid);
    }
    targets
}

fn normalize_skill_effects_for_operation(
    effects: Vec<ActEffect>,
    caster_uid: i64,
    skill_id: i32,
) -> Vec<ActEffect> {
    if effects.is_empty() {
        return effects;
    }

    let mut out = Vec::new();
    let mut iter = effects.into_iter();

    if let Some(mut first) = iter.next() {
        let first_fight_step = first.fight_step.take();
        let inline_root = first_fight_step.as_ref().is_some_and(|step| {
            first.effect_type == Some(EffectType::FightStep as i32)
                && step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.act_id == Some(skill_id)
                && (step.from_id == Some(caster_uid) || step.from_id == Some(0))
        });
        if inline_root {
            let step = first_fight_step.expect("inline_root checked first_fight_step presence");
            out.extend(step.act_effect);
        } else {
            out.push(first);
        }
    }

    out.extend(iter);
    out
}

fn canonical_ai_caster_uid(
    fight: &Fight,
    caster_uid: i64,
    skill_id: i32,
) -> Option<i64> {
    let defender = fight.defender.as_ref()?;
    let mut owners: Vec<&FightEntityInfo> = defender
        .entitys
        .iter()
        .filter(|e| e.current_hp.unwrap_or(0) > 0)
        .filter(|e| {
            e.skill_group1.contains(&skill_id)
                || e.skill_group2.contains(&skill_id)
                || e.ex_skill == Some(skill_id)
        })
        .collect();

    if owners.len() <= 1 {
        return Some(caster_uid);
    }

    owners.sort_by_key(|e| e.position.unwrap_or(i32::MAX));
    owners.first().and_then(|e| e.uid).or(Some(caster_uid))
}

fn advance_ai_preview_after_cast(
    fight: &mut Fight,
    managers: &mut Managers,
    caster_uid: i64,
    step: &FightStep,
) {
    if gains_standard_action_ex(fight, caster_uid) {
        apply_preview_ex_delta(fight, managers, caster_uid, 1);
    }
    apply_preview_ex_effects(fight, managers, &step.act_effect);
}

fn clamp_ai_add_ex_with_max_effects(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
    effects: &mut [ActEffect],
) {
    for effect in effects {
        if let Some(nested) = effect.fight_step.as_mut() {
            clamp_ai_add_ex_with_max_effects(fight, managers, caster_uid, &mut nested.act_effect);
            continue;
        }

        let effect_type = EffectType::from(effect.effect_type.unwrap_or(0));
        if !matches!(
            effect_type,
            EffectType::AddExPoint | EffectType::ExPointChange
        ) {
            continue;
        }
        if effect.config_effect != Some(20002) {
            continue;
        }

        let Some(target_id) = effect.target_id else {
            continue;
        };
        let raw = effect.effect_num.unwrap_or(0);
        if raw <= 0 {
            continue;
        }

        let Some(entity) = find_entity(fight, target_id) else {
            continue;
        };
        let base_max = match entity.ex_point_type.unwrap_or(0) {
            0 => 5,
            1 => 8,
            _ => 0,
        };
        if base_max <= 0 {
            continue;
        }

        let overflow_bonus = managers
            .buff_mgr
            .get(target_id)
            .iter()
            .find_map(|buff| buff_get_ex_point_overflow(buff.buff_id))
            .unwrap_or(0);
        let pending_standard_gain =
            (target_id == caster_uid && gains_standard_action_ex(fight, caster_uid)) as i32;
        let current = entity.ex_point.unwrap_or(0) + pending_standard_gain;
        let allowed = (base_max + overflow_bonus - current).max(0);
        effect.effect_num = Some(raw.min(allowed));
    }
}

fn apply_preview_ex_effects(fight: &mut Fight, managers: &mut Managers, effects: &[ActEffect]) {
    for effect in effects {
        if let Some(nested) = effect.fight_step.as_ref() {
            apply_preview_ex_effects(fight, managers, &nested.act_effect);
            continue;
        }

        match EffectType::from(effect.effect_type.unwrap_or(0)) {
            EffectType::AddExPoint | EffectType::ExPointChange => {
                if let Some(target_id) = effect.target_id {
                    apply_preview_ex_delta(fight, managers, target_id, effect.effect_num.unwrap_or(0));
                }
            }
            EffectType::ExPointDel => {
                if let Some(target_id) = effect.target_id {
                    apply_preview_ex_delta(fight, managers, target_id, -effect.effect_num.unwrap_or(0).max(0));
                }
            }
            _ => {}
        }
    }
}

fn apply_preview_ex_delta(fight: &mut Fight, managers: &mut Managers, target_id: i64, delta: i32) {
    let Some(entity) = find_entity_mut(fight, target_id) else {
        return;
    };

    let overflow_bonus = managers
        .buff_mgr
        .get(target_id)
        .iter()
        .find_map(|buff| buff_get_ex_point_overflow(buff.buff_id))
        .unwrap_or(0);

    let base_max = match entity.ex_point_type.unwrap_or(0) {
        0 => 5,
        1 => 8,
        _ => 0,
    };
    let old = entity.ex_point.unwrap_or(0);
    let new = if base_max > 0 {
        (old + delta).clamp(0, base_max + overflow_bonus)
    } else {
        (old + delta).max(0)
    };

    entity.ex_point = Some(new);
    managers.ex_point_mgr.set_ex_point(target_id, new);
}
