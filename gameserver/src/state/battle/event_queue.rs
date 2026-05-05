#![allow(dead_code)]

use std::collections::HashMap;

use sonettobuf::{
    ActEffect, BuffInfo, Fight, FightEntityInfo, FightHurtInfo as HurtInfo, FightStep,
    effect_type_enum::EffectType, fight_step,
};

use crate::state::battle::skill::sibling_coalesce::parent_skill_fans_out_into_mergeable_siblings;
use crate::state::battle::{
    fight_step::{ActEffectBuilder, effect_container_step, make_skill_step, wrap_step},
    manager::{
        buff_mgr::{BuffMgr, next_buff_uid_for_target},
        ex_point_mgr::ExPointMgr,
    },
    mechanics::bloodtithe::BloodtitheState,
    types::buff::BuffLayerType,
    utils::{buff_del, buff_get_act_common_params, buff_update},
};

/// A typed event recording a state mutation or visual emission
/// that should land in the FightStep stream.
#[derive(Debug, Clone)]
pub enum BattleEvent {
    BuffApply {
        target: i64,
        buff_id: i32,
        count: i32,
        layer: i32,
        from: i64,
        config_effect: Option<i32>,
    },
    BuffUpdate {
        target: i64,
        buff_uid: i64,
        new_count: i32,
        new_layer: i32,
    },
    BuffRemove {
        target: i64,
        buff_uid: i64,
    },
    BuffSyncAddWithUid {
        target: i64,
        buff_id: i32,
        from: i64,
        count: i32,
        layer: i32,
        buff_uid: i64,
    },
    BuffSyncAddWithUidAndEmitUpdate {
        target: i64,
        buff_id: i32,
        from: i64,
        sync_count: i32,
        sync_layer: i32,
        buff_uid: i64,
        emit_count: i32,
        emit_layer: i32,
    },
    Damage {
        target: i64,
        amount: i32,
        is_crit: bool,
        hurt_info: HurtInfo,
        from: i64,
        skill_id: Option<i32>,
    },
    Heal {
        target: i64,
        amount: i32,
        from: i64,
    },
    HealCrit {
        target: i64,
        amount: i32,
        from: i64,
    },
    ExPointChange {
        target: i64,
        delta: i32,
    },
    PowerChange {
        delta: i32,
    },
    BloodpoolValueChange {
        team_type: i32,
        target: i64,
        delta: i32,
    },
    BloodpoolMaxChange {
        team_type: i32,
        max: i32,
    },
    /// Bridge for migrations that already execute and sync state through
    /// legacy ActEffect builders but need queue-controlled shaping.
    SerializedActEffect {
        effect: ActEffect,
    },
    SkillEmit {
        skill_id: i32,
        from: i64,
        to: i64,
        children: Vec<BattleEvent>,
        kind: SkillEmitKind,
    },
    EffectMarker {
        effect_type: i32,
    },
}

/// Classifies how a `SkillEmit` event should be serialized into the
/// FightStep stream. Names follow the project's naming-conventions
/// skill: each variant is a domain noun phrase that reveals intent
/// without overloading common English/programming terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEmitKind {
    /// Top-level player card play (the player chose this card this turn).
    PlayerInitiated,
    /// Round-tied automatic emission walked by `passives::executor`.
    /// Includes hero passives (Insight, Euphoria) AND battle-rule-derived
    /// skills where `rule.json::effect == skill_id` (e.g. the `530000*`
    /// family that battle config attaches via `additionRule`).
    AutomaticPhase,
    /// Event-driven reactive walked by `trigger/combat.rs::expand_trigger_chain`.
    EventTriggered,
    /// Sourced from equipment (psychube) — `equip_skill.json` references
    /// the skill id. Phase 4 uses this kind to embed the emission in the
    /// next outgoing `PlayerInitiated` host of the same caster instead
    /// of letting it stand alone.
    EquipmentEmbedded,
}

#[derive(Debug, Default)]
pub struct HostEventAccumulator {
    direct: Vec<BattleEvent>,
    trigger_lane: Vec<BattleEvent>,
    be_attacked: Vec<BattleEvent>,
    injury: Vec<BattleEvent>,
}

#[derive(Debug, Clone, Copy)]
pub enum HostLane {
    Direct,
    Trigger,
    BeAttacked,
    Injury,
}

