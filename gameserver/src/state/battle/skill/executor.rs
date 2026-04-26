use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::{ActEffect, BuffInfo, Fight, FightStep, fight_step};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    time::Instant,
};

use super::super::{
    context::{FightContext, behavior_context::BehaviorContext},
    fight_step::ActEffectBuilder,
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        fight_data_mgr::Managers,
    },
    mechanics::Mechanics,
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
    utils::buff_del,
};

use super::{
    behavior::execute_behavior,
    cache::{SKILL_CACHE, resolve_skill_effect_id},
    condition::{self, ConditionEval, buff::deleted_matches},
    damage::{calculate_damage, should_crit_hit},
    euphoria,
    phase::{PhaseFilter, TriggerState},
    targets::{
        TargetResolver, alive_enemies, alive_enemies_by_position, get_ally_uids, get_entity,
    },
};

#[derive(Default, Debug, Clone)]
pub struct SkillExecutor {
    pub side_effects: Vec<ActEffect>,
    /// (caster_uid, trigger_skill_id) pairs queued by RaspberryBigSkill/CreateMaxHpAdditionalDamageAndRemove
    pub pending_monitor_triggers: Vec<(i64, i32)>,
    /// (target_uid, buff_id) pairs for buffs that self-delete after use (e.g. AttrFromEntity)
    pub pending_buff_dels: Vec<(i64, i32)>,
    /// Per-target damage-rate bonus accumulated by SkillRateUp-like behaviors.
    pub pending_target_rate_bonus: HashMap<i64, i32>,
    /// Global damage-rate bonus applied to all fallback damage targets.
    pub pending_global_rate_bonus: i32,
    /// Per-entity temporary attribute bonuses for this skill execution.
    /// Key: (entity_uid, attr_id)
    pub pending_attr_bonus: HashMap<(i64, i32), i32>,
    /// Per-team preview of bloodtithe `(value, accumulator)` for this skill execution.
    /// This lets combat damage emit live-like positive 335 packets without mutating
    /// authoritative bloodtithe state before play_step_data replays the step.
    pub pending_bloodtithe_preview: HashMap<i32, (i32, i32)>,
    call_depth: usize,
}

struct DepthGuard {
    depth: *mut usize,
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        // SAFETY: `depth` points to `self.call_depth` for the lifetime of execute_skill.
        unsafe {
            *self.depth = (*self.depth).saturating_sub(1);
        }
    }
}

thread_local! {
    static EXEC_SKILL_STACK: RefCell<Vec<(i64, i64, i32)>> = const { RefCell::new(Vec::new()) };
}

struct ReentryGuard {
    active: bool,
}

impl ReentryGuard {
    fn enter(caster_uid: i64, target_uid: i64, skill_id: i32) -> Option<Self> {
        let mut entered = false;
        EXEC_SKILL_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            let key = (caster_uid, target_uid, skill_id);
            if stack.contains(&key) {
                return;
            }
            stack.push(key);
            entered = true;
        });
        if entered {
            Some(Self { active: true })
        } else {
            None
        }
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        EXEC_SKILL_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            stack.pop();
        });
    }
}

fn merge_duplicate_pickles_child_steps(effect_steps: &mut Vec<ActEffect>, child_act_id: i32) {
    let mut first_idx = None;
    let mut duplicate_indices = Vec::new();

    for (idx, effect) in effect_steps.iter().enumerate() {
        let act_id = effect
            .fight_step
            .as_ref()
            .and_then(|step| step.act_id)
            .unwrap_or_default();
        if act_id != child_act_id {
            continue;
        }
        if first_idx.is_none() {
            first_idx = Some(idx);
        } else {
            duplicate_indices.push(idx);
        }
    }

    let Some(first_idx) = first_idx else {
        return;
    };
    if duplicate_indices.is_empty() {
        return;
    }

    let mut merged_nested = Vec::new();
    for &idx in &duplicate_indices {
        if let Some(step) = effect_steps[idx].fight_step.as_ref() {
            merged_nested.extend(step.act_effect.clone());
        }
    }

    if let Some(step) = effect_steps[first_idx].fight_step.as_mut() {
        step.act_effect.extend(merged_nested);
    }

    for &idx in duplicate_indices.iter().rev() {
        effect_steps.remove(idx);
    }
}

// TODO(event-queue): post-execution coalescer for Pickles 30630151 fanout
// (see `09c4d5ed`). The coalescing IS the right semantic merge but it's
// applied AFTER both behavior slots have already serialized into separate
// SKILL wrappers. With EventQueue Phase 4 + Phase 5, the merge happens
// during drain when sibling SkillEmit events with matching act_id share
// the same parent — making this fn obsolete. See `_eventqueue_design.md`.
fn coalesce_pickles_30630151_wrappers(skill_id: i32, effect_steps: &mut Vec<ActEffect>) {
    if skill_id != 30630151 {
        return;
    }

    let count_30630122 = effect_steps
        .iter()
        .filter(|effect| effect.fight_step.as_ref().and_then(|step| step.act_id) == Some(30630122))
        .count();
    let count_30630161 = effect_steps
        .iter()
        .filter(|effect| effect.fight_step.as_ref().and_then(|step| step.act_id) == Some(30630161))
        .count();

    if count_30630122 < 2 || count_30630161 < 2 {
        return;
    }

    merge_duplicate_pickles_child_steps(effect_steps, 30630122);
    merge_duplicate_pickles_child_steps(effect_steps, 30630161);
}

impl SkillExecutor {
    pub fn new() -> Self {
        Self {
            side_effects: Vec::new(),
            pending_monitor_triggers: Vec::new(),
            pending_buff_dels: Vec::new(),
            pending_target_rate_bonus: HashMap::new(),
            pending_global_rate_bonus: 0,
            pending_attr_bonus: HashMap::new(),
            pending_bloodtithe_preview: HashMap::new(),
            call_depth: 0,
        }
    }

