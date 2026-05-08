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
    fight::defender::Defender,
    fight_step::ActEffectBuilder,
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        ex_point_mgr::sync_from_fight,
        fight_data_mgr::Managers,
        round_mgr::seed_entry_max_hp_from_fight,
        wave_mgr::WaveMgr,
    },
    mechanics::{Mechanics, empathy::has_empathy_buff},
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
    utils::buff_del,
};

use super::{
    behavior::execute_behavior,
    cache::{SKILL_CACHE, resolve_skill_effect_id},
    condition::{self, ConditionEval},
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
    /// Current executing skill context `(skill_id, selected_target_uid)`.
    /// Nested direct-skill chains temporarily overwrite this and restore it
    /// on unwind so buff-action fanout can resolve the active hostile targets.
    current_skill_context: Option<(i32, i64)>,
    /// Per-entity temporary attribute bonuses for this skill execution.
    /// Key: (entity_uid, attr_id)
    pub pending_attr_bonus: HashMap<(i64, i32), i32>,
    /// Per-team preview of bloodtithe `(value, accumulator)` for this skill execution.
    /// This lets combat damage emit live-like positive 335 packets without mutating
    /// authoritative bloodtithe state before play_step_data replays the step.
    pub pending_bloodtithe_preview: HashMap<i32, (i32, i32)>,
    /// Deferred silent summons applied by the outer caller once a mutable `Fight` is available.
    pub(crate) pending_summons: Vec<PendingSummon>,
    /// Deferred monster-form transformations from `MonsterChange`
    /// behavior — drained by `apply_pending_monster_changes` once the
    /// outer caller can take a mutable `Fight`.
    pub(crate) pending_monster_changes: Vec<PendingMonsterChange>,
    override_damage_targets: Option<Vec<i64>>,
    call_depth: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingSummon {
    pub caster_uid: i64,
    pub monster_id: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingMonsterChange {
    pub target_uid: i64,
    pub new_monster_id: i32,
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

struct SkillContextGuard {
    current: *mut Option<(i32, i64)>,
    previous: Option<(i32, i64)>,
}

impl Drop for SkillContextGuard {
    fn drop(&mut self) {
        // SAFETY: `current` points to `self.current_skill_context` for the
        // lifetime of `execute_skill`.
        unsafe {
            *self.current = self.previous;
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

impl SkillExecutor {
    pub fn new() -> Self {
        Self {
            side_effects: Vec::new(),
            pending_monitor_triggers: Vec::new(),
            pending_buff_dels: Vec::new(),
            pending_target_rate_bonus: HashMap::new(),
            pending_global_rate_bonus: 0,
            current_skill_context: None,
            pending_attr_bonus: HashMap::new(),
            pending_bloodtithe_preview: HashMap::new(),
            pending_summons: Vec::new(),
            pending_monster_changes: Vec::new(),
            override_damage_targets: None,
            call_depth: 0,
        }
    }

    pub fn apply_pending_summons(
        &mut self,
        fight: &mut Fight,
        managers: &mut Managers,
    ) -> Result<()> {
        let pending = self.take_pending_summons();
        if pending.is_empty() {
            return Ok(());
        }

        Self::apply_summon_batch(fight, managers, &pending)
    }

    /// Drain queued `MonsterChange` requests and apply them to the
    /// fight via `mechanics::phase_change::transform_entity`. Called
    /// from sites that hold a mutable `Fight` after behavior dispatch.
    pub fn apply_pending_monster_changes(&mut self, fight: &mut Fight) -> Result<()> {
        let pending: Vec<PendingMonsterChange> = self.pending_monster_changes.drain(..).collect();
        for change in pending {
            crate::state::battle::mechanics::phase_change::transform_entity(
                fight,
                change.target_uid,
                change.new_monster_id,
            )?;
        }
        Ok(())
    }

    pub fn set_override_damage_targets(&mut self, targets: Vec<i64>) {
        self.override_damage_targets = Some(targets);
    }

    pub fn take_override_damage_targets(&mut self) -> Option<Vec<i64>> {
        self.override_damage_targets.take()
    }

    pub(crate) fn take_pending_summons(&mut self) -> Vec<PendingSummon> {
        self.pending_summons.drain(..).collect()
    }

    pub(crate) fn apply_summon_batch(
        fight: &mut Fight,
        managers: &mut Managers,
        summons: &[PendingSummon],
    ) -> Result<()> {
        if summons.is_empty() {
            return Ok(());
        }

        for summon in summons.iter().copied() {
            apply_pending_summon(fight, managers, summon)?;
        }

        sync_from_fight(fight, &mut managers.ex_point_mgr);
        seed_entry_max_hp_from_fight(fight);
        managers.entity_mgr.rebuild_cache(fight);
        managers.calculate_mgr.update_cache(fight);
        Ok(())
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
        // Catch-all timeline record: every emission funnels through here.
        // Higher-level call sites (CardCast, TriggerCombatPassive, etc.)
        // record their own entries too — appearing twice in the timeline
        // is expected. Skills that appear ONLY with ExecutorLowLevel
        // identify call sites that lack a dedicated phase tag yet.
        let exec_record_idx = mechanics.emission_timeline.record(
            crate::state::battle::emission_timeline::EmissionPhase::ExecutorLowLevel,
            caster_uid,
            skill_id,
            0,
            None,
            None,
        );
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
        let previous_skill_context = self.current_skill_context.replace((skill_id, target_uid));
        let _skill_context_guard = SkillContextGuard {
            current: &mut self.current_skill_context as *mut Option<(i32, i64)>,
            previous: previous_skill_context,
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
        let override_damage_targets = self.take_override_damage_targets();
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

            if !phase.check(&b.condition, b.behavior_target, caster_uid) {
                continue;
            }

            if !phase.allows_behavior(&b.behavior) {
                continue;
            }

            let cond_pass = if let PhaseFilter::Combat(event) = phase {
                let combat_raw = condition::eval_trigger_state_condition(
                    &b.condition,
                    condition::TriggerStateConditionContext {
                        event,
                        owner_uid: caster_uid,
                    },
                    condition::TriggerStateConditionOptions {
                        include_none: true,
                        include_combat_none: true,
                        ..Default::default()
                    },
                );
                if let Some(raw) = combat_raw {
                    if b.negated { !raw } else { raw }
                } else {
                    let condition_uid = if matches!(
                        b.condition,
                        ConditionType::TargetIsSelf | ConditionType::TargetIsTeamNoMe
                    ) {
                        if target_uid != 0 {
                            target_uid
                        } else {
                            caster_uid
                        }
                    } else if b.condition_target != 0 {
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
                    .with_trigger_state(has_trigger_state)
                    .with_condition_target(b.condition_target);
                    let condition_eval = if let PhaseFilter::Combat(event) = phase {
                        condition_eval.with_active_card_cast_uids(&event.active_card_cast_uids)
                    } else {
                        condition_eval
                    };
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
                    // HasBuffGroup / NoBuffGroup are always evaluated against
                    // the behavior's target (e.g. Tuesday's `In Mother's Arms`
                    // 30980121 condition3 `77208#7` checks the enemy receiving
                    // the buff, not Tuesday). condition_target=0 + logic_target=999
                    // alone resolves to caster_uid, which would always evaluate
                    // FALSE here. Override when target_uid is set.
                    let raw = if target_uid != 0 && target_uid != condition_uid {
                        match &b.condition {
                            ConditionType::HasBuffGroup { .. }
                            | ConditionType::NoBuffGroup { .. } => condition_eval
                                .for_target(target_uid)
                                .with_condition_target(0)
                                .check(&b.condition),
                            _ => raw,
                        }
                    } else {
                        raw
                    };
                    if b.negated { !raw } else { raw }
                }
            } else {
                let condition_uid = if matches!(
                    b.condition,
                    ConditionType::TargetIsSelf | ConditionType::TargetIsTeamNoMe
                ) {
                    if target_uid != 0 {
                        target_uid
                    } else {
                        caster_uid
                    }
                } else if b.condition_target != 0 {
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
                .with_trigger_state(has_trigger_state)
                .with_condition_target(b.condition_target);
                let condition_eval = if let PhaseFilter::Combat(event) = phase {
                    condition_eval.with_active_card_cast_uids(&event.active_card_cast_uids)
                } else {
                    condition_eval
                };
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
                // HasBuffGroup / NoBuffGroup are always evaluated against the
                // behavior's target — see combat-path block above.
                let raw = if target_uid != 0 && target_uid != condition_uid {
                    match &b.condition {
                        ConditionType::HasBuffGroup { .. } | ConditionType::NoBuffGroup { .. } => {
                            condition_eval
                                .for_target(target_uid)
                                .with_condition_target(0)
                                .check(&b.condition)
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

            if std::env::var_os("SONETTO_TRACE_BEHAVIOR_FIRE").is_some() {
                let trace_filter = std::env::var("SONETTO_TRACE_BEHAVIOR_FIRE")
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok())
                    .unwrap_or(0);
                if trace_filter == 0 || trace_filter == skill_id {
                    eprintln!(
                        "[behavior_fire] round={} skill={} caster={} target={} slot={} condition={:?}",
                        crate::state::battle::round_state::simulated_round(),
                        skill_id,
                        caster_uid,
                        target_uid,
                        slot_index,
                        b.condition,
                    );
                }
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
            let preview_summon_start = self.pending_summons.len();
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
            for summon in self.pending_summons[preview_summon_start..].iter().copied() {
                preview_pending_summon(&mut sim_fight, summon)?;
            }
            let behavior_effects = inject_empathy_storage_injuries(
                mechanics,
                &mut sim_buff_mgr,
                &mut managers.buff_mgr,
                &sim_fight,
                caster_uid,
                behavior_effects,
            );
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
            // The fallback `damageRate` damage path should still fire
            // even when behaviors emit "bonus" damage effects that
            // ride alongside the primary damage (e.g. Kakania's
            // Subconscious Empathy bonus, marked with
            // `config_effect = 60038`). Standard primary-damage
            // emissions from `lost_life::apply` carry
            // `config_effect = -1` (and the fallback path itself
            // emits the same value), so the bonus-only marker we
            // need to ignore is specifically the positive
            // bonus-config-effect family.
            let has_damage_effect = all_effects.iter().any(|e| {
                e.effect_type.map(is_damage_effect_type).unwrap_or(false)
                    && !is_bonus_damage_config_effect(e.config_effect.unwrap_or(0))
            });
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
                    override_damage_targets.as_deref(),
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
                let mut damage_effects = inject_empathy_storage_injuries(
                    mechanics,
                    &mut sim_buff_mgr,
                    &mut managers.buff_mgr,
                    &sim_fight,
                    caster_uid,
                    damage_effects,
                );
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

        // Sotheby Duality Potion (`30091120`) holder-consume for the
        // basic (`30090111`) and upgraded-basic (`30090112`) lanes.
        // Detonate (`300901321`) keeps its existing inline consume
        // in `damage.rs::execute_sotheby_detonate2`. Per
        // `_30091120_design.md`: 1 fanout wrapper containing all
        // stack sequences flattened + 1 sibling delete wrapper. The
        // shared helper `build_sotheby_holder_consume_steps` produces
        // both and emits Cure as `Add (uid X) + Update (uid X, layer
        // climb)` so runtime BuffMgr ends with one Cure instance per
        // ally at layer=stack_count, matching LIVE r5. Eligibility
        // intentionally narrow — only basic + upgraded-basic skill
        // ids — to avoid the wide-eligibility regression documented
        // in `_30091120_findings.md` attempts 1+2.
        if matches!(skill_id, 30090111 | 30090112)
            && all_effects
                .iter()
                .any(|e| e.effect_type.map(is_damage_effect_type).unwrap_or(false))
            && let Some(holder) = managers
                .buff_mgr
                .find_instance_by_buff_id(
                    caster_uid,
                    crate::state::battle::skill::behavior::damage::DUALITY_POTION_BUFF_ID,
                )
                .cloned()
        {
            let mut hostile_targets: Vec<i64> = Vec::new();
            for effect in &all_effects {
                let et = effect.effect_type.unwrap_or(0);
                if !is_damage_effect_type(et) {
                    continue;
                }
                if let Some(ti) = effect.target_id
                    && ti.signum() != caster_uid.signum()
                    && !hostile_targets.contains(&ti)
                {
                    hostile_targets.push(ti);
                }
            }
            if !hostile_targets.is_empty() {
                let stack_count = holder.layer.max(1);
                let consume_steps =
                    crate::state::battle::skill::behavior::damage::build_sotheby_holder_consume_steps(
                        &sim_fight,
                        caster_uid,
                        &hostile_targets,
                        crate::state::battle::skill::behavior::damage::CURE_TYPE_ID,
                        &holder,
                        stack_count,
                        false,
                    );
                all_effects.extend(consume_steps);
                managers.buff_mgr.remove_by_uid(caster_uid, holder.uid);
            }
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
                phase,
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

        if !result.is_empty() {
            mechanics.emission_timeline.mark_produced(exec_record_idx);
        }
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
        parent_phase: &PhaseFilter,
    ) -> Result<(ActEffect, Vec<(i64, i32)>)> {
        let mut inner_executor = SkillExecutor::new();
        let active_card_cast_uids = if let PhaseFilter::Combat(event) = parent_phase {
            event.active_card_cast_uids.clone()
        } else {
            HashSet::new()
        };
        let phase = PhaseFilter::combat_with(TriggerState {
            active_use_skill: false,
            skill_id: 0,
            action_order_index: 0,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            trigger_bullet: false,
            event_driven_only: false,
            be_attacked: false,
            hurt_magic: false,
            lost_ex_point: false,
            hurt_not_restraint: false,
            hurt_restraint: false,
            teammate_injury_count: 0,
            teammate_injury_count_not_reset: 0,
            team_injury_count_round: false,
            deleted_buff_ids: managers.buff_mgr.step_deleted_buff_ids().to_vec(),
            active_card_cast_uids,
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

    pub fn current_skill_context(&self) -> Option<(i32, i64)> {
        self.current_skill_context
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
    // Keep this list in sync with `classification::is_combat_event_condition`
    // (skill/classification.rs:86). They MUST agree on which conditions count
    // as "combat events" — otherwise `has_combat_reactive_condition`-gated
    // sweep entry can let a skill in whose behaviors the executor then skips
    // (because their condition isn't recognized as combat-event), producing
    // missing emissions. Battle1 r1 step[30] (Pickles' `30630141` end-of-round
    // "Clarified Topic" with `NoActRound`) regressed when this list omitted
    // `NoActRound`/`TriggerBullet` — fixed by adding them here.
    condition::is_combat_event_condition(
        condition,
        condition::CombatEventConditionOptions::default(),
    )
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
                || x == EffectType::Cure2 as i32
                || x == EffectType::InjuryBankHeal as i32 =>
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

/// Returns true for damage effects whose `configEffect` marks them as
/// a "bonus" emission that runs alongside the primary `damageRate`
/// damage (Kakania's Subconscious Empathy bonus uses 60038, Solace
/// self-loss uses 60039, EX consume-and-bonus uses 60040). The
/// fallback damage path uses these markers to know it should still
/// emit the primary damage rate even when one of these bonus
/// emissions has already fired.
fn is_bonus_damage_config_effect(config_effect: i32) -> bool {
    matches!(config_effect, 60038 | 60039 | 60040)
}

fn inject_empathy_storage_injuries(
    mechanics: &mut Mechanics,
    preview_buff_mgr: &mut BuffMgr,
    live_buff_mgr: &mut BuffMgr,
    fight: &Fight,
    source_uid: i64,
    effects: Vec<ActEffect>,
) -> Vec<ActEffect> {
    let effects = crate::state::battle::heroes::kakania::inject_damage_redirect(
        &mut mechanics.empathy,
        preview_buff_mgr,
        live_buff_mgr,
        fight,
        source_uid,
        effects,
    );

    let mut effect_targets = Vec::new();
    for effect in &effects {
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if effect_type == EffectType::DamageFromAbsorb as i32
            || !is_damage_effect_type(effect_type)
            || target_uid == source_uid
            || effect_targets.contains(&target_uid)
            || !has_empathy_buff(preview_buff_mgr, target_uid)
        {
            continue;
        }
        effect_targets.push(target_uid);
    }

    let mut out = effects;
    for target_uid in effect_targets {
        let Some(target) = get_entity(fight, target_uid) else {
            continue;
        };
        let target_max_hp = target.attr.as_ref().and_then(|attr| attr.hp).unwrap_or(0);
        if target_max_hp <= 0 {
            continue;
        }

        out = crate::state::battle::heroes::kakania::inject_storage_injury_for_damage_emissions(
            &mut mechanics.empathy,
            preview_buff_mgr,
            fight,
            source_uid,
            target_uid,
            target_max_hp,
            out,
        );
        mechanics.empathy.sync_buff_state(
            live_buff_mgr,
            target_uid,
            mechanics.empathy.current(target_uid),
            target_max_hp,
        );
    }

    crate::state::battle::heroes::kakania::inject_insight_iii_bounces_for_heal_emissions(
        &mechanics.empathy,
        preview_buff_mgr,
        fight,
        out,
    )
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
                    let _ = buff_mgr.set_instance_act_common_params(
                        target_uid,
                        buff.uid.unwrap_or(0),
                        buff.act_common_params.as_deref().unwrap_or_default(),
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
                    let _ = buff_mgr.set_instance_act_common_params(
                        target_uid,
                        buff.uid.unwrap_or(0),
                        buff.act_common_params.as_deref().unwrap_or_default(),
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
    override_damage_targets: Option<&[i64]>,
) -> Vec<i64> {
    if let Some(override_damage_targets) = override_damage_targets
        && !override_damage_targets.is_empty()
    {
        return override_damage_targets.to_vec();
    }

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
    let effects = executor.execute_skill(
        rng,
        &*ctx.fight,
        &mut *ctx.managers,
        &mut *ctx.mechanics,
        caster_uid,
        target_uid,
        skill_id,
        phase,
    )?;
    executor.apply_pending_summons(ctx.fight, ctx.managers)?;
    Ok(effects)
}

fn apply_pending_summon(
    fight: &mut Fight,
    managers: &mut Managers,
    summon: PendingSummon,
) -> Result<()> {
    let new_uid = spawn_summoned_entity(fight, summon)?;
    managers.buff_mgr.clear(new_uid);

    tracing::info!(
        "applied summon caster={} monster={} uid={}",
        summon.caster_uid,
        summon.monster_id,
        new_uid
    );
    Ok(())
}

fn next_summon_uid(fight: &Fight) -> i64 {
    let min_existing_uid = fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
        .filter_map(|entity| entity.uid)
        .min()
        .unwrap_or(0);
    let min_wave_reserved_uid = -(2 * WaveMgr::max_wave_for_fight(fight) as i64);
    min_existing_uid.min(min_wave_reserved_uid) - 1
}

fn next_summon_position(fight: &Fight) -> i32 {
    fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.sub_entitys.iter())
        .filter_map(|entity| entity.position)
        .filter(|position| *position < 0)
        .min()
        .map(|position| position - 1)
        .unwrap_or(-1)
}

fn preview_pending_summon(fight: &mut Fight, summon: PendingSummon) -> Result<()> {
    let new_uid = spawn_summoned_entity(fight, summon)?;
    tracing::debug!(
        "previewed summon caster={} monster={} uid={}",
        summon.caster_uid,
        summon.monster_id,
        new_uid
    );
    Ok(())
}

fn spawn_summoned_entity(fight: &mut Fight, summon: PendingSummon) -> Result<i64> {
    let uid = next_summon_uid(fight);
    let position = next_summon_position(fight);
    let entity = Defender::build_enemy_with_uid(summon.monster_id, uid, position, 2)?;
    let new_uid = entity.uid.unwrap_or(uid);

    let defender = fight
        .defender
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Fight missing defender team"))?;
    defender.sub_entitys.push(entity);
    Ok(new_uid)
}