impl HostEventAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_direct(&mut self, event: BattleEvent) {
        self.direct.push(event);
    }

    pub fn push_trigger_lane(&mut self, event: BattleEvent) {
        self.trigger_lane.push(event);
    }

    pub fn push_be_attacked(&mut self, event: BattleEvent) {
        self.be_attacked.push(event);
    }

    pub fn push_injury(&mut self, event: BattleEvent) {
        self.injury.push(event);
    }

    pub fn child_count(&self) -> usize {
        self.direct.len() + self.trigger_lane.len() + self.be_attacked.len() + self.injury.len()
    }

    pub fn lane_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.direct.len(),
            self.trigger_lane.len(),
            self.be_attacked.len(),
            self.injury.len(),
        )
    }

    /// Iterate every captured lane in push-order and yield the
    /// inner ActEffect for SerializedActEffect events.
    pub fn iter_captured_act_effects(&self) -> impl Iterator<Item = &ActEffect> {
        self.direct
            .iter()
            .chain(self.trigger_lane.iter())
            .chain(self.be_attacked.iter())
            .chain(self.injury.iter())
            .filter_map(|event| match event {
                BattleEvent::SerializedActEffect { effect } => Some(effect),
                _ => None,
            })
    }

    /// Iterate a single captured lane for diagnostics.
    pub fn lane_iter(&self, lane: HostLane) -> impl Iterator<Item = &ActEffect> {
        let slice: &[BattleEvent] = match lane {
            HostLane::Direct => &self.direct,
            HostLane::Trigger => &self.trigger_lane,
            HostLane::BeAttacked => &self.be_attacked,
            HostLane::Injury => &self.injury,
        };
        slice.iter().filter_map(|event| match event {
            BattleEvent::SerializedActEffect { effect } => Some(effect),
            _ => None,
        })
    }

    /// Drain everything pushed into `lane` since `lane_offset_before_emit`
    /// and splice the resulting `ActEffect`s into `host_step.act_effect`
    /// at the position returned by `position_fn`. No-op when the drain
    /// is empty.
    ///
    /// `lane_offset_before_emit` is the lane's `lane_iter().count()`
    /// captured BEFORE the events were pushed — used to skip earlier
    /// content in the same lane (e.g. so the monitor splice doesn't
    /// re-pick up the trigger events).
    pub fn splice_lane_drain_into_host(
        &self,
        lane: HostLane,
        lane_offset_before_emit: usize,
        host_step: &mut FightStep,
        position_fn: impl FnOnce(&FightStep) -> usize,
    ) {
        let drained: Vec<ActEffect> = self
            .lane_iter(lane)
            .skip(lane_offset_before_emit)
            .cloned()
            .collect();
        if drained.is_empty() {
            return;
        }
        let insert_at = position_fn(host_step);
        host_step.act_effect.splice(insert_at..insert_at, drained);
    }
}

#[derive(Debug)]
pub struct HostMembershipDiff {
    /// Captured effects that were missing from the final host step.
    pub missing_from_host: Vec<ActEffect>,
}

pub fn check_host_lane_membership(
    captured: &[&ActEffect],
    host_act_effects: &[ActEffect],
) -> Result<(), HostMembershipDiff> {
    let mut consumed = vec![false; host_act_effects.len()];
    let mut missing = Vec::new();

    for &captured_effect in captured {
        let matched = host_act_effects
            .iter()
            .enumerate()
            .find(|(idx, host_effect)| !consumed[*idx] && *host_effect == captured_effect);
        match matched {
            Some((idx, _)) => consumed[idx] = true,
            None => missing.push(captured_effect.clone()),
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(HostMembershipDiff {
            missing_from_host: missing,
        })
    }
}

#[derive(Debug, Default)]
pub struct EventQueue {
    events: Vec<BattleEvent>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn push(&mut self, event: BattleEvent) {
        self.events.push(event);
    }

    pub fn drain(&mut self) -> Vec<BattleEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Owns mut-borrows for the drain step.
pub struct EventContext<'a> {
    pub fight: &'a mut Fight,
    pub buff_mgr: &'a mut BuffMgr,
    pub ex_point_mgr: &'a mut ExPointMgr,
    pub bloodtithe: &'a mut BloodtitheState,
}

fn find_entity_mut(fight: &mut Fight, uid: i64) -> Option<&mut FightEntityInfo> {
    if let Some(attacker) = fight.attacker.as_mut()
        && let Some(entity) = attacker
            .entitys
            .iter_mut()
            .chain(attacker.sub_entitys.iter_mut())
            .find(|entity| entity.uid == Some(uid))
    {
        return Some(entity);
    }
    if let Some(defender) = fight.defender.as_mut()
        && let Some(entity) = defender
            .entitys
            .iter_mut()
            .chain(defender.sub_entitys.iter_mut())
            .find(|entity| entity.uid == Some(uid))
    {
        return Some(entity);
    }
    None
}

fn coalesce_sibling_skills(mut children: Vec<ActEffect>) -> Vec<ActEffect> {
    let mut occurrences: HashMap<i32, Vec<usize>> = HashMap::new();

    for (idx, effect) in children.iter().enumerate() {
        let Some(162) = effect.effect_type else {
            continue;
        };
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        if step.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }
        let Some(act_id) = step.act_id else {
            continue;
        };
        if act_id <= 0 {
            continue;
        }
        occurrences.entry(act_id).or_default().push(idx);
    }

    let mut duplicate_indices: Vec<usize> = Vec::new();
    for indices in occurrences.values() {
        if indices.len() < 2 {
            continue;
        }
        let first_idx = indices[0];
        let mut merged_children: Vec<ActEffect> = Vec::new();
        for &dup_idx in &indices[1..] {
            if let Some(step) = children[dup_idx].fight_step.as_ref() {
                merged_children.extend(step.act_effect.clone());
            }
            duplicate_indices.push(dup_idx);
        }
        if let Some(first_step) = children[first_idx].fight_step.as_mut() {
            first_step.act_effect.extend(merged_children);
        }
    }

    duplicate_indices.sort_unstable_by(|a, b| b.cmp(a));
    for idx in duplicate_indices {
        children.remove(idx);
    }

    children
}

