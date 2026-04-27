use sonettobuf::{ActEffect, Fight, FightStep, effect_type_enum::EffectType};
use std::collections::HashSet;

use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::{
    ConditionType,
    context::FightContext,
    passives::collector::CollectedPassives,
    passives::steps::skill::execute_skill,
    round::step_shape::build_effect_step,
    skill::cache::{SKILL_CACHE, resolve_skill_effect_id},
    skill::classification::{
        CombatPassiveScanMode, has_combat_reactive_condition, has_injury_reactive_condition,
    },
    skill::condition::buff::deleted_matches,
    skill::condition::parser::parse_condition,
    skill::euphoria::resolve_with_euphoria,
    skill::{PhaseFilter, TriggerState},
    trigger::passes::{
        BloodPoolSyncPass, BloodValueUseSkillPass, CardEnergySyncPass, CombatPassivesPass,
        ExPointSyncPass, HpSyncPass, TriggerPass,
    },
};

/// Context passed to each trigger check describing what just happened.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// Entity that played the card / cast the skill (attacker side).
    pub caster_uid: i64,
    /// Skill id used by the caster for this step.
    pub skill_id: i32,
    /// 1-based action slot for the current acting unit in the round, when known.
    pub action_order_index: i32,
    /// Primary target from the originating root step (card/skill target).
    pub primary_target_uid: i64,
    /// Whether this step used the caster's ex skill.
    pub used_ex_skill: bool,
    /// True when the root operation card had caster uid 0 (wrapper/precast card).
    pub from_wrapper_card: bool,
    /// Nested skill uses found in inline fightStep payloads.
    pub nested_skill_uses: Vec<(i64, i32, bool, i64)>, // (uid, skill_id, used_ex, to_id)
    /// Entities that took damage this step.
    pub damaged_uids: Vec<i64>,
    /// Subset of `damaged_uids` that took damage from cross-side (enemy)
    /// attacks rather than same-side self-cost or teammate-inflicted hits.
    /// Used to gate BeAttacked reactive passives so e.g. raspberry bloodpool
    /// cost on Nautika does not fire BeAttacked-triggered behaviors like
    /// 31200222 c1, which LIVE only fires for actual enemy attacks.
    pub cross_side_damaged_uids: Vec<i64>,
    /// Entities that dealt damage this step (usually just the caster).
    pub dealer_uids: Vec<i64>,
    /// Buff ids/type-ids deleted during this step.
    pub deleted_buff_ids: Vec<i32>,
    /// Buff instance uids added during this step.
    pub added_buff_uids: Vec<i64>,
    /// Whether this step triggered bullet mechanics.
    pub trigger_bullet: bool,
    /// Positive bloodpool gain deltas emitted during this step, keyed by team_type.
    pub bloodpool_gain_by_team: Vec<(i32, i32)>,
    /// Positive bloodpool gain deltas emitted during this step, keyed by (skill_id, team_type).
    /// `skill_id == 0` means the gain was not nested under a concrete skill wrapper.
    pub bloodpool_gain_by_skill_team: Vec<(i32, i32, i32)>,
    /// Positive bloodpool gain packet count emitted during this step, keyed by team_type.
    pub bloodpool_gain_packets_by_team: Vec<(i32, i32)>,
}

impl TriggerEvent {
    pub fn took_damage(&self, uid: i64) -> bool {
        self.damaged_uids.contains(&uid)
    }
    /// Returns true only when `uid` was hit by an enemy in this event.
    /// Excludes same-side self-cost damage (raspberry/bloodpool) so that
    /// passives gated on BeAttacked do not fire on internal HP drops.
    pub fn was_attacked_by_enemy(&self, uid: i64) -> bool {
        self.cross_side_damaged_uids.contains(&uid)
    }
    pub fn dealt_damage(&self, uid: i64) -> bool {
        self.dealer_uids.contains(&uid)
    }
    /// Returns true when any teammate of `uid` (same signum) dealt damage.
    /// Used by the damage-reactive extra-skill fallback in `skill_should_fire`
    /// so channel-extra skills on teammates wake up on the dealer's damage.
    pub fn same_team_dealt_damage(&self, uid: i64) -> bool {
        uid != 0
            && self
                .dealer_uids
                .iter()
                .any(|&dealer| dealer != 0 && dealer.signum() == uid.signum())
    }
    pub fn used_card(&self, uid: i64) -> bool {
        self.caster_uid == uid
    }
    pub fn skill_used_by(&self, uid: i64) -> Option<(i32, bool, i64)> {
        if self.caster_uid == uid {
            return Some((self.skill_id, self.used_ex_skill, self.primary_target_uid));
        }
        self.nested_skill_uses
            .iter()
            .rev()
            .find(|(u, _, _, _)| *u == uid)
            .map(|(_, sid, ex, to)| (*sid, *ex, *to))
    }
    /// Resolve the card/skill-use event that should gate `uid`'s passive.
    /// For teammates reacting to an ally's cast, surface the acting ally's
    /// skill use so `ActiveUseSkill`-family conditions evaluate against the
    /// shared same-side event.
    pub fn skill_used_for_passive_owner(&self, uid: i64) -> Option<(i64, i32, bool, i64)> {
        if let Some((sid, ex, to)) = self.skill_used_by(uid) {
            return Some((uid, sid, ex, to));
        }
        if uid != 0
            && self.caster_uid != 0
            && self.caster_uid != uid
            && self.caster_uid.signum() == uid.signum()
        {
            return Some((
                self.caster_uid,
                self.skill_id,
                self.used_ex_skill,
                self.primary_target_uid,
            ));
        }
        self.nested_skill_uses
            .iter()
            .rev()
            .find(|(u, _, _, _)| *u != uid && *u != 0 && u.signum() == uid.signum())
            .copied()
    }
    pub fn teammate_used_ex_skill(&self, uid: i64) -> bool {
        if self.from_wrapper_card {
            return false;
        }
        self.nested_skill_uses
            .iter()
            .any(|(u, _sid, ex, _to)| *u != uid && u.signum() == uid.signum() && *ex)
            || (self.caster_uid != uid
                && self.caster_uid.signum() == uid.signum()
                && self.used_ex_skill)
    }

