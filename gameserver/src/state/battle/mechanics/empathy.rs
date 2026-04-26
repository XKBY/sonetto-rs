use std::collections::HashMap;

use sonettobuf::{ActEffect, BuffInfo, Fight};

use crate::state::battle::{
    manager::buff_mgr::BuffMgr,
    types::{buff::BuffLayerType, effects::EffectType},
};

pub const EMPATHY_BUFF_ID: i32 = 30800141;
const EMPATHY_ACT_ID: i32 = 770;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EmpathyState {
    values: HashMap<i64, i32>,
}

impl EmpathyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        self.values.clear();

        let mut seed_side = |entitys: &[sonettobuf::FightEntityInfo]| {
            for entity in entitys {
                let Some(uid) = entity.uid else { continue };
                let value = entity
                    .buffs
                    .iter()
                    .chain(entity.no_effect_buffs.iter())
                    .find(|buff| buff.buff_id == Some(EMPATHY_BUFF_ID))
                    .and_then(|buff| buff.act_common_params.as_deref())
                    .and_then(parse_empathy_value)
                    .unwrap_or(0);
                if value > 0 {
                    self.values.insert(uid, value);
                }
            }
        };

        if let Some(attacker) = &fight.attacker {
            seed_side(&attacker.entitys);
            seed_side(&attacker.sub_entitys);
        }
        if let Some(defender) = &fight.defender {
            seed_side(&defender.entitys);
            seed_side(&defender.sub_entitys);
        }
    }

    pub fn compute_storage_amount(damage: i32) -> i32 {
        damage.max(0) / 10
    }

    pub fn storage_cap(max_hp: i32) -> i32 {
        max_hp.max(0).saturating_mul(2) / 10
    }

    pub fn current(&self, uid: i64) -> i32 {
        self.values.get(&uid).copied().unwrap_or(0)
    }

    pub fn apply_storage(
        &mut self,
        buff_mgr: &mut BuffMgr,
        kakania_uid: i64,
        amount: i32,
        caster_max_hp: i32,
    ) -> i32 {
        let cap = Self::storage_cap(caster_max_hp);
        let current = self.current(kakania_uid);
        let next = current.saturating_add(amount.max(0)).min(cap);
        self.values.insert(kakania_uid, next);

        let buff_uid = ensure_empathy_buff(buff_mgr, kakania_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            kakania_uid,
            buff_uid,
            &build_empathy_params(next, cap),
        );

        next
    }

    pub fn emit_storage_injury(
        &self,
        target_uid: i64,
        amount: i32,
        buff_uid: i64,
        from_uid: i64,
        cap: i32,
    ) -> ActEffect {
        ActEffect {
            effect_type: Some(EffectType::StorageInjury as i32),
            target_id: Some(target_uid),
            effect_num: Some(amount.max(0)),
            buff: Some(BuffInfo {
                buff_id: Some(EMPATHY_BUFF_ID),
                duration: Some(0),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(0),
                act_common_params: Some(build_empathy_params(amount.max(0), cap)),
                layer: Some(0),
                r#type: Some(BuffLayerType::Normal as i32),
                act_info: vec![],
            }),
            ..Default::default()
        }
    }

    pub fn emit_buff_update(
        &self,
        target_uid: i64,
        amount: i32,
        buff_uid: i64,
        from_uid: i64,
        cap: i32,
    ) -> ActEffect {
        ActEffect {
            effect_type: Some(EffectType::BuffUpdate as i32),
            target_id: Some(target_uid),
            effect_num: Some(0),
            buff: Some(BuffInfo {
                buff_id: Some(EMPATHY_BUFF_ID),
                duration: Some(0),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(0),
                act_common_params: Some(build_empathy_params(amount.max(0), cap)),
                layer: Some(0),
                r#type: Some(BuffLayerType::Normal as i32),
                act_info: vec![],
            }),
            ..Default::default()
        }
    }
}

fn ensure_empathy_buff(buff_mgr: &mut BuffMgr, target_uid: i64) -> i64 {
    if let Some(existing) = buff_mgr.find_instance_by_buff_id(target_uid, EMPATHY_BUFF_ID) {
        return existing.uid;
    }

    buff_mgr.add(target_uid, EMPATHY_BUFF_ID, target_uid, 0, 0);
    buff_mgr
        .find_instance_by_buff_id(target_uid, EMPATHY_BUFF_ID)
        .map(|buff| buff.uid)
        .unwrap_or(0)
}

fn parse_empathy_value(params: &str) -> Option<i32> {
    let mut parts = params.split('#');
    let act_id = parts.next()?.trim().parse::<i32>().ok()?;
    if act_id != EMPATHY_ACT_ID {
        return None;
    }
    parts.next()?.trim().parse::<i32>().ok()
}

fn build_empathy_params(current: i32, cap: i32) -> String {
    format!("{}#{}#{}", EMPATHY_ACT_ID, current.max(0), cap.max(0))
}