/// Serialize a queue to FightStep ActEffects. Phase 1 only: this
/// is a stub that returns an empty Vec. Phase 2 migrations will
/// fill it in incrementally as each migrated leaf adds its own
/// case.
pub fn drain_to_fight_steps(
    events: Vec<BattleEvent>,
    _ctx: &mut EventContext<'_>,
) -> Vec<ActEffect> {
    let mut out = Vec::with_capacity(events.len());

    for event in events {
        match event {
            BattleEvent::BuffApply {
                target,
                buff_id,
                count,
                layer,
                from,
                config_effect,
            } => {
                let buff_uid = next_buff_uid_for_target(target);
                let duration = config::configs::get()
                    .skill_buff
                    .iter()
                    .find(|buff| buff.id == buff_id)
                    .map(|buff| buff.during_time)
                    .unwrap_or(0);
                let act_common_params = buff_get_act_common_params(buff_id);

                _ctx.buff_mgr
                    .add_with_uid(target, buff_id, from, count, layer, buff_uid);
                out.push(ActEffect {
                    effect_type: Some(EffectType::Buffadd as i32),
                    target_id: Some(target),
                    effect_num: Some(buff_id),
                    config_effect,
                    buff: Some(BuffInfo {
                        buff_id: Some(buff_id),
                        duration: Some(duration),
                        uid: Some(buff_uid),
                        ex_info: Some(0),
                        from_uid: Some(from),
                        count: Some(count),
                        act_common_params: Some(act_common_params),
                        layer: Some(layer),
                        r#type: Some(BuffLayerType::Normal as i32),
                        act_info: vec![],
                    }),
                    ..Default::default()
                });
            }
            BattleEvent::BuffUpdate {
                target,
                buff_uid,
                new_count,
                new_layer,
            } => {
                let existing = _ctx
                    .buff_mgr
                    .get(target)
                    .iter()
                    .find(|instance| instance.uid == buff_uid)
                    .cloned();

                if let Some(instance) = existing {
                    let updated = _ctx.buff_mgr.set_instance_count_layer(
                        target, buff_uid, new_count, new_layer,
                    );
                    if updated {
                        out.push(buff_update(
                            target,
                            instance.from_uid,
                            instance.buff_id,
                            buff_uid,
                            new_count,
                            new_layer,
                        ));
                    }
                }
            }
            BattleEvent::BuffRemove { target, buff_uid } => {
                let removed = _ctx
                    .buff_mgr
                    .get(target)
                    .iter()
                    .find(|instance| instance.uid == buff_uid)
                    .cloned();
                _ctx.buff_mgr.remove_by_uid(target, buff_uid);

                if let Some(instance) = removed {
                    out.push(buff_del(
                        target,
                        buff_uid,
                        instance.buff_id,
                        instance.from_uid,
                    ));
                }
            }
            BattleEvent::BuffSyncAddWithUid {
                target,
                buff_id,
                from,
                count,
                layer,
                buff_uid,
            } => {
                _ctx.buff_mgr
                    .add_with_uid(target, buff_id, from, count, layer, buff_uid);
            }
            BattleEvent::BuffSyncAddWithUidAndEmitUpdate {
                target,
                buff_id,
                from,
                sync_count,
                sync_layer,
                buff_uid,
                emit_count,
                emit_layer,
            } => {
                _ctx.buff_mgr.add_with_uid(
                    target,
                    buff_id,
                    from,
                    sync_count,
                    sync_layer,
                    buff_uid,
                );
                out.push(buff_update(
                    target, from, buff_id, buff_uid, emit_count, emit_layer,
                ));
            }
            BattleEvent::Damage {
                target,
                amount,
                is_crit,
                mut hurt_info,
                from,
                skill_id,
            } => {
                let damage = amount.max(0);
                if let Some(entity) = find_entity_mut(_ctx.fight, target) {
                    let current_hp = entity.current_hp.unwrap_or(0);
                    let new_hp = current_hp.saturating_sub(damage);
                    entity.current_hp = Some(new_hp);
                }
                _ctx.ex_point_mgr.apply_damage(target, damage);

                if hurt_info.from_uid.is_none() {
                    hurt_info.from_uid = Some(from);
                }
                if hurt_info.skill_id.is_none()
                    && let Some(skill_id) = skill_id
                {
                    hurt_info.skill_id = Some(skill_id);
                }
                if hurt_info.damage.is_none() {
                    hurt_info.damage = Some(damage);
                }
                let effect_type = if is_crit {
                    EffectType::Crit as i32
                } else {
                    EffectType::Damage as i32
                };
                if hurt_info.hurt_effect.is_none() {
                    hurt_info.hurt_effect = Some(effect_type);
                }

                out.push(ActEffect {
                    effect_type: Some(effect_type),
                    target_id: Some(target),
                    effect_num: Some(damage),
                    config_effect: hurt_info.config_effect,
                    hurt_info: Some(hurt_info),
                    ..Default::default()
                });
            }
            BattleEvent::Heal {
                target,
                amount,
                from: _from,
            } => {
                if let Some(entity) = find_entity_mut(_ctx.fight, target) {
                    let current_hp = entity.current_hp.unwrap_or(0);
                    let max_hp = entity
                        .attr
                        .as_ref()
                        .and_then(|attr| attr.hp)
                        .unwrap_or(current_hp);
                    let new_hp = (current_hp + amount).min(max_hp);
                    entity.current_hp = Some(new_hp);
                    _ctx.ex_point_mgr.set_hp(target, new_hp);
                }

                out.push(ActEffect {
                    effect_type: Some(EffectType::Heal as i32),
                    target_id: Some(target),
                    effect_num: Some(amount),
                    ..Default::default()
                });
            }
            BattleEvent::HealCrit {
                target,
                amount,
                from: _from,
            } => {
                if let Some(entity) = find_entity_mut(_ctx.fight, target) {
                    let current_hp = entity.current_hp.unwrap_or(0);
                    let max_hp = entity
                        .attr
                        .as_ref()
                        .and_then(|attr| attr.hp)
                        .unwrap_or(current_hp);
                    let new_hp = (current_hp + amount).min(max_hp);
                    entity.current_hp = Some(new_hp);
                    _ctx.ex_point_mgr.set_hp(target, new_hp);
                }

                out.push(ActEffect {
                    effect_type: Some(EffectType::Healcrit as i32),
                    target_id: Some(target),
                    effect_num: Some(amount),
                    ..Default::default()
                });
            }
            BattleEvent::ExPointChange { target, delta } => {
                _ctx.ex_point_mgr.add_ex_point(target, delta);
                out.push(ActEffect {
                    effect_type: Some(EffectType::Expointchange as i32),
                    effect_num: Some(delta),
                    target_id: Some(target),
                    ..Default::default()
                });
            }
            BattleEvent::PowerChange { delta } => out.push(ActEffect {
                effect_type: Some(EffectType::Powerchange as i32),
                effect_num: Some(delta),
                ..Default::default()
            }),
            BattleEvent::BloodpoolValueChange {
                team_type,
                target,
                delta,
            } => {
                _ctx.bloodtithe.add_value(team_type, delta);
                out.push(ActEffectBuilder::bloodpool_value_change(
                    target, team_type, delta,
                ));
            }
            BattleEvent::BloodpoolMaxChange { team_type, max } => {
                _ctx.bloodtithe.set_max(team_type, max);
                out.push(ActEffectBuilder::bloodpool_max_change(team_type, max));
            }
            BattleEvent::SerializedActEffect { effect } => out.push(effect),
            BattleEvent::SkillEmit {
                skill_id,
                from,
                to,
                children,
                kind: SkillEmitKind::PlayerInitiated,
            } => {
                let child_effects = drain_to_fight_steps(children, _ctx);
                out.push(wrap_step(make_skill_step(
                    from,
                    to,
                    skill_id,
                    0,
                    child_effects,
                )));
            }
            BattleEvent::SkillEmit {
                skill_id,
                from,
                to,
                children,
                kind: SkillEmitKind::EventTriggered,
            } => {
                let mut child_effects = drain_to_fight_steps(children, _ctx);
                if parent_skill_fans_out_into_mergeable_siblings(skill_id) {
                    child_effects = coalesce_sibling_skills(child_effects);
                }
                out.push(wrap_step(make_skill_step(
                    from,
                    to,
                    skill_id,
                    0,
                    child_effects,
                )));
            }
            BattleEvent::SkillEmit {
                skill_id,
                from,
                to,
                children,
                kind: SkillEmitKind::AutomaticPhase,
            } => {
                let mut child_effects = drain_to_fight_steps(children, _ctx);
                if parent_skill_fans_out_into_mergeable_siblings(skill_id) {
                    child_effects = coalesce_sibling_skills(child_effects);
                }
                let inner_skill = make_skill_step(from, to, skill_id, 0, child_effects);
                let inner_effect = effect_container_step(0, 0, 0, vec![wrap_step(inner_skill)]);
                let outer_effect = effect_container_step(0, 0, 0, vec![wrap_step(inner_effect)]);
                out.push(wrap_step(outer_effect));
            }
            BattleEvent::SkillEmit {
                skill_id,
                from,
                to,
                children,
                kind: SkillEmitKind::EquipmentEmbedded,
            } => {
                let child_effects = drain_to_fight_steps(children, _ctx);
                out.push(wrap_step(make_skill_step(
                    from,
                    to,
                    skill_id,
                    0,
                    child_effects,
                )));
            }
            _ => {}
        }
    }

    out
}