    pub fn triggered_bullet_for(&self, uid: i64) -> bool {
        // TriggerBullet is team-scoped in current data: every TriggerBullet row
        // in skill_effect.json uses conditionTarget=103, so same-side bullet
        // reactions must wake teammate passives and channel extra skills too.
        self.trigger_bullet && self.caster_uid != 0 && self.caster_uid.signum() == uid.signum()
    }

    pub fn bloodpool_gain(&self, team_type: i32) -> i32 {
        self.bloodpool_gain_by_team
            .iter()
            .find(|(team, _)| *team == team_type)
            .map(|(_, gain)| *gain)
            .unwrap_or(0)
    }
}

/// Extract a TriggerEvent from the effects produced by a card skill step.
pub fn event_from_step(
    fight: &sonettobuf::Fight,
    caster_uid: i64,
    primary_target_uid: i64,
    skill_id: i32,
    effects: &[ActEffect],
) -> TriggerEvent {
    let mut damaged_uids = Vec::new();
    let mut cross_side_damaged_uids = Vec::new();
    let mut dealer_uids = Vec::new();
    let mut deleted_buff_ids = Vec::new();
    let mut added_buff_uids = Vec::new();
    let mut _added_buff_ids = Vec::new();

    collect_damage_uids(
        effects,
        caster_uid,
        &mut damaged_uids,
        &mut cross_side_damaged_uids,
        &mut dealer_uids,
    );
    collect_deleted_buff_ids(effects, &mut deleted_buff_ids);
    collect_added_buffs(effects, &mut added_buff_uids, &mut _added_buff_ids);
    let mut nested_skill_ids = Vec::new();
    collect_nested_skill_ids(effects, &mut nested_skill_ids);
    let mut nested_skill_uses = Vec::new();
    collect_nested_skill_uses(fight, effects, &mut nested_skill_uses);
    let mut bloodpool_gain_by_team = Vec::new();
    let mut bloodpool_gain_by_skill_team = Vec::new();
    let mut bloodpool_gain_packets_by_team = Vec::new();
    collect_bloodpool_gains(
        effects,
        &mut bloodpool_gain_by_team,
        &mut bloodpool_gain_by_skill_team,
        &mut bloodpool_gain_packets_by_team,
    );

    // Wrapper cards may use fromId=0; in that case derive actor/skill from nested steps.
    let mut effective_caster_uid = caster_uid;
    let mut effective_skill_id = skill_id;
    let mut used_ex_skill = false;
    let from_wrapper_card = caster_uid == 0;

    if caster_uid == 0 {
        let wrapper_ex_id = skill_id - 20;
        if wrapper_ex_id > 0
            && let Some((uid, sid, _, _)) = nested_skill_uses
                .iter()
                .find(|(_, sid, _, _)| *sid == wrapper_ex_id)
                .copied()
        {
            effective_caster_uid = uid;
            effective_skill_id = sid;
            // Direct-use wrapper cards (caster=0) should not be treated as
            // "UseExSkill" for teammate reactive passives in trigger pass.
            used_ex_skill = false;
        } else if let Some((uid, sid, ex, _)) =
            nested_skill_uses.iter().find(|(_, _, ex, _)| *ex).copied()
        {
            effective_caster_uid = uid;
            effective_skill_id = sid;
            used_ex_skill = ex;
        } else if let Some((uid, sid, ex, _)) = nested_skill_uses.first().copied() {
            effective_caster_uid = uid;
            effective_skill_id = sid;
            used_ex_skill = ex;
        }
    }

    if !used_ex_skill {
        let ex_skill = crate::state::battle::skill::get_entity(fight, effective_caster_uid)
            .and_then(|e| e.ex_skill);
        if let Some(ex_id) = ex_skill {
            if nested_skill_ids.contains(&ex_id) {
                effective_skill_id = ex_id;
            }
            used_ex_skill = ex_id == effective_skill_id;
        }
    }

    let trigger_bullet = has_bullet_trigger(effective_skill_id, &nested_skill_uses);

    TriggerEvent {
        caster_uid: effective_caster_uid,
        skill_id: effective_skill_id,
        action_order_index: 0,
        primary_target_uid,
        used_ex_skill,
        from_wrapper_card,
        nested_skill_uses,
        damaged_uids,
        cross_side_damaged_uids,
        dealer_uids,
        deleted_buff_ids,
        added_buff_uids,
        trigger_bullet,
        bloodpool_gain_by_team,
        bloodpool_gain_by_skill_team,
        bloodpool_gain_packets_by_team,
    }
}

fn push_team_metric(out: &mut Vec<(i32, i32)>, team_type: i32, value: i32) {
    if let Some((_, existing)) = out.iter_mut().find(|(team, _)| *team == team_type) {
        *existing += value;
    } else {
        out.push((team_type, value));
    }
}

fn push_skill_team_metric(
    out: &mut Vec<(i32, i32, i32)>,
    skill_id: i32,
    team_type: i32,
    value: i32,
) {
    if let Some((_, _, existing)) = out
        .iter_mut()
        .find(|(sid, team, _)| *sid == skill_id && *team == team_type)
    {
        *existing += value;
    } else {
        out.push((skill_id, team_type, value));
    }
}

fn collect_bloodpool_gains(
    effects: &[ActEffect],
    delta_out: &mut Vec<(i32, i32)>,
    delta_by_skill_out: &mut Vec<(i32, i32, i32)>,
    packet_out: &mut Vec<(i32, i32)>,
) {
    collect_bloodpool_gains_inner(effects, delta_out, delta_by_skill_out, packet_out, None);
}