    #[allow(clippy::too_many_arguments, clippy::extend_with_drain)]
    pub fn execute_skill(
        &mut self,
        rng: &mut StdRng,
        fight: &Fight,
        managers: &mut Managers,
        mechanics: &mut Mechanics,
        caster_uid: i64,
        target_uid: i64,
        skill_id: i32,
        phase: &PhaseFilter,
    ) -> Result<Vec<ActEffect>> {
        let exec_start = Instant::now();
        let skill_id = euphoria::resolve_with_euphoria(fight, caster_uid, skill_id);
        let Some(_reentry_guard) = ReentryGuard::enter(caster_uid, target_uid, skill_id) else {
            tracing::warn!(
                "[execute_skill] reentry loop blocked at skill={} caster={} target={}",
                skill_id,
                caster_uid,
                target_uid
            );
            return Ok(vec![]);
        };

        self.call_depth += 1;
        let _depth_guard = DepthGuard {
            depth: &mut self.call_depth as *mut usize,
        };
        if self.call_depth > 64 {
            tracing::warn!(
                "[execute_skill] depth limit reached at skill={} caster={} target={}, skipping",
                skill_id,
                caster_uid,
                target_uid
            );
            return Ok(vec![]);
        }

        let skill_effect_id = resolve_skill_effect_id(skill_id);
        let cfg = config::configs::get();
        let skill_cfg = cfg.skill_effect.get(skill_effect_id);
        self.pending_target_rate_bonus.clear();
        self.pending_global_rate_bonus = 0;
        self.pending_attr_bonus.clear();
        let behaviors = SKILL_CACHE
            .get(&skill_effect_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        tracing::info!(
            "[execute_skill] skill={} caster={} target={} phase={:?} behaviors={}",
            skill_id,
            caster_uid,
            target_uid,
            phase,
            behaviors.len()
        );
        let mut all_effects: Vec<ActEffect> = Vec::new();
        let mut force_effect_step = false;
        let setup_done = Instant::now();
        let mut sim_fight = fight.clone();
        let mut sim_buff_mgr = managers.buff_mgr.clone();
        let clones_done = Instant::now();

        // Conditions should see evolving buff state produced by prior behavior slots.
        let has_trigger_state = matches!(phase, PhaseFilter::Combat(_));
        let execution_order = behavior_execution_order(behaviors);

        // Preserve config slot order; each slot condition still evaluates against skill-entry snapshot.
        for &behavior_idx in &execution_order {
            let b = &behaviors[behavior_idx];
            let slot_index = (behavior_idx + 1) as u8;
            tracing::debug!(
                "  [behavior {}] condition={:?} behavior={:?} behavior_target={} condition_target={} logic_target={}",
                behavior_idx + 1,
                b.condition,
                b.behavior,
                b.behavior_target,
                b.condition_target,
                b.logic_target
            );
            if let PhaseFilter::Combat(event) = phase
                && event.event_driven_only
                && !condition_has_combat_event(&b.condition)
            {
                continue;
            }

            if b.round_limit > 0 {
                let used = managers.buff_mgr.skill_slot_round_usage(
                    caster_uid,
                    skill_effect_id,
                    slot_index,
                );
                if used >= b.round_limit {
                    tracing::debug!(
                        "  [behavior {}] skipped by round_limit={} used={}",
                        behavior_idx + 1,
                        b.round_limit,
                        used
                    );
                    continue;
                }
            }

            if !phase.check(&b.condition, b.behavior_target) {
                continue;
            }

            if !phase.allows_behavior(&b.behavior) {
                continue;
            }

            let cond_pass = if let PhaseFilter::Combat(event) = phase {
                let combat_raw = match &b.condition {
                    ConditionType::None | ConditionType::CombatNone => Some(true),
                    ConditionType::ActiveUseSkill => Some(event.active_use_skill),
                    ConditionType::ActiveUseSkillId { skill_ids } => {
                        Some(event.active_use_skill && skill_ids.contains(&event.skill_id))
                    }
                    ConditionType::UseExSkill => {
                        Some(event.active_use_skill && event.used_ex_skill)
                    }
                    ConditionType::TeammateUseExSkill => Some(event.teammate_use_ex_skill),
                    ConditionType::BeAttacked => Some(event.be_attacked),
                    ConditionType::HurtNotRestraint => Some(event.hurt_not_restraint),
                    ConditionType::HurtRestraint => Some(event.hurt_restraint),
                    ConditionType::TeammateInjuryCount { threshold } => {
                        Some(event.teammate_injury_count >= *threshold)
                    }
                    ConditionType::TeammateInjuryCountNotReset { threshold } => {
                        Some(event.teammate_injury_count_not_reset >= *threshold)
                    }
                    ConditionType::TeamInjuryCountRound => Some(event.team_injury_count_round),
                    ConditionType::BuffIdDel { buff_ids } => {
                        Some(deleted_matches(&event.deleted_buff_ids, buff_ids))
                    }
                    ConditionType::TriggerBullet => Some(event.trigger_bullet),
                    ConditionType::NoActRound => Some(!event.active_use_skill),
                    // All stateful/static conditions (HasBuffId/NoBuffId/TargetCareer/CareerCheck/etc.)
                    // must be evaluated against live fight state in check_condition.
                    _ => None,
                };
                if let Some(raw) = combat_raw {
                    if b.negated { !raw } else { raw }
                } else {
                    let condition_uid = if b.condition_target != 0 {
                        TargetResolver::new(&sim_fight, caster_uid, target_uid)
                            .behavior(b.condition_target)
                            .logic(b.logic_target)
                            .resolve()
                            .into_iter()
                            .next()
                            .unwrap_or(caster_uid)
                    } else {
                        caster_uid
                    };
                    let condition_eval = ConditionEval::new(
                        &sim_fight,
                        &sim_buff_mgr,
                        &managers.ex_point_mgr,
                        &mechanics.bloodtithe,
                        caster_uid,
                    )
                    .with_trigger_state(has_trigger_state);
                    let raw = if has_trigger_state
                        && b.condition_target == 103
                        && target_uid != 0
                        && target_uid.signum() != caster_uid.signum()
                        && condition_has_trigger_bullet_and_random(&b.condition)
                    {
                        condition_eval
                            .for_target(condition_uid)
                            .check_with_random_target(target_uid, &b.condition)
                    } else {
                        condition_eval.for_target(condition_uid).check(&b.condition)
                    };
                    let raw = apply_no_act_seed_hint(
                        &sim_fight,
                        caster_uid,
                        condition_uid,
                        &b.condition,
                        raw,
                    );
                    let raw =
                        if b.condition_target == 103 && b.logic_target == 201 && target_uid != 0 {
                            match &b.condition {
                                ConditionType::HasBuffId { .. } => {
                                    raw || condition_eval.for_target(target_uid).check(&b.condition)
                                }
                                ConditionType::NoBuffId { .. } => {
                                    raw && condition_eval.for_target(target_uid).check(&b.condition)
                                }
                                _ => raw,
                            }
                        } else {
                            raw
                        };
                    if b.negated { !raw } else { raw }
                }
            } else {
                let condition_uid = if b.condition_target != 0 {
                    TargetResolver::new(&sim_fight, caster_uid, target_uid)
                        .behavior(b.condition_target)
                        .logic(b.logic_target)
                        .resolve()
                        .into_iter()
                        .next()
                        .unwrap_or(caster_uid)
                } else {
                    caster_uid
                };
                let condition_eval = ConditionEval::new(
                    &sim_fight,
                    &sim_buff_mgr,
                    &managers.ex_point_mgr,
                    &mechanics.bloodtithe,
                    caster_uid,
                )
                .with_trigger_state(has_trigger_state);
                let raw = condition_eval.for_target(condition_uid).check(&b.condition);
                let raw = apply_no_act_seed_hint(
                    &sim_fight,
                    caster_uid,
                    condition_uid,
                    &b.condition,
                    raw,
                );
                let raw = if b.condition_target == 103 && b.logic_target == 201 && target_uid != 0 {
                    match &b.condition {
                        ConditionType::HasBuffId { .. } => {
                            raw || condition_eval.for_target(target_uid).check(&b.condition)
                        }
                        ConditionType::NoBuffId { .. } => {
                            raw && condition_eval.for_target(target_uid).check(&b.condition)
                        }
                        _ => raw,
                    }
                } else {
                    raw
                };
                if b.negated { !raw } else { raw }
            };

            tracing::debug!(
                "    -> condition check: {}{}",
                if b.negated { "!" } else { "" },
                if cond_pass { "PASS" } else { "FAIL" }
            );
            if !cond_pass {
                continue;
            }

            if matches!(&b.behavior, BehaviorType::LostLife { mode: 1, .. }) {
                force_effect_step = true;
            }

            let behavior_ctx = BehaviorContext::new(
                &sim_fight,
                caster_uid,
                target_uid,
                skill_id,
                slot_index,
                b.behavior_target,
                b.condition_target,
                b.logic_target,
                phase,
            );
            let behavior_effects = execute_behavior(
                self,
                rng,
                managers,
                mechanics,
                &behavior_ctx,
                &b.behavior,
                b.condition_id,
                &b.condition,
            )?;
            managers.buff_mgr.increment_skill_slot_round_usage(
                caster_uid,
                skill_effect_id,
                slot_index,
            );

            tracing::debug!("    -> effects built: {}", behavior_effects.len());
            for e in &behavior_effects {
                tracing::trace!(
                    "       effect_type={:?} target={:?} num={:?}",
                    e.effect_type,
                    e.target_id,
                    e.effect_num
                );
            }
            apply_preview_effects_to_sim_fight(&mut sim_fight, &behavior_effects);
            apply_preview_effects_to_sim_buffs(&mut sim_buff_mgr, &behavior_effects);
            // Nested DirectUseSkill execution reads managers.buff_mgr directly.
            // Keep managers in lockstep with previewed buff deltas so recursive
            // behavior slots see the same buff state as this skill chain.
            apply_preview_effects_to_sim_buffs(&mut managers.buff_mgr, &behavior_effects);
            all_effects.extend(behavior_effects);
        }
        let behaviors_done = Instant::now();

        // fallback: damage_rate should still fire if no Damage effect was emitted,
        // unless explicitly disabled by IgnoreSkillConfigDamageRate behavior.
        if let Some(skill) = skill_cfg
            && skill.damage_rate > 0
        {
            let has_damage_effect = all_effects
                .iter()
                .any(|e| e.effect_type.map(is_damage_effect_type).unwrap_or(false));
            let ignore_config_damage = behaviors.iter().any(|b| {
                matches!(
                    b.behavior,
                    super::super::types::behavior::BehaviorType::IgnoreSkillConfigDamageRate
                )
            });

            if !has_damage_effect && !ignore_config_damage {
                let mut damage_effects = Vec::new();
                for dmg_target in fallback_damage_targets(
                    &sim_fight,
                    caster_uid,
                    target_uid,
                    skill.logic_target.trim().parse::<i32>().unwrap_or(0),
                ) {
                    let bonus = self.pending_global_rate_bonus
                        + self
                            .pending_target_rate_bonus
                            .get(&dmg_target)
                            .copied()
                            .unwrap_or(0);
                    let final_rate = (skill.damage_rate + bonus).max(0);
                    let is_crit = should_crit_hit(
                        &sim_fight,
                        &managers.buff_mgr,
                        Some(&self.pending_attr_bonus),
                        caster_uid,
                        dmg_target,
                        skill_id,
                    );
                    damage_effects.extend(calculate_damage(
                        &sim_fight,
                        &managers.buff_mgr,
                        Some(&self.pending_attr_bonus),
                        caster_uid,
                        dmg_target,
                        final_rate,
                        skill_id,
                        is_crit,
                    ));
                }
                damage_effects.extend(all_effects);
                all_effects = damage_effects;
            }
        }

        let dead_effects = collect_dead_effects_after_damage(&sim_fight, &all_effects);
        if !dead_effects.is_empty() {
            all_effects.extend(dead_effects);
        }

        // Some temporary offense buffs are consumed when the owner deals damage
        // (e.g. buff 301 / AttrOnlyCalDamageAttack), emitted as inline 162 steps.
        if all_effects
            .iter()
            .any(|e| e.effect_type.map(is_damage_effect_type).unwrap_or(false))
        {
            let mut consume_targets = vec![caster_uid];
            for target in all_effects.iter().filter_map(|e| {
                let et = e.effect_type.unwrap_or(0);
                if is_damage_effect_type(et) {
                    e.target_id
                } else {
                    None
                }
            }) {
                if !consume_targets.contains(&target) {
                    consume_targets.push(target);
                }
            }
            let mut consume_steps = Vec::new();
            for uid in consume_targets {
                consume_steps.extend(self.consume_attr_only_damage_buffs(managers, uid));
            }
            all_effects.extend(consume_steps);
        }

        // Prevent self-nested skill emission: if behavior output already includes a
        // same-act_id FightStep carrying damage, lift its payload into this skill step.
        let mut normalized_effects = Vec::with_capacity(all_effects.len());
        for mut effect in all_effects.drain(..) {
            if effect.effect_type == Some(EffectType::FightStep as i32)
                && let Some(step) = effect.fight_step.take()
            {
                if step.act_id == Some(skill_id)
                    && step
                        .act_effect
                        .iter()
                        .any(|e| e.effect_type.is_some_and(is_damage_effect_type))
                {
                    normalized_effects.extend(step.act_effect);
                    continue;
                }
                effect.fight_step = Some(step);
            }
            normalized_effects.push(effect);
        }
        coalesce_pickles_30630151_wrappers(skill_id, &mut normalized_effects);
        all_effects = normalized_effects;

        if all_effects.is_empty()
            && self.pending_monitor_triggers.is_empty()
            && self.side_effects.is_empty()
        {
            tracing::info!("[execute_skill] skill={} no effects fired", skill_id);
            return Ok(vec![]);
        }

        // If the skill's only emitted effects are Attr-update markers with
        // effect_num == 0 (e.g. AttrFix-only passives like 71004), the state
        // change is already applied to the executor's pending_attr_bonus and
        // LIVE does not emit a visible 162 wrapper. Suppress the container
        // so these passives don't over-fire in the skill-count walker.
        let all_attr_only = !all_effects.is_empty()
            && all_effects.iter().all(|e| {
                e.effect_type == Some(EffectType::Attr as i32) && e.effect_num.unwrap_or(0) == 0
            });
        if all_attr_only && self.pending_monitor_triggers.is_empty() && self.side_effects.is_empty()
        {
            return Ok(vec![]);
        }

        let logic_to_id = skill_cfg
            .and_then(|s| {
                let lt = s.logic_target.trim();
                if lt.is_empty() {
                    None
                } else {
                    lt.parse::<i32>().ok()
                }
            })
            .and_then(|target_type| {
                TargetResolver::new(&sim_fight, caster_uid, target_uid)
                    .behavior(target_type)
                    .resolve()
                    .into_iter()
                    .next()
            })
            .unwrap_or(target_uid);

        let mut step_act_type = fight_step::ActType::Skill.into();
        if force_effect_step
            && let PhaseFilter::Combat(event) = phase
            && !event.active_use_skill
        {
            step_act_type = fight_step::ActType::Effect.into();
        }
        let total_inner_effects = all_effects.len();

        let skill_step = FightStep {
            act_type: Some(step_act_type),
            from_id: Some(caster_uid),
            to_id: Some(logic_to_id),
            act_id: Some(skill_id),
            act_effect: all_effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        };

        let mut skill_act_effect = ActEffectBuilder::new(EffectType::FightStep as i32, 0)
            .effect_num(0)
            .fight_step(skill_step)
            .build();

        let mut result = Vec::new();

        // Fire pending monitor triggers (CreateMaxHpAdditionalDamageAndRemove) as
        // inline 162 steps BEFORE the main skill step, matching live order.
        let triggers: Vec<(i64, i32)> = self.pending_monitor_triggers.drain(..).collect();
        for (trigger_uid, trigger_skill_id) in triggers {
            match Self::execute_trigger_skill(
                rng,
                &sim_fight,
                managers,
                mechanics,
                trigger_uid,
                trigger_skill_id,
            ) {
                Ok((trigger_162, buff_dels)) => {
                    result.push(trigger_162);
                    // BuffDels from AttrFromEntity self-deletes go to outer side_effects
                    for (del_target, del_buff_id) in buff_dels {
                        let uid = managers
                            .buff_mgr
                            .get(del_target)
                            .iter()
                            .find(|b| b.buff_id == del_buff_id)
                            .map(|b| b.uid)
                            .unwrap_or(0);
                        self.side_effects.push(ActEffect {
                            effect_type: Some(EffectType::BuffDel as i32),
                            target_id: Some(del_target),
                            buff: Some(BuffInfo {
                                buff_id: Some(del_buff_id),
                                uid: Some(uid),
                                from_uid: Some(del_target),
                                duration: Some(0),
                                count: Some(0),
                                ex_info: Some(0),
                                layer: Some(0),
                                r#type: Some(0),
                                act_common_params: Some(String::new()),
                                act_info: vec![],
                            }),
                            ..Default::default()
                        });
                    }
                }
                Err(e) => tracing::warn!(
                    "monitor trigger skill={} uid={}: {}",
                    trigger_skill_id,
                    trigger_uid,
                    e
                ),
            }
        }

        if self.call_depth > 1 {
            if let Some(step) = skill_act_effect.fight_step.as_mut() {
                step.act_effect.extend(self.side_effects.drain(..));
            }
            result.push(skill_act_effect);
        } else {
            result.push(skill_act_effect);
            result.extend(self.side_effects.drain(..));
        }

        let result_done = Instant::now();
        let setup_ms = setup_done.duration_since(exec_start).as_millis();
        let clone_ms = clones_done.duration_since(setup_done).as_millis();
        let body_ms = behaviors_done.duration_since(clones_done).as_millis();
        let tail_ms = result_done.duration_since(behaviors_done).as_millis();
        let total_ms = result_done.duration_since(exec_start).as_millis();
        if total_ms >= 20 {
            tracing::warn!(
                "[execute_skill][timing] skill={} setup={}ms clone={}ms body={}ms tail={}ms total={}ms effects={} out={}",
                skill_id,
                setup_ms,
                clone_ms,
                body_ms,
                tail_ms,
                total_ms,
                total_inner_effects,
                result.len()
            );
        }

        tracing::info!(
            "[execute_skill] skill={} total 162s: {}",
            skill_id,
            result.len()
        );

        Ok(result)
    }

    fn consume_attr_only_damage_buffs(
        &mut self,
        managers: &mut Managers,
        caster_uid: i64,
    ) -> Vec<ActEffect> {
        let cfg = config::configs::get();
        let mut out = Vec::new();

        let to_consume: Vec<(i64, i32, i64)> = managers
            .buff_mgr
            .get(caster_uid)
            .iter()
            .filter_map(|b| {
                let buff_cfg = cfg.skill_buff.get(b.buff_id)?;
                let has_attr_only = buff_cfg.features.split('|').any(|entry| {
                    let act_id = entry
                        .split('#')
                        .next()
                        .and_then(|v| v.trim().parse::<i32>().ok())
                        .unwrap_or(0);
                    cfg.buff_act
                        .get(act_id)
                        .map(|a| a.r#type == "AttrOnlyCalDamageAttack")
                        .unwrap_or(false)
                });
                if has_attr_only {
                    Some((b.uid, b.buff_id, b.from_uid))
                } else {
                    None
                }
            })
            .collect();

        for (buff_uid, buff_id, from_uid) in to_consume {
            managers.buff_mgr.remove_by_uid(caster_uid, buff_uid);
            let inner = FightStep {
                act_type: Some(fight_step::ActType::Effect.into()),
                from_id: Some(from_uid),
                to_id: Some(caster_uid),
                act_id: Some(buff_id),
                act_effect: vec![buff_del(caster_uid, buff_uid, buff_id, from_uid)],
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            };
            out.push(
                ActEffectBuilder::new(EffectType::FightStep as i32, 0)
                    .effect_num(0)
                    .fight_step(inner)
                    .build(),
            );
        }

        out
    }

    /// Execute a trigger skill and wrap as a 162 inline step.
    /// Used for CreateMaxHpAdditionalDamageAndRemove / RaspberryBigSkill procs.
    fn execute_trigger_skill(
        rng: &mut StdRng,
        fight: &Fight,
        managers: &mut Managers,
        mechanics: &mut Mechanics,
        caster_uid: i64,
        skill_id: i32,
    ) -> Result<(ActEffect, Vec<(i64, i32)>)> {
        let mut inner_executor = SkillExecutor::new();
        let phase = PhaseFilter::combat_with(TriggerState {
            active_use_skill: false,
            skill_id: 0,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            trigger_bullet: false,
            event_driven_only: false,
            be_attacked: false,
            hurt_not_restraint: false,
            hurt_restraint: false,
            teammate_injury_count: 0,
            teammate_injury_count_not_reset: 0,
            team_injury_count_round: false,
            deleted_buff_ids: managers.buff_mgr.step_deleted_buff_ids().to_vec(),
            bloodpool_max_attacker: Some(mechanics.bloodtithe.get_max(1)),
            bloodpool_value_attacker: Some(mechanics.bloodtithe.get_value(1)),
        });

        let mut results = inner_executor.execute_skill(
            rng, fight, managers, mechanics, caster_uid, -1, skill_id, &phase,
        )?;

        let buff_dels = inner_executor.pending_buff_dels.drain(..).collect();

        let mut act_effect = if results.is_empty() {
            ActEffectBuilder::new(EffectType::FightStep as i32, 0)
                .fight_step(FightStep {
                    act_type: Some(fight_step::ActType::Skill.into()),
                    from_id: Some(caster_uid),
                    to_id: Some(caster_uid),
                    act_id: Some(skill_id),
                    act_effect: vec![],
                    card_index: Some(0),
                    support_hero_id: Some(0),
                    fake_timeline: Some(false),
                    real_skill_type: Some(0),
                    real_skin_id: Some(0),
                })
                .build()
        } else {
            let has_step_damage = |step: &FightStep| {
                step.act_effect
                    .iter()
                    .any(|effect| effect.effect_type.is_some_and(is_damage_effect_type))
            };
            let selected_idx = results
                .iter()
                .position(|effect| {
                    effect
                        .fight_step
                        .as_ref()
                        .is_some_and(|step| step.act_id == Some(skill_id) && has_step_damage(step))
                })
                .or_else(|| {
                    results.iter().position(|effect| {
                        effect
                            .fight_step
                            .as_ref()
                            .is_some_and(|step| step.act_id == Some(skill_id))
                    })
                })
                .unwrap_or(0);
            results.remove(selected_idx)
        };

        // Some trigger paths return a self-wrapper SkillStep (act_id = X) that only
        // nests another SkillStep with the same act_id. Prefer the nested payload step.
        if let Some(step) = act_effect.fight_step.as_ref() {
            let parent_act_id = step.act_id;
            let parent_has_damage = step
                .act_effect
                .iter()
                .any(|effect| effect.effect_type.is_some_and(is_damage_effect_type));
            if !parent_has_damage {
                let nested_idx =
                    step.act_effect
                        .iter()
                        .position(|effect| {
                            effect.effect_type == Some(EffectType::FightStep as i32)
                                && effect.fight_step.as_ref().is_some_and(|nested| {
                                    nested.act_id == parent_act_id
                                        && nested.act_effect.iter().any(|e| {
                                            e.effect_type.is_some_and(is_damage_effect_type)
                                        })
                                })
                        })
                        .or_else(|| {
                            step.act_effect.iter().position(|effect| {
                                effect.effect_type == Some(EffectType::FightStep as i32)
                                    && effect
                                        .fight_step
                                        .as_ref()
                                        .is_some_and(|nested| nested.act_id == parent_act_id)
                            })
                        });
                if let Some(index) = nested_idx {
                    if let Some(nested) = step.act_effect.get(index) {
                        act_effect = nested.clone();
                    }
                }
            }
        }

        Ok((act_effect, buff_dels))
    }

    pub fn get_ally_uids(&self, fight: &Fight, caster_uid: i64) -> Vec<i64> {
        get_ally_uids(fight, caster_uid)
    }

    pub fn add_skill_rate_bonus(&mut self, caster_uid: i64, target_uid: i64, amount: i32) {
        if amount == 0 {
            return;
        }
        // target=self behaves like a global modifier for this skill instance.
        if target_uid == caster_uid {
            self.pending_global_rate_bonus = self.pending_global_rate_bonus.saturating_add(amount);
            return;
        }
        let entry = self
            .pending_target_rate_bonus
            .entry(target_uid)
            .or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    pub fn add_attr_bonus(&mut self, entity_uid: i64, attr_id: i32, amount: i32) {
        if amount == 0 {
            return;
        }
        let entry = self
            .pending_attr_bonus
            .entry((entity_uid, attr_id))
            .or_insert(0);
        *entry = entry.saturating_add(amount);
    }
}

fn behavior_execution_order(behaviors: &[super::cache::ResolvedBehavior]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..behaviors.len()).collect();

    for add_idx in 0..behaviors.len() {
        let BehaviorType::AddBuff { buff_id, .. } = behaviors[add_idx].behavior else {
            continue;
        };
        if buff_id <= 0 {
            continue;
        }

        let Some(dep_idx) = (0..add_idx).find(|idx| {
            let dep = &behaviors[*idx];
            dep.condition_target == behaviors[add_idx].behavior_target
                && condition_depends_on_buff(&dep.condition, buff_id)
        }) else {
            continue;
        };

        let Some(from_pos) = order.iter().position(|&v| v == add_idx) else {
            continue;
        };
        let Some(to_pos) = order.iter().position(|&v| v == dep_idx) else {
            continue;
        };
        if from_pos > to_pos {
            let moved = order.remove(from_pos);
            order.insert(to_pos, moved);
        }
    }

    order
}

fn condition_depends_on_buff(condition: &ConditionType, buff_id: i32) -> bool {
    condition::fold(condition, &mut |cond| match cond {
        ConditionType::HasBuffId { buff_ids }
        | ConditionType::NoBuffId { buff_ids }
        | ConditionType::PerBuffIdCount { buff_ids } => buff_ids.contains(&buff_id),
        _ => false,
    })
}

fn condition_has_combat_event(condition: &ConditionType) -> bool {
    condition::fold(condition, &mut |cond| {
        matches!(
            cond,
            ConditionType::ActiveUseSkill
                | ConditionType::ActiveUseSkillId { .. }
                | ConditionType::UseExSkill
                | ConditionType::TeammateUseExSkill
                | ConditionType::BeAttacked
                | ConditionType::HurtNotRestraint
                | ConditionType::HurtRestraint
                | ConditionType::TeammateInjuryCount { .. }
                | ConditionType::TeammateInjuryCountNotReset { .. }
                | ConditionType::TeamInjuryCountRound
                | ConditionType::BuffIdDel { .. }
                | ConditionType::CombatNone
        )
    })
}

fn condition_has_trigger_bullet_and_random(condition: &ConditionType) -> bool {
    match condition {
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => {
            let mut saw_trigger_bullet = false;
            let mut saw_random = false;
            for cond in conds {
                saw_trigger_bullet |= matches!(cond, ConditionType::TriggerBullet);
                saw_random |= matches!(cond, ConditionType::Random { .. });
            }
            saw_trigger_bullet && saw_random
        }
        _ => false,
    }
}

fn apply_no_act_seed_hint(
    fight: &Fight,
    caster_uid: i64,
    condition_uid: i64,
    condition: &ConditionType,
    raw: bool,
) -> bool {
    if condition_uid != caster_uid {
        return raw;
    }

    match condition {
        ConditionType::HasBuffId { buff_ids } => {
            if raw {
                return true;
            }
            raw || has_no_act_seed_buff(fight, caster_uid, buff_ids)
        }
        ConditionType::NoBuffId { buff_ids } => {
            if !raw {
                return false;
            }
            !has_no_act_seed_buff(fight, caster_uid, buff_ids)
        }
        _ => raw,
    }
}

fn has_no_act_seed_buff(fight: &Fight, caster_uid: i64, wanted_ids: &[i32]) -> bool {
    if wanted_ids.is_empty() {
        return false;
    }
    let cfg = config::configs::get();
    let Some(entity) = get_entity(fight, caster_uid) else {
        return false;
    };

    for passive_sid in &entity.passive_skill {
        if *passive_sid <= 0 {
            continue;
        }
        let effect_id = resolve_skill_effect_id(*passive_sid);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else {
            continue;
        };
        for row in rows {
            if !matches!(row.condition, ConditionType::NoActRound) {
                continue;
            }
            let BehaviorType::AddBuff { buff_id, .. } = row.behavior else {
                continue;
            };
            if wanted_ids.contains(&buff_id) {
                return true;
            }
            let type_id = cfg
                .skill_buff
                .iter()
                .find(|b| b.id == buff_id)
                .map(|b| b.type_id)
                .unwrap_or(0);
            if type_id > 0 && wanted_ids.contains(&type_id) {
                return true;
            }
        }
    }

    false
}

fn apply_preview_effects_to_sim_fight(fight: &mut Fight, effects: &[ActEffect]) {
    fn update_hp(fight: &mut Fight, uid: i64, delta: i32) {
        let apply = |entitys: &mut Vec<sonettobuf::FightEntityInfo>| {
            if let Some(entity) = entitys.iter_mut().find(|e| e.uid == Some(uid)) {
                let cur = entity.current_hp.unwrap_or(0);
                entity.current_hp = Some((cur + delta).max(0));
                return true;
            }
            false
        };
        if let Some(attacker) = fight.attacker.as_mut()
            && (apply(&mut attacker.entitys) || apply(&mut attacker.sub_entitys))
        {
            return;
        }
        if let Some(defender) = fight.defender.as_mut() {
            let _ = apply(&mut defender.entitys) || apply(&mut defender.sub_entitys);
        }
    }
    fn set_hp_zero(fight: &mut Fight, uid: i64) {
        let apply = |entitys: &mut Vec<sonettobuf::FightEntityInfo>| {
            if let Some(entity) = entitys.iter_mut().find(|e| e.uid == Some(uid)) {
                entity.current_hp = Some(0);
                return true;
            }
            false
        };
        if let Some(attacker) = fight.attacker.as_mut()
            && (apply(&mut attacker.entitys) || apply(&mut attacker.sub_entitys))
        {
            return;
        }
        if let Some(defender) = fight.defender.as_mut() {
            let _ = apply(&mut defender.entitys) || apply(&mut defender.sub_entitys);
        }
    }

    for effect in effects {
        let et = effect.effect_type.unwrap_or(0);
        if let Some(step) = &effect.fight_step {
            apply_preview_effects_to_sim_fight(fight, &step.act_effect);
            continue;
        }

        let target = effect.target_id.unwrap_or(0);
        if target == 0 {
            continue;
        }

        match et {
            x if is_damage_effect_type(x) => {
                update_hp(fight, target, -effect.effect_num.unwrap_or(0));
            }
            x if x == EffectType::Heal as i32
                || x == EffectType::HealCrit as i32
                || x == EffectType::Cure2 as i32 =>
            {
                update_hp(fight, target, effect.effect_num.unwrap_or(0));
            }
            x if x == EffectType::Kill as i32 => set_hp_zero(fight, target),
            _ => {}
        }
    }
}

fn is_damage_effect_type(effect_type: i32) -> bool {
    effect_type == EffectType::Damage as i32
        || effect_type == EffectType::Crit as i32
        || effect_type == EffectType::DamageExtra as i32
        || effect_type == EffectType::OriginDamage as i32
        || effect_type == EffectType::OriginCrit as i32
        || effect_type == EffectType::AdditionalDamage as i32
        || effect_type == EffectType::AdditionalDamageCrit as i32
        || effect_type == EffectType::FixedDamage as i32
        || effect_type == EffectType::DamageFromAbsorb as i32
        || effect_type == EffectType::DamageFromLostHp as i32
        || effect_type == EffectType::EnchantBurnDamage as i32
        || effect_type == EffectType::EnchantDepresseDamage as i32
        || effect_type == EffectType::DeadlyPoisonOriginDamage as i32
        || effect_type == EffectType::DeadlyPoisonOriginCrit as i32
}

fn collect_dead_effects_after_damage(fight: &Fight, effects: &[ActEffect]) -> Vec<ActEffect> {
    let mut states: HashMap<i64, (i32, i32)> = HashMap::new();
    let mut dead_targets: HashSet<i64> = effects
        .iter()
        .filter(|effect| effect.effect_type == Some(EffectType::Dead as i32))
        .filter_map(|effect| effect.target_id)
        .collect();
    let mut killed_in_order = Vec::new();

    for effect in effects {
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if !is_damage_effect_type(effect_type) {
            continue;
        }

        let Some(target_id) = effect.target_id else {
            continue;
        };
        if target_id == 0 || dead_targets.contains(&target_id) {
            continue;
        }

        let Some(entity) = get_entity(fight, target_id) else {
            continue;
        };
        let current_hp = entity.current_hp.unwrap_or(0);
        if current_hp <= 0 {
            dead_targets.insert(target_id);
            continue;
        }

        let (hp, shield) = states
            .entry(target_id)
            .or_insert((current_hp, entity.shield_value.unwrap_or(0)));
        let damage = effect.effect_num.unwrap_or(0).max(0);
        let shield_absorbed = damage.min(*shield);
        let hp_damage = damage.saturating_sub(shield_absorbed);

        *shield = shield.saturating_sub(shield_absorbed);
        *hp = hp.saturating_sub(hp_damage);

        if *hp <= 0 {
            dead_targets.insert(target_id);
            killed_in_order.push(target_id);
        }
    }

    killed_in_order
        .into_iter()
        .map(|target_id| {
            ActEffectBuilder::new(EffectType::Dead as i32, target_id)
                .effect_num(0)
                .build()
        })
        .collect()
}

fn apply_preview_effects_to_sim_buffs(buff_mgr: &mut BuffMgr, effects: &[ActEffect]) {
    for effect in effects {
        match effect.effect_type.unwrap_or(0) {
            x if x == EffectType::BuffAdd as i32 => {
                let target_uid = effect.target_id.unwrap_or(0);
                let buff_id = effect.effect_num.unwrap_or(0);
                let Some(buff) = effect.buff.as_ref() else {
                    continue;
                };
                if target_uid != 0 && buff_id != 0 {
                    observe_explicit_buff_uid_for_target(target_uid, buff.uid.unwrap_or(0));
                    buff_mgr.add_with_uid(
                        target_uid,
                        buff_id,
                        buff.from_uid.unwrap_or(0),
                        buff.count.unwrap_or(0),
                        buff.layer.unwrap_or(0),
                        buff.uid.unwrap_or(0),
                    );
                }
            }
            x if x == EffectType::BuffDel as i32 => {
                // Keep skill-slot condition checks anchored to pre-delete state.
                // Live lanes like 305122 evaluate downstream HasBuffId slots before
                // in-step BuffDel effects become visible.
                let _ = x;
            }
            x if x == EffectType::BuffUpdate as i32 => {
                let target_uid = effect.target_id.unwrap_or(0);
                let Some(buff) = effect.buff.as_ref() else {
                    continue;
                };
                let buff_id = buff.buff_id.unwrap_or(0);
                if target_uid != 0 && buff_id != 0 {
                    observe_explicit_buff_uid_for_target(target_uid, buff.uid.unwrap_or(0));
                    buff_mgr.add_with_uid(
                        target_uid,
                        buff_id,
                        buff.from_uid.unwrap_or(0),
                        buff.count.unwrap_or(0),
                        buff.layer.unwrap_or(0),
                        buff.uid.unwrap_or(0),
                    );
                }
            }
            x if x == EffectType::FightStep as i32 => {
                if let Some(step) = &effect.fight_step {
                    apply_preview_effects_to_sim_buffs(buff_mgr, &step.act_effect);
                }
            }
            _ => {}
        }
    }
}

fn fallback_damage_targets(
    fight: &Fight,
    caster_uid: i64,
    selected_target_uid: i64,
    logic_target: i32,
) -> Vec<i64> {
    // Live-like fallback targeting:
    // - default(single): selected target
    // - logicTarget=201: selected target + one more enemy
    // - logicTarget=202/301/302: all enemies
    if matches!(logic_target, 202 | 301 | 302) {
        return alive_enemies_by_position(fight, caster_uid);
    }

    if logic_target == 201 {
        let enemies = alive_enemies_by_position(fight, caster_uid);
        if enemies.is_empty() {
            return vec![selected_target_uid];
        }
        if enemies.contains(&selected_target_uid) {
            let idx = enemies
                .iter()
                .position(|uid| *uid == selected_target_uid)
                .unwrap_or(0);
            let mut out = vec![selected_target_uid];
            if enemies.len() > 1 {
                let extra = enemies[(idx + 1) % enemies.len()];
                if extra != selected_target_uid {
                    out.push(extra);
                }
            }
            return out;
        }
        let mut enemies = alive_enemies(fight, caster_uid);
        enemies.sort_by(|a, b| {
            let hp = |uid: i64| {
                get_entity(fight, uid)
                    .and_then(|e| e.current_hp)
                    .unwrap_or(0)
            };
            hp(*b).cmp(&hp(*a)).then_with(|| {
                let pos = |uid: i64| {
                    get_entity(fight, uid)
                        .and_then(|e| e.position)
                        .unwrap_or(99)
                };
                pos(*a).cmp(&pos(*b))
            })
        });
        return enemies.into_iter().take(2).collect();
    }

    let desired = 1usize;
    if desired <= 1 {
        return vec![selected_target_uid];
    }

    let enemies = alive_enemies_by_position(fight, caster_uid);

    let mut out = Vec::new();
    let selected_is_enemy = enemies.contains(&selected_target_uid)
        && get_entity(fight, selected_target_uid)
            .map(|e| e.current_hp.unwrap_or(0) > 0)
            .unwrap_or(false);
    if selected_is_enemy {
        out.push(selected_target_uid);
    }

    for uid in enemies {
        if out.len() >= desired {
            break;
        }
        if out.contains(&uid) {
            continue;
        }
        out.push(uid);
    }

    if out.is_empty() {
        vec![selected_target_uid]
    } else {
        out
    }
}

pub fn build_skill_act_effect(
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    phase: &PhaseFilter,
) -> Result<Vec<ActEffect>> {
    let mut executor = SkillExecutor::new();
    let seed = ctx.fight.cur_round.unwrap_or(0) as u64;
    let mut fallback_rng = StdRng::seed_from_u64(seed);
    let rng = match ctx.rng_ptr() {
        Some(mut rng) => {
            // SAFETY: FightContext only stores pointers captured from live mutable RNG refs.
            unsafe { rng.as_mut() }
        }
        None => &mut fallback_rng,
    };
    let fight = &*ctx.fight;
    let managers = &mut *ctx.managers;
    let mechanics = &mut *ctx.mechanics;
    executor.execute_skill(
        rng, fight, managers, mechanics, caster_uid, target_uid, skill_id, phase,
    )
}