pub fn skill_step_to_event_triggered(step: FightStep) -> BattleEvent {
    let from = step.from_id.unwrap_or(0);
    let to = step.to_id.unwrap_or(0);
    let skill_id = step.act_id.unwrap_or(0);
    let children = step
        .act_effect
        .into_iter()
        .map(|effect| BattleEvent::SerializedActEffect { effect })
        .collect();
    BattleEvent::SkillEmit {
        skill_id,
        from,
        to,
        children,
        kind: SkillEmitKind::EventTriggered,
    }
}

pub fn fight_step_to_event(step: FightStep) -> BattleEvent {
    if step.act_type == Some(fight_step::ActType::Skill as i32) {
        skill_step_to_event_triggered(step)
    } else {
        BattleEvent::SerializedActEffect {
            effect: wrap_step(step),
        }
    }
}

pub fn serialize_leaf_event(event: BattleEvent) -> ActEffect {
    let mut queue = EventQueue::new();
    queue.push(event);

    let mut fight = Fight::default();
    let mut buff_mgr = BuffMgr::new();
    let mut ex_point_mgr = ExPointMgr::new();
    let mut bloodtithe = BloodtitheState::new();
    let mut ctx = EventContext {
        fight: &mut fight,
        buff_mgr: &mut buff_mgr,
        ex_point_mgr: &mut ex_point_mgr,
        bloodtithe: &mut bloodtithe,
    };

    drain_to_fight_steps(queue.drain(), &mut ctx)
        .into_iter()
        .next()
        .expect("leaf event should serialize to a single ActEffect")
}