fn collect_bloodpool_gains_inner(
    effects: &[ActEffect],
    delta_out: &mut Vec<(i32, i32)>,
    delta_by_skill_out: &mut Vec<(i32, i32, i32)>,
    packet_out: &mut Vec<(i32, i32)>,
    current_skill_id: Option<i32>,
) {
    for effect in effects {
        if effect.effect_type == Some(EffectType::Bloodpoolvaluechange as i32) {
            let team_type = effect.effect_num.unwrap_or(0);
            let delta = effect.effect_num1.unwrap_or(0);
            let from_magic_circle = current_skill_id.is_some_and(
                crate::state::battle::mechanics::magic_circle::is_magic_circle_self_skill,
            );
            if team_type > 0 && delta > 0 && !from_magic_circle {
                push_team_metric(delta_out, team_type, delta);
                push_skill_team_metric(
                    delta_by_skill_out,
                    current_skill_id.unwrap_or(0),
                    team_type,
                    delta,
                );
                if current_skill_id != Some(308801322) {
                    push_team_metric(packet_out, team_type, 1);
                }
            }
        }
        if let Some(step) = &effect.fight_step {
            let nested_skill_id = (step.act_type
                == Some(sonettobuf::fight_step::ActType::Skill as i32))
            .then(|| step.act_id.unwrap_or(0))
            .filter(|id| *id > 0)
            .or(current_skill_id);
            collect_bloodpool_gains_inner(
                &step.act_effect,
                delta_out,
                delta_by_skill_out,
                packet_out,
                nested_skill_id,
            );
        }
    }
}

fn collect_damage_uids(
    effects: &[ActEffect],
    caster_uid: i64,
    damaged: &mut Vec<i64>,
    cross_side_damaged: &mut Vec<i64>,
    dealers: &mut Vec<i64>,
) {
    for effect in effects {
        let et = effect.effect_type.unwrap_or(0);
        let is_damage = et == EffectType::Damage as i32
            || et == EffectType::Crit as i32
            || et == EffectType::Additionaldamage as i32
            || et == EffectType::Additionaldamagecrit as i32
            || et == EffectType::Fixeddamage as i32
            || et == EffectType::Origindamage as i32
            || et == EffectType::Origincrit as i32;

        if is_damage && let Some(target) = effect.target_id {
            if !damaged.contains(&target) {
                damaged.push(target);
            }
            if !dealers.contains(&caster_uid) {
                dealers.push(caster_uid);
            }
            // Cross-side damage (enemy-source) feeds the BeAttacked gate.
            // Self-inflicted damage (same-side, e.g. raspberry bloodpool
            // cost) is still tracked in `damaged` for injury counts, but
            // excluded from the BeAttacked set so reactive passives don't
            // fire on internal HP drops.
            let is_cross_side =
                caster_uid != 0 && target != 0 && caster_uid.signum() != target.signum();
            if is_cross_side && !cross_side_damaged.contains(&target) {
                cross_side_damaged.push(target);
            }
        }

        // Recurse into any nested fightStep payload and keep dealer attribution
        // aligned with the nested step's own caster when present.
        if let Some(step) = &effect.fight_step {
            let nested_caster = step.from_id.unwrap_or(caster_uid);
            collect_damage_uids(
                &step.act_effect,
                nested_caster,
                damaged,
                cross_side_damaged,
                dealers,
            );
        }
    }
}

/// Fire all combat triggers after a card skill executes.
/// Returns top-level FightSteps to append to the round step list.
/// Shape contract:
/// - one top-level EFFECT step per triggering entity
/// - each entity step packs that entity's effects in arrival order
/// - packed effects may be mixed (162 wrappers and flat effects)
pub fn fire_combat_triggers(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    event: &TriggerEvent,
) -> Vec<FightStep> {
    let passes: [&dyn TriggerPass; 6] = [
        &CombatPassivesPass,
        &BloodValueUseSkillPass,
        &ExPointSyncPass,
        &HpSyncPass,
        &BloodPoolSyncPass,
        &CardEnergySyncPass,
    ];

    passes
        .iter()
        .flat_map(|pass| pass.run(ctx, event, collected))
        .collect()
}

