use sonettobuf::{Fight, FightEntityInfo, FightExPointInfo, FightStep};
use std::collections::{HashMap, HashSet};

use super::super::{
    fight_step::{ActEffectBuilder, FightStepBuilder},
    types::ex_point::ExPointType,
};
use super::traits::Manager;

#[derive(Debug, Clone, Copy)]
pub struct EntityLocation {
    pub is_attacker: bool,
    pub index: usize,
}

pub type FightEntityDataMgr = EntityMgr;

#[derive(Default, Debug, Clone)]
pub struct EntityMgr {
    entity_cache: HashMap<i64, EntityLocation>,
    ex_points: HashMap<i64, i32>,
    ex_max: HashMap<i64, i32>,
    ex_point_required: HashMap<i64, i32>,
    pub current_hp: HashMap<i64, i32>,
    pub max_hp: HashMap<i64, i32>,
    recent_decr_ex_point: HashMap<i64, i32>,
}

impl EntityMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut mgr = Self::default();
        mgr.rebuild_cache(fight);
        mgr.init(fight);
        mgr
    }

    pub fn rebuild_cache(&mut self, fight: &Fight) {
        self.entity_cache.clear();
        if let Some(attacker) = &fight.attacker {
            for (idx, entity) in attacker.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: true, index: idx });
                }
            }
            /* 
            When a hero die, sub_entity will move to entity list
            So we may only cache the main entity list
            for (idx, entity) in attacker.sub_entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: true, index: idx });
                }
            }
            */
        }
        if let Some(defender) = &fight.defender {
            for (idx, entity) in defender.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: false, index: idx });
                }
            }
        }
    }

    pub fn get_location(&self, entity_id: i64) -> Option<EntityLocation> {
        self.entity_cache.get(&entity_id).copied()
    }

    pub fn alive_hero_uids(&self) -> HashSet<i64> {
        self.entity_cache
            .iter()
            .filter(|(uid, loc)| loc.is_attacker && self.current_hp.get(uid).copied().unwrap_or(0) > 0)
            .map(|(uid, _)| *uid)
            .collect()
    }

    pub fn alive_enemy_uids(&self) -> HashSet<i64> {
        self.entity_cache
            .iter()
            .filter(|(uid, loc)| !loc.is_attacker && self.current_hp.get(uid).copied().unwrap_or(0) > 0)
            .map(|(uid, _)| *uid)
            .collect()
    }

    /// If any active hero is dead and a sub is available, substitutes the first such hero.
    /// Returns `(dead_uid, new_entity, position)` where position is 1-based.
    pub fn sub_hero(&mut self, fight: &mut Fight) -> Option<(i64, FightEntityInfo, i32)> {
        let attacker = fight.attacker.as_mut()?;
        if attacker.sub_entitys.is_empty() {
            return None;
        }
        let (slot, dead_uid, position) = attacker.entitys.iter().enumerate().find_map(|(i, e)| {
            let uid = e.uid?;
            let pos = e.position.unwrap_or(0);
            if e.current_hp.unwrap_or(0) <= 0 && pos > 0 {
                Some((i, uid, pos))
            } else {
                None
            }
        })?;
        let mut sub = attacker.sub_entitys.remove(0);
        sub.position = Some(position);
        attacker.entitys[slot] = sub.clone();
        self.rebuild_cache(fight);
        Some((dead_uid, sub, position))
    }

    #[allow(dead_code)]
    pub fn get_team_entities<'a>(&self, fight: &'a Fight, team_type: i32) -> Vec<&'a FightEntityInfo> {
        let mut entities = Vec::new();
        if let Some(attacker) = &fight.attacker {
            entities.extend(attacker.entitys.iter().filter(|e| e.team_type == Some(team_type)));
        }
        if let Some(defender) = &fight.defender {
            entities.extend(defender.entitys.iter().filter(|e| e.team_type == Some(team_type)));
        }
        entities
    }

    pub fn init(&mut self, fight: &Fight) {
        let cfg = config::configs::get();
        let iter = fight
            .attacker
            .iter()
            .chain(fight.defender.iter())
            .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()));
        for e in iter {
            let Some(uid) = e.uid else { continue };
            self.ex_points.insert(uid, e.ex_point.unwrap_or(0));
            self.ex_max.insert(uid, if e.ex_point_type == Some(1) { 8 } else { 5 });
            let required = e.model_id
                .and_then(|mid| cfg.monster_skill_template.iter().find(|t| t.id == mid))
                .map(|t| t.unique_skill_point)
                .unwrap_or(0);
            self.ex_point_required.insert(uid, required);
            let hp = e.current_hp.unwrap_or(0);
            self.current_hp.insert(uid, hp);
            let mhp = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(hp);
            self.max_hp.insert(uid, mhp);
        }
    }

    pub fn add_ex_point(&mut self, uid: i64, amount: i32) {
        let v = self.ex_points.entry(uid).or_insert(0);
        *v = (*v + amount).max(0);
    }

    pub fn on_use_card(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn on_move_card(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn on_compose_card(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn set_recent_decr_ex_point(&mut self, uid: i64, amount: i32) {
        self.recent_decr_ex_point.insert(uid, amount.max(0));
    }

    pub fn get_recent_decr_ex_point(&self, uid: i64) -> i32 {
        self.recent_decr_ex_point.get(&uid).copied().unwrap_or(0)
    }

    pub fn clear_recent_decr_ex_point(&mut self, uid: i64) {
        self.recent_decr_ex_point.remove(&uid);
    }

    #[allow(dead_code)]
    pub fn add_ex_max(&mut self, uid: i64, amount: i32) {
        let v = self.ex_max.entry(uid).or_insert(0);
        *v += amount;
    }

    pub fn get_ex_max(&self, uid: i64) -> i32 {
        self.ex_max.get(&uid).copied().unwrap_or(0)
    }

    pub fn get_ex_point_required(&self, uid: i64) -> i32 {
        self.ex_point_required.get(&uid).copied().unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn consume_ex_point(&mut self, uid: i64, amount: i32) -> bool {
        let v = self.ex_points.entry(uid).or_insert(0);
        if *v >= amount {
            *v -= amount;
            true
        } else {
            false
        }
    }

    pub fn set_ex_point(&mut self, uid: i64, value: i32) {
        self.ex_points.insert(uid, value.max(0));
    }

    pub fn get_ex_point(&self, uid: i64) -> i32 {
        self.ex_points.get(&uid).copied().unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn set_hp(&mut self, uid: i64, hp: i32) {
        self.current_hp.insert(uid, hp.max(0));
    }

    pub fn get_hp(&self, uid: i64) -> i32 {
        self.current_hp.get(&uid).copied().unwrap_or(0)
    }

    pub fn set_max_hp(&mut self, uid: i64, hp: i32) {
        self.max_hp.insert(uid, hp.max(0));
    }

    pub fn get_max_hp(&self, uid: i64) -> i32 {
        self.max_hp.get(&uid).copied().unwrap_or(0)
    }

    pub fn apply_damage(&mut self, uid: i64, amount: i32) {
        let hp = self.current_hp.entry(uid).or_insert(0);
        *hp = (*hp - amount).max(0);
    }

    #[allow(dead_code)]
    pub fn apply_heal(&mut self, uid: i64, amount: i32, max_hp: i32) {
        let hp = self.current_hp.entry(uid).or_insert(0);
        *hp = (*hp + amount).min(max_hp);
    }
}

impl Manager for EntityMgr {
    fn on_round_end(&mut self, _fight: &mut Fight) {
        self.recent_decr_ex_point.clear();
    }

    fn on_battle_end(&mut self) {
        self.ex_points.clear();
        self.current_hp.clear();
        self.recent_decr_ex_point.clear();
    }
}

pub fn get_entity_mut_by_location(
    fight: &mut Fight,
    location: EntityLocation,
) -> Option<&mut sonettobuf::FightEntityInfo> {
    if location.is_attacker {
        fight.attacker.as_mut()?.entitys.get_mut(location.index)
    } else {
        fight.defender.as_mut()?.entitys.get_mut(location.index)
    }
}

pub fn build_ex_point_info(fight: &Fight, mgr: &EntityMgr) -> Vec<FightExPointInfo> {
    fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
        .map(|e| {
            let uid = e.uid.unwrap_or(0);
            let hp = mgr.get_hp(uid);
            let ex = mgr.get_ex_point(uid);
            tracing::debug!("build_ex_point_info uid={} hp={} ex={}", uid, hp, ex);
            let ex_point_type = match e.model_id {
                Some(3120) => Some(ExPointType::Belief as i32),
                Some(3123) => Some(ExPointType::Synchronization as i32),
                Some(3124) | Some(3122) => Some(ExPointType::Adrenaline as i32),
                _ => e.ex_point_type.or(Some(ExPointType::Common as i32)),
            };
            FightExPointInfo {
                uid: e.uid,
                ex_point: Some(mgr.get_ex_point(uid)),
                power_infos: e.power_infos.clone(),
                current_hp: Some(hp),
                ex_point_type,
            }
        })
        .collect()
}

pub fn sync_to_fight(fight: &mut Fight, mgr: &EntityMgr) {
    for e in fight
        .attacker
        .iter_mut()
        .chain(fight.defender.iter_mut())
        .flat_map(|t| t.entitys.iter_mut().chain(t.sub_entitys.iter_mut()))
    {
        let Some(uid) = e.uid else { continue };
        e.ex_point = Some(mgr.get_ex_point(uid));
        e.current_hp = Some(mgr.get_hp(uid));
        let max_hp = mgr.get_max_hp(uid);
        if max_hp > 0 {
            if let Some(attr) = e.attr.as_mut() {
                attr.hp = Some(max_hp);
            }
            if let Some(base) = e.base_attr.as_mut() {
                base.hp = Some(max_hp);
            }
        }
    }
}

pub fn sync_from_fight(fight: &Fight, mgr: &mut EntityMgr) {
    for e in fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
    {
        let Some(uid) = e.uid else { continue };
        mgr.set_ex_point(uid, e.ex_point.unwrap_or(0));
        mgr.set_hp(uid, e.current_hp.unwrap_or(0));
        if let Some(max) = e.attr.as_ref().and_then(|a| a.hp) {
            mgr.set_max_hp(uid, max);
        }
    }
}

pub fn seed_ex_point_required_from_fight(fight: &Fight, mgr: &mut EntityMgr) {
    let cfg = config::configs::get();
    for e in fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
    {
        let Some(uid) = e.uid else { continue };
        let required = e.model_id
            .and_then(|mid| cfg.monster_skill_template.iter().find(|t| t.id == mid))
            .map(|t| t.unique_skill_point)
            .unwrap_or(0);
        mgr.ex_point_required.insert(uid, required);
    }
}