#[cfg(test)]
mod tests {
    use super::{
        BattleEvent, EventContext, EventQueue, SkillEmitKind, coalesce_sibling_skills,
        drain_to_fight_steps, fight_step_to_event,
    };
    use crate::state::battle::{
        fight_step::{effect_container_step, make_skill_step, wrap_step},
        manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr},
        mechanics::bloodtithe::BloodtitheState,
    };
    use sonettobuf::{ActEffect, Fight, FightEntityInfo, FightTeam, HeroAttribute};
    use std::{path::PathBuf, sync::Once};

    static TEST_CONFIG_INIT: Once = Once::new();

    fn ensure_game_data_initialized() {
        TEST_CONFIG_INIT.call_once(|| {
            if config::configs::try_get().is_some() {
                return;
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let excel_dir = root.join("data").join("excel2json");
            if excel_dir.exists()
                && let Some(path) = excel_dir.to_str()
            {
                let _ = config::configs::init(path);
            }
        });
    }

    fn test_ctx() -> EventContext<'static> {
        let fight = Box::leak(Box::new(Fight::default()));
        let buff_mgr = Box::leak(Box::new(BuffMgr::new()));
        let ex_point_mgr = Box::leak(Box::new(ExPointMgr::new()));
        let bloodtithe = Box::leak(Box::new(BloodtitheState::new()));
        EventContext {
            fight,
            buff_mgr,
            ex_point_mgr,
            bloodtithe,
        }
    }

    fn synthetic_effect(effect_type: i32, effect_num: i32) -> ActEffect {
        ActEffect {
            effect_type: Some(effect_type),
            effect_num: Some(effect_num),
            target_id: Some(42),
            ..Default::default()
        }
    }

    fn synthetic_skill_wrapper(skill_id: i32, child: ActEffect) -> ActEffect {
        wrap_step(make_skill_step(1001, 2002, skill_id, 0, vec![child]))
    }

    #[test]
    fn coalesce_sibling_skills_merges_duplicate_act_id_wrappers() {
        let first_child = synthetic_effect(301, 1);
        let second_child = synthetic_effect(302, 2);
        let out = coalesce_sibling_skills(vec![
            synthetic_skill_wrapper(30630122, first_child.clone()),
            synthetic_skill_wrapper(30630122, second_child.clone()),
        ]);

        assert_eq!(out.len(), 1);
        let merged = out[0]
            .fight_step
            .as_ref()
            .expect("merged output should carry skill wrapper");
        assert_eq!(merged.act_id, Some(30630122));
        assert_eq!(merged.act_effect, vec![first_child, second_child]);
    }

    #[test]
    fn coalesce_sibling_skills_preserves_unique_act_ids() {
        let out = coalesce_sibling_skills(vec![
            synthetic_skill_wrapper(30630122, synthetic_effect(401, 1)),
            synthetic_skill_wrapper(30630161, synthetic_effect(402, 2)),
            synthetic_skill_wrapper(30630122, synthetic_effect(403, 3)),
        ]);

        assert_eq!(out.len(), 2);
        let first = out[0]
            .fight_step
            .as_ref()
            .expect("first output should carry merged wrapper");
        let second = out[1]
            .fight_step
            .as_ref()
            .expect("second output should carry unique wrapper");
        assert_eq!(first.act_id, Some(30630122));
        assert_eq!(first.act_effect.len(), 2);
        assert_eq!(second.act_id, Some(30630161));
        assert_eq!(second.act_effect, vec![synthetic_effect(402, 2)]);
    }

    #[test]
    fn coalesce_sibling_skills_passes_through_non_skill_effects() {
        let direct_damage = synthetic_effect(sonettobuf::effect_type_enum::EffectType::Damage as i32, 15);
        let direct_heal = synthetic_effect(sonettobuf::effect_type_enum::EffectType::Heal as i32, 7);
        let skill = synthetic_skill_wrapper(30630161, synthetic_effect(501, 5));
        let input = vec![direct_damage.clone(), skill.clone(), direct_heal.clone()];
        let out = coalesce_sibling_skills(input.clone());

        assert_eq!(out, input);
    }

    #[test]
    fn coalesce_sibling_skills_keeps_non_contiguous_relative_order() {
        let child_a = synthetic_effect(601, 6);
        let child_b = synthetic_effect(602, 7);
        let damage = synthetic_effect(sonettobuf::effect_type_enum::EffectType::Damage as i32, 20);
        let heal = synthetic_effect(sonettobuf::effect_type_enum::EffectType::Heal as i32, 10);
        let out = coalesce_sibling_skills(vec![
            synthetic_skill_wrapper(30630122, child_a.clone()),
            damage.clone(),
            synthetic_skill_wrapper(30630122, child_b.clone()),
            heal.clone(),
        ]);

        assert_eq!(out.len(), 3);
        let merged = out[0]
            .fight_step
            .as_ref()
            .expect("first output should carry merged skill wrapper");
        assert_eq!(merged.act_id, Some(30630122));
        assert_eq!(merged.act_effect, vec![child_a, child_b]);
        assert_eq!(out[1], damage);
        assert_eq!(out[2], heal);
    }

    #[test]
    fn serialized_act_effect_passes_through_drain() {
        let effect = synthetic_effect(999, 7);
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(
            vec![BattleEvent::SerializedActEffect {
                effect: effect.clone(),
            }],
            &mut ctx,
        );

        assert_eq!(out, vec![effect]);
    }

    #[test]
    fn ex_point_change_updates_manager_and_serializes() {
        let mut ctx = test_ctx();
        let uid = 77;
        ctx.ex_point_mgr.set_ex_point(uid, 2);

        let out = drain_to_fight_steps(
            vec![BattleEvent::ExPointChange {
                target: uid,
                delta: 3,
            }],
            &mut ctx,
        );

        assert_eq!(ctx.ex_point_mgr.get_ex_point(uid), 5);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Expointchange as i32)
        );
        assert_eq!(out[0].effect_num, Some(3));
        assert_eq!(out[0].target_id, Some(uid));
    }

    #[test]
    fn heal_updates_state_and_serializes() {
        let mut ctx = test_ctx();
        let uid = 303_i64;
        ctx.fight.attacker = Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(uid),
                current_hp: Some(30),
                attr: Some(HeroAttribute {
                    hp: Some(100),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        ctx.ex_point_mgr.set_hp(uid, 30);

        let out = drain_to_fight_steps(
            vec![BattleEvent::Heal {
                target: uid,
                amount: 80,
                from: 404,
            }],
            &mut ctx,
        );

        let hp = ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|team| team.entitys.first())
            .and_then(|entity| entity.current_hp)
            .expect("healed entity should remain in fight");
        assert_eq!(hp, 100);
        assert_eq!(ctx.ex_point_mgr.get_hp(uid), 100);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Heal as i32)
        );
        assert_eq!(out[0].target_id, Some(uid));
        assert_eq!(out[0].effect_num, Some(80));
    }

    #[test]
    fn damage_updates_state_and_serializes() {
        let mut ctx = test_ctx();
        let uid = 304_i64;
        ctx.fight.attacker = Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(uid),
                current_hp: Some(90),
                attr: Some(HeroAttribute {
                    hp: Some(100),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        ctx.ex_point_mgr.set_hp(uid, 90);

        let hurt = sonettobuf::FightHurtInfo {
            config_effect: Some(30006),
            ..Default::default()
        };
        let out = drain_to_fight_steps(
            vec![BattleEvent::Damage {
                target: uid,
                amount: 35,
                is_crit: false,
                hurt_info: hurt,
                from: 404,
                skill_id: Some(505),
            }],
            &mut ctx,
        );

        let hp = ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|team| team.entitys.first())
            .and_then(|entity| entity.current_hp)
            .expect("damaged entity should remain in fight");
        assert_eq!(hp, 55);
        assert_eq!(ctx.ex_point_mgr.get_hp(uid), 55);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Damage as i32)
        );
        assert_eq!(out[0].target_id, Some(uid));
        assert_eq!(out[0].effect_num, Some(35));
        assert_eq!(out[0].config_effect, Some(30006));
        let emitted_hurt = out[0]
            .hurt_info
            .as_ref()
            .expect("hurt payload should be present");
        assert_eq!(emitted_hurt.damage, Some(35));
        assert_eq!(emitted_hurt.from_uid, Some(404));
        assert_eq!(emitted_hurt.skill_id, Some(505));
        assert_eq!(
            emitted_hurt.hurt_effect,
            Some(sonettobuf::effect_type_enum::EffectType::Damage as i32)
        );
    }

    #[test]
    fn crit_damage_serializes_with_crit_effect_type() {
        let mut ctx = test_ctx();
        let uid = 305_i64;
        ctx.fight.attacker = Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(uid),
                current_hp: Some(90),
                attr: Some(HeroAttribute {
                    hp: Some(100),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        ctx.ex_point_mgr.set_hp(uid, 90);

        let out = drain_to_fight_steps(
            vec![BattleEvent::Damage {
                target: uid,
                amount: 10,
                is_crit: true,
                hurt_info: sonettobuf::FightHurtInfo::default(),
                from: 404,
                skill_id: Some(505),
            }],
            &mut ctx,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Crit as i32)
        );
        assert_eq!(
            out[0]
                .hurt_info
                .as_ref()
                .and_then(|hurt| hurt.hurt_effect),
            Some(sonettobuf::effect_type_enum::EffectType::Crit as i32)
        );
    }

    #[test]
    fn heal_crit_updates_state_and_serializes() {
        let mut ctx = test_ctx();
        let uid = 306_i64;
        ctx.fight.attacker = Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(uid),
                current_hp: Some(30),
                attr: Some(HeroAttribute {
                    hp: Some(100),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        ctx.ex_point_mgr.set_hp(uid, 30);

        let out = drain_to_fight_steps(
            vec![BattleEvent::HealCrit {
                target: uid,
                amount: 80,
                from: 404,
            }],
            &mut ctx,
        );

        let hp = ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|team| team.entitys.first())
            .and_then(|entity| entity.current_hp)
            .expect("healed entity should remain in fight");
        assert_eq!(hp, 100);
        assert_eq!(ctx.ex_point_mgr.get_hp(uid), 100);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Healcrit as i32)
        );
        assert_eq!(out[0].target_id, Some(uid));
        assert_eq!(out[0].effect_num, Some(80));
    }

    #[test]
    fn event_triggered_skill_emit_serializes_with_children() {
        let child = synthetic_effect(321, 11);
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id: 30630122,
            from: 1001,
            to: 2002,
            children: vec![BattleEvent::SerializedActEffect {
                effect: child.clone(),
            }],
            kind: SkillEmitKind::EventTriggered,
        });
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(queue.drain(), &mut ctx);

        assert_eq!(
            out,
            vec![wrap_step(make_skill_step(
                1001,
                2002,
                30630122,
                0,
                vec![child]
            ))]
        );
    }

    #[test]
    fn player_initiated_skill_emit_serializes_single_wrap_with_recursive_children() {
        let direct = synthetic_effect(123, 9);
        let reactive_direct = synthetic_effect(321, 11);
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id: 31140151,
            from: 1001,
            to: 2002,
            children: vec![
                BattleEvent::SerializedActEffect {
                    effect: direct.clone(),
                },
                BattleEvent::SkillEmit {
                    skill_id: 30630122,
                    from: 7777,
                    to: 8888,
                    children: vec![BattleEvent::SerializedActEffect {
                        effect: reactive_direct.clone(),
                    }],
                    kind: SkillEmitKind::EventTriggered,
                },
            ],
            kind: SkillEmitKind::PlayerInitiated,
        });
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(queue.drain(), &mut ctx);

        assert_eq!(out.len(), 1);
        let root = out[0]
            .fight_step
            .as_ref()
            .expect("top-level act effect should carry fight_step");
        assert_eq!(root.act_type, Some(sonettobuf::fight_step::ActType::Skill as i32));
        assert_eq!(root.act_id, Some(31140151));
        assert_eq!(root.from_id, Some(1001));
        assert_eq!(root.to_id, Some(2002));
        assert_eq!(root.act_effect.len(), 2);
        assert_eq!(root.act_effect[0], direct);

        let child = root.act_effect[1]
            .fight_step
            .as_ref()
            .expect("reactive child should be single-wrap fight_step");
        assert_eq!(child.act_type, Some(sonettobuf::fight_step::ActType::Skill as i32));
        assert_eq!(child.act_id, Some(30630122));
        assert_eq!(child.from_id, Some(7777));
        assert_eq!(child.to_id, Some(8888));
        assert_eq!(child.act_effect, vec![reactive_direct]);
    }

    #[test]
    fn automatic_phase_skill_emit_serializes_with_double_wrap() {
        let child = synthetic_effect(654, 32);
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id: 530000411,
            from: -1,
            to: -2,
            children: vec![BattleEvent::SerializedActEffect {
                effect: child.clone(),
            }],
            kind: SkillEmitKind::AutomaticPhase,
        });
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(queue.drain(), &mut ctx);

        let expected_skill = make_skill_step(-1, -2, 530000411, 0, vec![child]);
        let expected = vec![wrap_step(effect_container_step(
            0,
            0,
            0,
            vec![wrap_step(effect_container_step(
                0,
                0,
                0,
                vec![wrap_step(expected_skill.clone())],
            ))],
        ))];
        assert_eq!(out, expected);

        let outer = out[0]
            .fight_step
            .as_ref()
            .expect("outer act effect should carry fight_step");
        assert_eq!(outer.act_type, Some(sonettobuf::fight_step::ActType::Effect as i32));
        assert_eq!(outer.act_id, Some(0));
        assert_eq!(outer.act_effect.len(), 1);

        let inner_effect = outer.act_effect[0]
            .fight_step
            .as_ref()
            .expect("outer 162 should carry inner effect container");
        assert_eq!(
            inner_effect.act_type,
            Some(sonettobuf::fight_step::ActType::Effect as i32)
        );
        assert_eq!(inner_effect.act_id, Some(0));
        assert_eq!(inner_effect.act_effect.len(), 1);

        let skill = inner_effect.act_effect[0]
            .fight_step
            .as_ref()
            .expect("inner 162 should carry skill step");
        assert_eq!(skill.act_type, Some(sonettobuf::fight_step::ActType::Skill as i32));
        assert_eq!(skill.act_id, Some(530000411));
        assert_eq!(skill.from_id, Some(-1));
        assert_eq!(skill.to_id, Some(-2));
        assert_eq!(skill.act_effect, vec![synthetic_effect(654, 32)]);
    }

    #[test]
    fn equipment_embedded_skill_emit_serializes_single_wrap_with_recursive_children() {
        let direct = synthetic_effect(456, 21);
        let reactive_direct = synthetic_effect(654, 32);
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id: 31200145,
            from: 1111,
            to: 2222,
            children: vec![
                BattleEvent::SerializedActEffect {
                    effect: direct.clone(),
                },
                BattleEvent::SkillEmit {
                    skill_id: 30630122,
                    from: 3333,
                    to: 4444,
                    children: vec![BattleEvent::SerializedActEffect {
                        effect: reactive_direct.clone(),
                    }],
                    kind: SkillEmitKind::EventTriggered,
                },
            ],
            kind: SkillEmitKind::EquipmentEmbedded,
        });
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(queue.drain(), &mut ctx);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].effect_type, Some(162));
        let root = out[0]
            .fight_step
            .as_ref()
            .expect("embedded top-level act effect should carry fight_step");
        assert_eq!(root.act_type, Some(sonettobuf::fight_step::ActType::Skill as i32));
        assert_eq!(root.act_id, Some(31200145));
        assert_eq!(root.from_id, Some(1111));
        assert_eq!(root.to_id, Some(2222));
        assert_eq!(root.act_effect.len(), 2);
        assert_eq!(root.act_effect[0], direct);

        let child = root.act_effect[1]
            .fight_step
            .as_ref()
            .expect("embedded reactive child should be single-wrap fight_step");
        assert_eq!(child.act_type, Some(sonettobuf::fight_step::ActType::Skill as i32));
        assert_eq!(child.act_id, Some(30630122));
        assert_eq!(child.from_id, Some(3333));
        assert_eq!(child.to_id, Some(4444));
        assert_eq!(child.act_effect, vec![reactive_direct]);
    }

    #[test]
    fn bloodpool_value_change_updates_state_and_serializes() {
        let mut ctx = test_ctx();
        let team_type = 1;
        let uid = 66;
        ctx.bloodtithe.set_value(team_type, 7);

        let out = drain_to_fight_steps(
            vec![BattleEvent::BloodpoolValueChange {
                team_type,
                target: uid,
                delta: -3,
            }],
            &mut ctx,
        );

        assert_eq!(ctx.bloodtithe.get_value(team_type), 4);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Bloodpoolvaluechange as i32)
        );
        assert_eq!(out[0].target_id, Some(uid));
        assert_eq!(out[0].effect_num, Some(team_type));
        assert_eq!(out[0].effect_num1, Some(-3));
    }

    #[test]
    fn bloodpool_max_change_updates_state_and_serializes() {
        let mut ctx = test_ctx();
        let team_type = 1;
        ctx.bloodtithe.set_max(team_type, 24);

        let out = drain_to_fight_steps(
            vec![BattleEvent::BloodpoolMaxChange {
                team_type,
                max: 57,
            }],
            &mut ctx,
        );

        assert_eq!(ctx.bloodtithe.get_max(team_type), 57);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Bloodpoolmaxchange as i32)
        );
        assert_eq!(out[0].target_id, Some(0));
        assert_eq!(out[0].effect_num, Some(team_type));
        assert_eq!(out[0].effect_num1, Some(57));
    }

    #[test]
    fn buff_remove_updates_manager_and_serializes() {
        ensure_game_data_initialized();

        let mut ctx = test_ctx();
        let target = 88_i64;
        let buff_uid = 1_000_123_i64;
        let buff_id = 30091122_i32;
        let from_uid = 77_i64;
        ctx.buff_mgr
            .add_with_uid(target, buff_id, from_uid, 1, 0, buff_uid);

        let out = drain_to_fight_steps(
            vec![BattleEvent::BuffRemove {
                target,
                buff_uid,
            }],
            &mut ctx,
        );

        assert!(
            !ctx.buff_mgr
                .get(target)
                .iter()
                .any(|instance| instance.uid == buff_uid)
        );
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Buffdel as i32)
        );
        assert_eq!(out[0].target_id, Some(target));
        let emitted = out[0].buff.as_ref().expect("buff metadata should be present");
        assert_eq!(emitted.uid, Some(buff_uid));
        assert_eq!(emitted.buff_id, Some(buff_id));
        assert_eq!(emitted.from_uid, Some(from_uid));
    }

    #[test]
    fn buff_apply_updates_manager_and_serializes() {
        ensure_game_data_initialized();

        let mut ctx = test_ctx();
        let target = 101_i64;
        let buff_id = 30091122_i32;
        let from_uid = 77_i64;
        let count = 0_i32;
        let layer = 1_i32;

        let out = drain_to_fight_steps(
            vec![BattleEvent::BuffApply {
                target,
                buff_id,
                count,
                layer,
                from: from_uid,
                config_effect: Some(30003),
            }],
            &mut ctx,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Buffadd as i32)
        );
        assert_eq!(out[0].target_id, Some(target));
        assert_eq!(out[0].effect_num, Some(buff_id));
        assert_eq!(out[0].config_effect, Some(30003));
        let emitted = out[0].buff.as_ref().expect("buff payload should be present");
        let emitted_uid = emitted.uid.expect("buff uid should be present");
        assert_eq!(emitted.buff_id, Some(buff_id));
        assert_eq!(emitted.from_uid, Some(from_uid));
        assert_eq!(emitted.count, Some(count));
        assert_eq!(emitted.layer, Some(layer));

        let applied = ctx
            .buff_mgr
            .get(target)
            .iter()
            .find(|instance| instance.uid == emitted_uid)
            .expect("buff should exist in manager after apply");
        let expected_stacks = config::configs::get()
            .skill_buff
            .iter()
            .find(|buff| buff.id == buff_id)
            .map(|buff| if count > 0 { count } else { buff.effect_count })
            .unwrap_or(count);
        assert_eq!(applied.buff_id, buff_id);
        assert_eq!(applied.from_uid, from_uid);
        assert_eq!(applied.layer, layer);
        assert_eq!(applied.stacks, expected_stacks);
    }

    #[test]
    fn buff_update_updates_manager_and_serializes() {
        ensure_game_data_initialized();

        let mut ctx = test_ctx();
        let target = 99_i64;
        let buff_uid = 1_000_321_i64;
        let buff_id = 30091122_i32;
        let from_uid = 55_i64;
        ctx.buff_mgr
            .add_with_uid(target, buff_id, from_uid, 4, 3, buff_uid);

        let out = drain_to_fight_steps(
            vec![BattleEvent::BuffUpdate {
                target,
                buff_uid,
                new_count: 2,
                new_layer: 1,
            }],
            &mut ctx,
        );

        let updated = ctx
            .buff_mgr
            .get(target)
            .iter()
            .find(|instance| instance.uid == buff_uid)
            .expect("buff should still exist after update");
        assert_eq!(updated.stacks, 2);
        assert_eq!(updated.layer, 1);

        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].effect_type,
            Some(sonettobuf::effect_type_enum::EffectType::Buffupdate as i32)
        );
        assert_eq!(out[0].target_id, Some(target));
        let emitted = out[0].buff.as_ref().expect("buff payload should be present");
        assert_eq!(emitted.uid, Some(buff_uid));
        assert_eq!(emitted.buff_id, Some(buff_id));
        assert_eq!(emitted.from_uid, Some(from_uid));
        assert_eq!(emitted.count, Some(2));
        assert_eq!(emitted.layer, Some(1));
    }

    #[test]
    fn fight_step_to_event_round_trip_preserves_skill_step() {
        let original = make_skill_step(
            3003,
            4004,
            5005,
            0,
            vec![synthetic_effect(11, 1), synthetic_effect(12, 2)],
        );
        let mut ctx = test_ctx();

        let out = drain_to_fight_steps(vec![fight_step_to_event(original.clone())], &mut ctx);

        assert_eq!(out, vec![wrap_step(original)]);
    }
}