pub(crate) fn run_combat_passives_pass(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    event: &TriggerEvent,
) -> Vec<FightStep> {
    let battle_rule_skills = collect_battle_rule_skills(ctx.fight);
    let mut steps = Vec::new();

    let all_uids: Vec<i64> = collected
        .defender_uids()
        .into_iter()
        .chain(collected.attacker_uids())
        .collect();

    for uid in all_uids {
        let base_skill_ids = collected.merged_for(uid);
        let mut buff_granted_ids: Vec<i32> = Vec::new();
        extend_with_buff_granted_passives(ctx, uid, &mut buff_granted_ids);
        // Track skills that only reach this pass via a buff-grant chain
        // (e.g. Sentinel's 31260181 granted via 31260151→SubBuff 31260201→
        // AddPassiveSkills). LIVE fires these when the owner acts; restrict
        // the bypass to passives whose conditions consult a HasBuffId check
        // so unconditional None-only passives don't leak into combat triggers.
        let buff_granted_has_buff_set: std::collections::HashSet<i32> = buff_granted_ids
            .iter()
            .copied()
            .filter(|sid| !base_skill_ids.contains(sid) && skill_has_has_buff_id_condition(*sid))
            .collect();
        let mut skill_ids = base_skill_ids;
        for sid in &buff_granted_ids {
            if !skill_ids.contains(sid) {
                skill_ids.push(*sid);
            }
        }
        for sid in &battle_rule_skills {
            if !skill_ids.contains(sid) {
                skill_ids.push(*sid);
            }
        }
        skill_ids.sort_unstable();
        let teammate_injury_hits = team_injury_hits_for_uid(uid, event);
        let teammate_injury_not_reset = ctx.managers.buff_mgr.teammate_injury_not_reset(uid);
        let mut entity_step_effects: Vec<ActEffect> = Vec::new();

        for skill_id in skill_ids {
            let trace_target = matches!(skill_id, 30090146 | 31040141 | 30980142);
            if is_enter_fight_only_passive(skill_id) {
                if trace_target {
                    eprintln!("trace passive skill={} uid={} skipped=enter_fight_only", skill_id, uid);
                }
                continue;
            }
            if skill_id == 31260181
                && ctx.managers.buff_mgr.has(uid, 31260131)
                && event
                    .skill_used_by(uid)
                    .map(|(sid, _, _)| sid == 31260121)
                    .unwrap_or(false)
            {
                continue;
            }
            let is_buff_granted_has_buff_skill = buff_granted_has_buff_set.contains(&skill_id);
            if !is_buff_granted_has_buff_skill
                && !has_combat_reactive_condition(skill_id, CombatPassiveScanMode::TriggerPass)
            {
                if trace_target {
                    eprintln!(
                        "trace passive skill={} uid={} skipped=no_combat_reactive_condition",
                        skill_id, uid
                    );
                }
                continue;
            }
            let should_fire = if is_buff_granted_has_buff_skill {
                event.used_card(uid)
            } else {
                skill_should_fire(
                    uid,
                    skill_id,
                    event,
                    teammate_injury_hits,
                    teammate_injury_not_reset,
                )
            };
            if trace_target {
                eprintln!(
                    "trace passive skill={} uid={} should_fire={} actor={:?}",
                    skill_id,
                    uid,
                    should_fire,
                    event.skill_used_for_passive_owner(uid)
                );
            }
            if !should_fire {
                continue;
            }
            // ActiveUseSkill/CombatNone passives for the main caster are already
            // fired inline inside the card step — skip them here to avoid duplicates.
            if event.used_card(uid) && is_active_use_skill_passive(skill_id) {
                if trace_target {
                    eprintln!("trace passive skill={} uid={} skipped=already_inline", skill_id, uid);
                }
                continue;
            }
            // Build a TriggerState that reflects what actually happened for this entity.
            let (active_use_skill, _trigger_actor_uid, uid_skill_id, uid_used_ex, uid_skill_target) =
                event
                    .skill_used_for_passive_owner(uid)
                    .map(|(actor_uid, sid, ex, to)| (true, actor_uid, sid, ex, to))
                    .unwrap_or((false, 0, 0, false, 0));
            let trigger_state_base = TriggerState {
                active_use_skill,
                skill_id: uid_skill_id,
                action_order_index: event.action_order_index,
                used_ex_skill: uid_used_ex,
                teammate_use_ex_skill: event.teammate_used_ex_skill(uid),
                trigger_bullet: event.triggered_bullet_for(uid),
                event_driven_only: should_use_strict_event_only(ctx.fight, skill_id),
                be_attacked: event.was_attacked_by_enemy(uid),
                hurt_not_restraint: event.dealt_damage(uid),
                hurt_restraint: event.dealt_damage(uid),
                teammate_injury_count: teammate_injury_hits,
                teammate_injury_count_not_reset: teammate_injury_not_reset,
                team_injury_count_round: teammate_injury_hits > 0,
                deleted_buff_ids: event.deleted_buff_ids.clone(),
                bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
                bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
            };
            let trigger_target_uid =
                if uid_skill_target != 0 && uid_skill_target.signum() != uid.signum() {
                    uid_skill_target
                } else if event.primary_target_uid != 0
                    && event.primary_target_uid.signum() != uid.signum()
                {
                    event.primary_target_uid
                } else if event.caster_uid != 0 && event.caster_uid.signum() != uid.signum() {
                    event.caster_uid
                } else {
                    uid
                };
            // LIVE fires injury-reactive passives (TeammateInjuryCount /
            // TeammateInjuryCountNotReset) once per distinct injured ally rather
            // than once per batched root event. Replay the skill N times with
            // per-event injury state so cond3-style refresh behaviors reach
            // parity. The first replay retains batch values so AoE-driven
            // conditions (e.g. TeammateInjuryCount(3)) still trigger when the
            // batch count satisfies them.
            let replay_count = if has_injury_reactive_condition(skill_id) {
                distinct_teammate_injuries_excluding_self(uid, event).max(1)
            } else {
                1
            };
            for replay_idx in 0..replay_count {
                let trigger_state = if replay_idx == 0 {
                    trigger_state_base.clone()
                } else {
                    let per_hits = 1_i32;
                    let per_not_reset = teammate_injury_not_reset + replay_idx + 1;
                    if !skill_should_fire(uid, skill_id, event, per_hits, per_not_reset) {
                        continue;
                    }
                    let mut ts = trigger_state_base.clone();
                    ts.teammate_injury_count = per_hits;
                    ts.teammate_injury_count_not_reset = per_not_reset;
                    ts.team_injury_count_round = true;
                    ts
                };
                match execute_skill(
                    ctx,
                    uid,
                    trigger_target_uid,
                    skill_id,
                    &PhaseFilter::combat_with(trigger_state),
                ) {
                    Ok(mut skill_effects) if !skill_effects.is_empty() => {
                        if trace_target {
                            eprintln!(
                                "trace passive skill={} uid={} emitted={}",
                                skill_id,
                                uid,
                                skill_effects.len()
                            );
                        }
                        for effect in &mut skill_effects {
                            if effect.effect_type == Some(EffectType::Fightstep as i32)
                                && let Some(step) = effect.fight_step.as_mut()
                            {
                                drop_del_when_update_exists(&mut step.act_effect);
                            }
                        }
                        // Live shape: keep one trigger EFFECT step per entity and pack
                        // both 162 wrappers and flat effects in arrival order.
                        entity_step_effects.extend(skill_effects);
                    }
                    Ok(_) => {
                        if trace_target {
                            eprintln!("trace passive skill={} uid={} emitted=0", skill_id, uid);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("trigger skill={} uid={}: {}", skill_id, uid, e)
                    }
                }
            }
        }

        if !entity_step_effects.is_empty() {
            let mut effect_step = build_effect_step(entity_step_effects);
            // Stamp the trigger-owner uid on the EFFECT wrapper's from_id so
            // downstream `insert_trigger_into_matching_nested` can match the
            // step against a same-origin nested SKILL when embedding into
            // the host card step. Without this, the wrapper's from_id=0
            // default trips the early-return at trigger_embed.rs `trigger_from == 0`
            // and the step either fallback-splices into the wrong child or
            // gets lost entirely.
            effect_step.from_id = Some(uid);
            steps.push(effect_step);
        }
        if teammate_injury_hits > 0 {
            ctx.managers
                .buff_mgr
                .add_teammate_injury_not_reset(uid, teammate_injury_hits);
        }
    }

    steps
}

fn collect_battle_rule_skills(fight: &Fight) -> Vec<i32> {
    let cfg = config::configs::get();
    let episode_id = fight.episode_id.unwrap_or(0);
    let Some(battle_id) = cfg
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .map(|e| e.battle_id)
    else {
        return vec![];
    };
    let Some(battle) = cfg.battle.iter().find(|b| b.id == battle_id) else {
        return vec![];
    };
    if battle.addition_rule.is_empty() {
        return vec![];
    }

    let mut out = Vec::new();
    for entry in battle.addition_rule.split('|') {
        let mut parts = entry.split('#');
        let Some(prefix) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
            continue;
        };
        if !(1..=3).contains(&prefix) {
            continue;
        }
        let Some(rule_id) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
            continue;
        };
        let Some(rule) = cfg.rule.iter().find(|r| r.id == rule_id) else {
            continue;
        };
        let sid = rule.effect.parse::<i32>().ok().unwrap_or(0);
        if sid > 0 && !out.contains(&sid) {
            out.push(sid);
        }
    }
    out
}

