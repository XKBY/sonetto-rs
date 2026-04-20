use super::super::{
    context::FightContext,
    fight_step::FightStepBuilder,
    manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr},
    round::step_shape::build_effect_step,
    utils::find_entity,
};
use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep, effect_type_enum::EffectType};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use crate::state::battle::utils::{buff_get_raspberry_params, moxie_change};

static SEEDED_RASPBERRY_MAX: Lazy<Mutex<HashMap<i32, i32>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static SHADOW_CLOAK_FULL_CAP_GRANTED: Lazy<Mutex<HashSet<(i32, i64)>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ShadowCloakState {
    total_shared: i32,     // total accumulated across all ticks
    last_gain_shared: i32, // gain from this tick
    pub rubuska_entry_max_hp: i32,
    pub rubuska_uid: i64,
    pub raspberry_accum: i32,
    pub raspberry_max: i32,
}

impl ShadowCloakState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        let battle_id = fight.battle_id.unwrap_or(0);
        if let Some(a) = &fight.attacker {
            for e in &a.entitys {
                if e.model_id == Some(3125) {
                    self.rubuska_uid = e.uid.unwrap_or(0);
                    self.rubuska_entry_max_hp = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(0);
                }
            }
        }
        let seeded_max = SEEDED_RASPBERRY_MAX
            .lock()
            .ok()
            .and_then(|m| m.get(&battle_id).copied())
            .unwrap_or(0);
        if seeded_max > 0 {
            self.raspberry_max = seeded_max;
            self.rubuska_entry_max_hp = seeded_max * 1000 / 150;
        } else {
            self.raspberry_max = self.rubuska_entry_max_hp * 150 / 1000;
        }
    }

    pub fn is_active(&self) -> bool {
        self.rubuska_uid != 0
    }

    pub fn add(&mut self, uid: i64, hp_lost: i32) -> i32 {
        tracing::warn!(
            "shadow_cloak add: uid={} rubuska_uid={} hp_lost={}",
            uid,
            self.rubuska_uid,
            hp_lost
        );
        // only Rubuska's own HP loss drives shadow cloak
        if uid != self.rubuska_uid {
            return 0;
        }
        let cap = self.rubuska_entry_max_hp * 150 / 1000;
        let gain = (hp_lost * 700 / 1000).min(cap);
        if gain <= 0 {
            return 0;
        }
        self.last_gain_shared = gain;
        self.total_shared += gain;
        gain
    }

    pub fn reset_tick(&mut self) {
        self.last_gain_shared = 0;
    }

    pub fn build_sync_effects(
        &self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &ExPointMgr,
    ) -> Vec<ActEffect> {
        let gain = self.last_gain_shared;
        tracing::warn!(
            "shadow_cloak sync: gain={} total={}",
            self.last_gain_shared,
            self.total_shared
        );
        if gain == 0 {
            return vec![];
        }
        let total = self.total_shared;
        let max_capacity = self.rubuska_entry_max_hp * 150 / 1000;
        let mut effects = Vec::new();

        let uids: Vec<i64> = fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
            .unwrap_or_default();

        for uid in uids {
            let current_hp = ex_point_mgr.get_hp(uid);
            let base_max_hp = find_entity(fight, uid)
                .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
                .unwrap_or(0);
            let new_max_hp = base_max_hp + total;
            let buff_uid = buff_mgr
                .get(uid)
                .iter()
                .find(|b| b.buff_id == 31250151)
                .map(|b| b.uid)
                .unwrap_or(0);

            effects.push(ActEffect {
                effect_type: Some(EffectType::Currenthpchange as i32),
                target_id: Some(uid),
                effect_num: Some(current_hp),
                ..Default::default()
            });
            effects.push(ActEffect {
                effect_type: Some(EffectType::Buffactinfoupdate as i32),
                target_id: Some(uid),
                reserve_id: Some(buff_uid),
                buff_act_info: Some(sonettobuf::BuffActInfo {
                    act_id: Some(1042),
                    param: vec![gain, max_capacity],
                    ..Default::default()
                }),
                ..Default::default()
            });
            effects.push(ActEffect {
                effect_type: Some(EffectType::Maxhpchange as i32),
                target_id: Some(uid),
                effect_num: Some(new_max_hp),
                buff_act_id: Some(1042), //Raspberry in buff_act
                ..Default::default()
            });
        }

        effects
    }
}

pub(crate) fn build_shadow_cloak_full_cap_step(
    ctx: &FightContext<'_>,
    raspberry_step: &FightStep,
) -> Option<FightStep> {
    let mut rubuska_uid = ctx.mechanics.shadow_cloak.rubuska_uid;
    if rubuska_uid <= 0 {
        rubuska_uid = raspberry_step
            .act_effect
            .iter()
            .filter_map(|e| e.fight_step.as_ref())
            .map(|s| s.from_id.unwrap_or(0))
            .find(|uid| *uid > 0)
            .unwrap_or(0);
    }
    let max_capacity = if ctx.mechanics.shadow_cloak.raspberry_max > 0 {
        ctx.mechanics.shadow_cloak.raspberry_max
    } else {
        find_entity(ctx.fight, rubuska_uid)
            .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
            .unwrap_or(0)
            * 150
            / 1000
    };
    if rubuska_uid <= 0 || max_capacity <= 0 {
        return None;
    }
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    let tracker_key = (battle_id, rubuska_uid);
    if SHADOW_CLOAK_FULL_CAP_GRANTED
        .lock()
        .unwrap()
        .contains(&tracker_key)
    {
        return None;
    }

    let mut targets = HashSet::new();
    let mut total_shadow_gain = 0;

    for effect in &raspberry_step.act_effect {
        let Some(inner) = &effect.fight_step else {
            continue;
        };
        let act_id = inner.act_id.unwrap_or(0);
        if buff_get_raspberry_params(act_id).is_none() {
            continue;
        }
        let target_uid = inner.to_id.unwrap_or(0);
        let Some(target) = find_entity(ctx.fight, target_uid) else {
            continue;
        };
        if target.team_type != Some(1) || target.current_hp.unwrap_or(0) <= 0 {
            continue;
        }

        let damage = inner
            .act_effect
            .iter()
            .filter(|e| e.effect_type == Some(2) || e.effect_type == Some(3))
            .map(|e| e.effect_num.unwrap_or(0).max(0))
            .sum::<i32>();
        if damage <= 0 {
            continue;
        }

        total_shadow_gain += damage * 700 / 1000;
        targets.insert(target_uid);
    }

    if total_shadow_gain < max_capacity || targets.is_empty() {
        return None;
    }
    SHADOW_CLOAK_FULL_CAP_GRANTED
        .lock()
        .unwrap()
        .insert(tracker_key);

    let payout_count = targets.len();
    if payout_count == 0 {
        return None;
    }

    let effects = (0..payout_count).map(|_| moxie_change(rubuska_uid, 1)).collect();
    Some(build_effect_step(effects))
}

pub fn seed_replay_raspberry_max(fight: &Fight, max_capacity: i32) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 || max_capacity <= 0 {
        return;
    }
    if let Ok(mut seeded) = SEEDED_RASPBERRY_MAX.lock() {
        seeded.insert(battle_id, max_capacity);
    }
}

impl ShadowCloakState {
    pub fn sync_step(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &ExPointMgr,
    ) -> Option<FightStep> {
        if !self.is_active() {
            return None;
        }
        let effects = self.build_sync_effects(fight, buff_mgr, ex_point_mgr);
        self.reset_tick();
        if effects.is_empty() {
            return None;
        }
        Some(FightStepBuilder::effect().with_many(effects).build())
    }
}
