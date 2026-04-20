use super::super::types::ex_point::ExPointType;
use super::traits::Manager;
use sonettobuf::{Fight, FightExPointInfo};
use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct ExPointMgr {
    ex_points: HashMap<i64, i32>,
    ex_max: HashMap<i64, i32>,
    pub current_hp: HashMap<i64, i32>,
    pub max_hp: HashMap<i64, i32>,
    recent_decr_ex_point: HashMap<i64, i32>,
}

impl ExPointMgr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        let iter = fight
            .attacker
            .iter()
            .chain(fight.defender.iter())
            .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()));

        for e in iter {
            let Some(uid) = e.uid else { continue };

            self.ex_points.insert(uid, e.ex_point.unwrap_or(0));

            self.ex_max
                .insert(uid, if e.ex_point_type == Some(1) { 8 } else { 5 });

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

impl Manager for ExPointMgr {
    fn on_round_end(&mut self) {
        // ex_point persists across rounds, hp resynced from fight
        self.recent_decr_ex_point.clear();
    }

    fn on_battle_end(&mut self) {
        self.ex_points.clear();
        self.current_hp.clear();
        self.recent_decr_ex_point.clear();
    }
}

pub fn build_ex_point_info(fight: &Fight, mgr: &ExPointMgr) -> Vec<FightExPointInfo> {
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

pub fn sync_to_fight(fight: &mut Fight, mgr: &ExPointMgr) {
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

pub fn sync_from_fight(fight: &Fight, mgr: &mut ExPointMgr) {
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