fn skill_has_has_buff_id_condition(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return false;
    };
    let raws = [
        skill.condition1.as_str(),
        skill.condition2.as_str(),
        skill.condition3.as_str(),
        skill.condition4.as_str(),
        skill.condition5.as_str(),
        skill.condition6.as_str(),
        skill.condition7.as_str(),
        skill.condition8.as_str(),
        skill.condition9.as_str(),
        skill.condition10.as_str(),
    ];
    for raw in raws {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (cond, _) = parse_condition(trimmed);
        if condition_contains_has_buff_id(&cond) {
            return true;
        }
    }
    false
}

fn condition_contains_has_buff_id(cond: &ConditionType) -> bool {
    use ConditionType::*;
    match cond {
        HasBuffId { .. } => true,
        EnterFightAnd(parts) | EnterFightOr(parts) => {
            parts.iter().any(condition_contains_has_buff_id)
        }
        _ => false,
    }
}

fn extend_with_buff_granted_passives(ctx: &FightContext<'_>, uid: i64, skill_ids: &mut Vec<i32>) {
    for instance in ctx.managers.buff_mgr.get(uid) {
        crate::state::battle::utils::for_each_buff_feature_chain(
            instance.buff_id,
            |act_type, parts| {
                let value_parts: Vec<&str> = match act_type {
                    "AddPassiveSkills" => parts.iter().skip(1).copied().collect(),
                    "AddToTarget" | "AddToTargetNoLimit" | "UseDamageSkillAddToTarget" => {
                        parts.iter().skip(2).copied().collect()
                    }
                    _ => Vec::new(),
                };
                if value_parts.is_empty() {
                    return;
                }
                for raw in value_parts {
                    for piece in raw.split(',') {
                        if let Ok(skill_id) = piece.trim().parse::<i32>()
                            && skill_id > 0
                        {
                            let resolved_skill_id = resolve_with_euphoria(ctx.fight, uid, skill_id);
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

/// Pre-filter: does this skill have any condition that fires for the current event?
/// Returns true if all firing conditions on this passive are ActiveUseSkill/CombatNone.
/// These are handled inline during card play and should not fire again in combat triggers.
fn is_active_use_skill_passive(skill_id: i32) -> bool {
    let mut has_any = false;
    for i in 1..=10i32 {
        let condition_str = get_condition(skill_id, i);
        if condition_str.is_empty() {
            break;
        }
        has_any = true;
        let (condition, _) = parse_condition(&condition_str);
        match condition {
            ConditionType::ActiveUseSkill
            | ConditionType::ActiveUseSkillId { .. }
            | ConditionType::ActOrder { .. }
            | ConditionType::UseSkillEffectTag { .. }
            | ConditionType::UseSpecificSkill { .. }
            | ConditionType::UseHurtSkill
            | ConditionType::CombatNone => {}
            _ => return false,
        }
    }
    has_any
}

fn is_enter_fight_only_passive(skill_id: i32) -> bool {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return false;
    };

    let raw_conditions = [
        skill.condition1.as_str(),
        skill.condition2.as_str(),
        skill.condition3.as_str(),
        skill.condition4.as_str(),
        skill.condition5.as_str(),
        skill.condition6.as_str(),
        skill.condition7.as_str(),
        skill.condition8.as_str(),
        skill.condition9.as_str(),
        skill.condition10.as_str(),
    ];

    let mut has_any = false;
    for raw in raw_conditions {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        has_any = true;
        let (cond, _) = parse_condition(trimmed);
        if !condition_is_enter_fight_only(&cond) {
            return false;
        }
    }

    has_any
}

fn condition_is_enter_fight_only(condition: &ConditionType) -> bool {
    match condition {
        ConditionType::EnterFight { .. } => true,
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => {
            !conds.is_empty() && conds.iter().all(condition_is_enter_fight_only)
        }
        _ => false,
    }
}

fn team_injury_hits_for_uid(uid: i64, event: &TriggerEvent) -> i32 {
    event
        .damaged_uids
        .iter()
        .filter(|&&d| d.signum() == uid.signum())
        .count() as i32
}

/// Distinct teammates (excluding `uid` itself) that took damage this event.
/// LIVE fires TeammateInjuryCount-reactive passives once per distinct injured
/// ally, so this drives the replay count for those skills.
fn distinct_teammate_injuries_excluding_self(uid: i64, event: &TriggerEvent) -> i32 {
    let mut seen: HashSet<i64> = HashSet::new();
    for &d in &event.damaged_uids {
        if d.signum() == uid.signum() && d != uid {
            seen.insert(d);
        }
    }
    seen.len() as i32
}

pub fn skill_should_fire(
    uid: i64,
    skill_id: i32,
    event: &TriggerEvent,
    teammate_injury_hits: i32,
    teammate_injury_not_reset: i32,
) -> bool {
    let mut saw_event_condition = false;
    for i in 1..=10i32 {
        let condition_str = get_condition(skill_id, i);
        if condition_str.is_empty() {
            break;
        }
        let (condition, _negated) = parse_condition(&condition_str);
        if let Some(pass) = condition_fires_for(
            skill_id,
            &condition,
            uid,
            event,
            teammate_injury_hits,
            teammate_injury_not_reset,
        ) {
            saw_event_condition = true;
            if pass {
                return true;
            }
        }
    }
    if !saw_event_condition && is_damage_reactive_extra_skill(skill_id) {
        return event.triggered_bullet_for(uid) || event.same_team_dealt_damage(uid);
    }
    false
}

fn collect_nested_skill_ids(effects: &[ActEffect], out: &mut Vec<i32>) {
    for effect in effects {
        if let Some(step) = &effect.fight_step {
            if step.act_type == Some(sonettobuf::fight_step::ActType::Skill as i32)
                && let Some(id) = step.act_id
                && id > 0
            {
                out.push(id);
            }
            collect_nested_skill_ids(&step.act_effect, out);
        }
    }
}

fn collect_nested_skill_uses(
    fight: &sonettobuf::Fight,
    effects: &[ActEffect],
    out: &mut Vec<(i64, i32, bool, i64)>,
) {
    for effect in effects {
        if let Some(step) = &effect.fight_step {
            if step.act_type == Some(sonettobuf::fight_step::ActType::Skill as i32)
                && let (Some(uid), Some(skill_id)) = (step.from_id, step.act_id)
                && skill_id > 0
            {
                let used_ex = crate::state::battle::skill::get_entity(fight, uid)
                    .and_then(|e| e.ex_skill)
                    .map(|ex| ex == skill_id)
                    .unwrap_or(false);
                out.push((uid, skill_id, used_ex, step.to_id.unwrap_or(0)));
            }
            collect_nested_skill_uses(fight, &step.act_effect, out);
        }
    }
}

/// Check whether a condition fires for the current trigger event on a given entity.
fn condition_fires_for(
    skill_id: i32,
    condition: &ConditionType,
    uid: i64,
    event: &TriggerEvent,
    teammate_injury_hits: i32,
    teammate_injury_not_reset: i32,
) -> Option<bool> {
    let teammate_used_ex = event.teammate_used_ex_skill(uid);

    match condition {
        ConditionType::BeAttacked => {
            if !event.was_attacked_by_enemy(uid) {
                return Some(false);
            }
            // Live parity: logicTarget=201 (single + random add-on) does not
            // trigger BeAttacked passives on the secondary random target lane.
            let logic_target = skill_logic_target(event.skill_id);
            if logic_target == 201 && uid != event.primary_target_uid {
                return Some(false);
            }
            Some(true)
        }
        ConditionType::ActiveUseSkill => Some(event.skill_used_for_passive_owner(uid).is_some()),
        ConditionType::ActiveUseSkillId { skill_ids } => Some(
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, sid, _, _)| skill_ids.contains(&sid))
                .unwrap_or(false),
        ),
        ConditionType::ActOrder { order_index } => Some(
            event.skill_used_for_passive_owner(uid).is_some()
                && event.action_order_index > 0
                && event.action_order_index == *order_index,
        ),
        ConditionType::UseSkillEffectTag { effect_tag } => Some(
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, sid, _, _)| {
                    active_skill_effect_tag(sid)
                        .map(|tag| tag == *effect_tag)
                        .unwrap_or(false)
                })
                .unwrap_or(false),
        ),
        ConditionType::UseSpecificSkill { skill_id } => Some(
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, sid, _, _)| skill_matches_specific(sid, *skill_id))
                .unwrap_or(false),
        ),
        ConditionType::UseHurtSkill => Some(
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, sid, _, _)| skill_is_hurt(sid))
                .unwrap_or(false),
        ),
        ConditionType::CombatNone => Some(
            event.skill_used_for_passive_owner(uid).is_some()
                || event.took_damage(uid)
                || event.dealt_damage(uid),
        ),
        ConditionType::HurtNotRestraint => Some(event.dealt_damage(uid)),
        ConditionType::HurtRestraint => Some(event.dealt_damage(uid)),
        ConditionType::TeammateInjuryCount { threshold } => {
            Some(teammate_injury_hits >= *threshold)
        }
        ConditionType::TeammateInjuryCountNotReset { threshold } => {
            Some(teammate_injury_not_reset >= *threshold && teammate_injury_hits > 0)
        }
        ConditionType::TeamInjuryCountRound => Some(
            event
                .damaged_uids
                .iter()
                .any(|&d| d.signum() == uid.signum()),
        ),
        ConditionType::TeammateUseExSkill => Some(teammate_used_ex),
        ConditionType::UseExSkill => Some(if event.from_wrapper_card && uid == event.caster_uid {
            false
        } else {
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, _, used_ex, _)| used_ex)
                .unwrap_or(false)
        }),
        // "PerDecrExPoint" should only become a trigger candidate for the
        // acting entity on an EX-consuming action. Exact threshold check stays
        // in skill-side condition evaluation.
        ConditionType::PerDecrExPoint { .. } => Some(
            event
                .skill_used_for_passive_owner(uid)
                .map(|(_, _, used_ex, _)| used_ex)
                .unwrap_or(false),
        ),
        // Leave NoActRound gating to skill-side condition checks.
        // Returning None keeps this trigger candidate eligible.
        ConditionType::NoActRound => None,
        ConditionType::TriggerBullet => Some(
            event.caster_uid == uid
                || event.triggered_bullet_for(uid)
                || (is_damage_reactive_extra_skill(skill_id) && event.triggered_bullet_for(uid)),
        ),
        ConditionType::BuffIdDel { buff_ids } => {
            let target_cross_side =
                event.primary_target_uid != 0 && event.primary_target_uid.signum() != uid.signum();
            Some(target_cross_side && deleted_matches(&event.deleted_buff_ids, buff_ids))
        }
        // Static/state conditions are evaluated in skill-side condition checks.
        // Keep trigger candidate gating event-driven here.
        ConditionType::TargetCareer { .. }
        | ConditionType::HasBuffId { .. }
        | ConditionType::NoBuffId { .. }
        | ConditionType::LifeLess { .. }
        | ConditionType::LifeMore { .. } => None,
        // Static-gate conditions used by several teammate-action passives.
        // Treat as reactive on same-side teammate actions; exact gating stays
        // in executor/check_condition.
        ConditionType::CareerCheck { .. } => Some(
            uid.signum() == event.caster_uid.signum()
                && uid != event.caster_uid
                && event.used_ex_skill,
        ),
        // Blood-pool state queries are not events — treat as None so they
        // don't turn a battle-start AND-clause (e.g. 435311's
        // TargetCareer AND BloodPoolMax) into a combat-reactive trigger.
        // Leave exact pool/max validation to skill-side condition checks.
        ConditionType::BloodPool | ConditionType::BloodPoolMax { .. } => None,
        ConditionType::TeammateAlive { .. } => None,
        ConditionType::Dead => Some(false), // handled separately
        ConditionType::EnterFightAnd(conds) => {
            let mut any_relevant = false;
            let mut all_pass = true;
            for c in conds {
                if let Some(pass) = condition_fires_for(
                    skill_id,
                    c,
                    uid,
                    event,
                    teammate_injury_hits,
                    teammate_injury_not_reset,
                ) {
                    any_relevant = true;
                    all_pass &= pass;
                }
            }
            if any_relevant { Some(all_pass) } else { None }
        }
        ConditionType::EnterFightOr(conds) => {
            let mut any_relevant = false;
            let mut any_pass = false;
            for c in conds {
                if let Some(pass) = condition_fires_for(
                    skill_id,
                    c,
                    uid,
                    event,
                    teammate_injury_hits,
                    teammate_injury_not_reset,
                ) {
                    any_relevant = true;
                    any_pass |= pass;
                }
            }
            if any_relevant { Some(any_pass) } else { None }
        }
        _ => None,
    }
}

fn skill_logic_target(skill_id: i32) -> i32 {
    if skill_id <= 0 {
        return 0;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    cfg.skill_effect
        .iter()
        .find(|s| s.id == effect_id)
        .and_then(|s| s.logic_target.trim().parse::<i32>().ok())
        .unwrap_or(0)
}

fn active_skill_effect_tag(skill_id: i32) -> Option<i32> {
    if skill_id <= 0 {
        return None;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    cfg.skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .map(|row| row.effect_tag)
}

fn skill_matches_specific(skill_id: i32, wanted: i32) -> bool {
    if skill_id <= 0 || wanted <= 0 {
        return false;
    }
    if skill_id == wanted || resolve_skill_effect_id(skill_id) == wanted {
        return true;
    }

    let cfg = config::configs::get();
    let Some(skill) = cfg.skill.get(skill_id) else {
        return false;
    };

    if wanted <= 3 && skill.skill_rank == wanted {
        return true;
    }

    wanted == 4
}

fn skill_is_hurt(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    if cfg
        .skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .map(|row| row.damage_rate > 0)
        .unwrap_or(false)
    {
        return true;
    }

    SKILL_CACHE
        .get(&effect_id)
        .map(|rows| {
            rows.iter()
                .any(|row| matches!(row.behavior, BehaviorType::Damage { .. }))
        })
        .unwrap_or(false)
}

fn collect_deleted_buff_ids(effects: &[ActEffect], out: &mut Vec<i32>) {
    for effect in effects {
        if effect.effect_type == Some(EffectType::Buffdel as i32) {
            let buff_id = effect
                .buff
                .as_ref()
                .and_then(|b| b.buff_id)
                .or(effect.effect_num)
                .unwrap_or(0);
            if buff_id > 0 && !out.contains(&buff_id) {
                out.push(buff_id);
            }
            if buff_id > 0 {
                let cfg = config::configs::get();
                if let Some(type_id) = cfg
                    .skill_buff
                    .iter()
                    .find(|b| b.id == buff_id)
                    .map(|b| b.type_id)
                    && type_id > 0
                    && !out.contains(&type_id)
                {
                    out.push(type_id);
                }
            }
        }
        if let Some(step) = &effect.fight_step {
            collect_deleted_buff_ids(&step.act_effect, out);
        }
    }
}

fn collect_added_buffs(effects: &[ActEffect], out_uids: &mut Vec<i64>, out_ids: &mut Vec<i32>) {
    for effect in effects {
        if effect.effect_type == Some(EffectType::Buffadd as i32) {
            if let Some(uid) = effect.buff.as_ref().and_then(|b| b.uid)
                && uid > 0
                && !out_uids.contains(&uid)
            {
                out_uids.push(uid);
            }
            if let Some(buff_id) = effect
                .buff
                .as_ref()
                .and_then(|b| b.buff_id)
                .or(effect.effect_num)
                && buff_id > 0
                && !out_ids.contains(&buff_id)
            {
                out_ids.push(buff_id);
            }
        }
        if let Some(step) = &effect.fight_step {
            collect_added_buffs(&step.act_effect, out_uids, out_ids);
        }
    }
}

// --- Config field accessors ---
// Look up by skill_id each call to avoid holding a reference into the config guard.

fn get_condition(skill_id: i32, i: i32) -> String {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return String::new();
    };
    match i {
        1 => skill.condition1.clone(),
        2 => skill.condition2.clone(),
        3 => skill.condition3.clone(),
        4 => skill.condition4.clone(),
        5 => skill.condition5.clone(),
        6 => skill.condition6.clone(),
        7 => skill.condition7.clone(),
        8 => skill.condition8.clone(),
        9 => skill.condition9.clone(),
        10 => skill.condition10.clone(),
        _ => String::new(),
    }
}

fn should_use_strict_event_only(fight: &Fight, skill_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == skill_id) else {
        return false;
    };

    // Wrapper shape used by direct-use-big-skill prep lane, matched by
    // condition/behavior topology instead of hardcoded condition IDs.
    let (cond1, _) = parse_condition(&skill.condition1);
    let (cond2, _) = parse_condition(&skill.condition2);
    let (cond3, _) = parse_condition(&skill.condition3);
    let b1 = add_buff_behavior_id(&skill.behavior1);
    let b2 = add_buff_behavior_id(&skill.behavior2);
    let b3 = add_buff_behavior_id(&skill.behavior3);
    let cond2_ids = no_buff_ids(&cond2);
    let cond3_ids = no_buff_ids(&cond3);
    let wrapper_shape = matches!(cond1, ConditionType::EnterFight { condition_id: 5 })
        && b1.is_some()
        && b2.is_some()
        && b3.is_some()
        && {
            let mut expect = vec![b1.unwrap_or_default(), b2.unwrap_or_default()];
            expect.sort_unstable();
            let mut actual = cond2_ids;
            actual.sort_unstable();
            actual == expect
        }
        && cond3_ids == vec![b2.unwrap_or_default()]
        && skill.behavior1.starts_with("1#")
        && skill.behavior2.starts_with("1#")
        && skill.behavior3.starts_with("1#")
        && skill.behavior_target1 == "103"
        && skill.behavior_target2 == "103"
        && skill.behavior_target3 == "103";
    if !wrapper_shape {
        return false;
    }

    // Restrict to exact skill ids declared by battle rules for current episode.
    let episode_id = fight.episode_id.unwrap_or(0);
    let Some(battle_id) = cfg
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .map(|e| e.battle_id)
    else {
        return false;
    };
    let Some(battle) = cfg.battle.iter().find(|b| b.id == battle_id) else {
        return false;
    };
    if battle.addition_rule.is_empty() {
        return false;
    }

    battle
        .addition_rule
        .split('|')
        .filter_map(|entry| {
            let mut parts = entry.split('#');
            let prefix = parts.next()?.parse::<i32>().ok()?;
            if !(1..=3).contains(&prefix) {
                return None;
            }
            parts.next()?.parse::<i32>().ok()
        })
        .any(|sid| sid == skill_id)
}

fn is_damage_reactive_extra_skill(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    cfg.skill_effect
        .iter()
        .find(|s| s.id == effect_id)
        .map(|s| s.is_extra > 0 && s.damage_rate > 0)
        .unwrap_or(false)
}

fn skill_has_bullet_behavior(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return false;
    };
    [
        skill.behavior1.as_str(),
        skill.behavior2.as_str(),
        skill.behavior3.as_str(),
        skill.behavior4.as_str(),
        skill.behavior5.as_str(),
        skill.behavior6.as_str(),
        skill.behavior7.as_str(),
        skill.behavior8.as_str(),
        skill.behavior9.as_str(),
        skill.behavior10.as_str(),
    ]
    .iter()
    .filter(|raw| !raw.is_empty())
    .any(|raw| behavior_triggers_bullet(raw))
}

fn behavior_triggers_bullet(raw: &str) -> bool {
    let cfg = config::configs::get();
    let parts: Vec<&str> = raw.split('#').collect();
    let behavior_id = parts
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let behavior_type = cfg
        .skill_behavior
        .iter()
        .find(|b| b.id == behavior_id)
        .map(|b| b.r#type.as_str())
        .unwrap_or("");
    if behavior_type.contains("Bullet") || behavior_type == "DistributeBuff" {
        return true;
    }

    let buff_id = match behavior_type {
        "ConsumeBloodAddBuff" | "ConsumeBloodAddBuff2" => parts
            .get(2)
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0),
        _ => 0,
    };
    buff_id > 0 && buff_has_feature_type(buff_id, "Bullet")
}

fn buff_has_feature_type(buff_id: i32, wanted_type: &str) -> bool {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    buff.features.split('|').any(|entry| {
        let act_id = entry
            .split('#')
            .next()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);
        cfg.buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type.as_str() == wanted_type)
            .unwrap_or(false)
    })
}

fn has_bullet_trigger(skill_id: i32, nested_skill_uses: &[(i64, i32, bool, i64)]) -> bool {
    if skill_has_bullet_behavior(skill_id) {
        return true;
    }
    if nested_skill_uses
        .iter()
        .any(|(_, sid, _, _)| skill_has_bullet_behavior(*sid))
    {
        return true;
    }
    false
}

fn add_buff_behavior_id(raw: &str) -> Option<i32> {
    let mut parts = raw.split('#');
    let kind = parts.next()?.trim().parse::<i32>().ok()?;
    if kind != 1 {
        return None;
    }
    parts.next()?.trim().parse::<i32>().ok()
}

fn no_buff_ids(cond: &ConditionType) -> Vec<i32> {
    match cond {
        ConditionType::NoBuffId { buff_ids } => buff_ids.clone(),
        ConditionType::EnterFightAnd(parts) | ConditionType::EnterFightOr(parts) => {
            let mut out = Vec::new();
            for c in parts {
                out.extend(no_buff_ids(c));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn drop_del_when_update_exists(effects: &mut Vec<ActEffect>) {
    let update_keys: HashSet<(i64, i32, i64)> = effects
        .iter()
        .filter_map(|e| {
            if e.effect_type != Some(EffectType::Buffupdate as i32) {
                return None;
            }
            let buff = e.buff.as_ref()?;
            Some((
                e.target_id.unwrap_or(0),
                buff.buff_id.unwrap_or(0),
                buff.uid.unwrap_or(0),
            ))
        })
        .collect();

    if update_keys.is_empty() {
        return;
    }

    effects.retain(|e| {
        if e.effect_type != Some(EffectType::Buffdel as i32) {
            return true;
        }
        let key = (
            e.target_id.unwrap_or(0),
            e.buff.as_ref().and_then(|b| b.buff_id).unwrap_or(0),
            e.buff.as_ref().and_then(|b| b.uid).unwrap_or(0),
        );
        !update_keys.contains(&key)
    });
}
