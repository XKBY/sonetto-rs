use std::collections::HashMap;

use sonettobuf::{ActEffect, BuffInfo, Fight};

use crate::state::battle::{
    manager::buff_mgr::BuffMgr,
    types::{buff::BuffLayerType, effects::EffectType},
};

/// Canonical Kakania Empathy bufftype id. Used for `BuffMgr` lookups so
/// the engine matches Kakania's portrait/rank variants too — buffs
/// 30800141 / 30800142 / 30800143 all share `typeId 30800141` per
/// `data/excel2json/skill_bufftype.json`. The variants only differ in
/// scaling params (storage cap, secondary buff id) but represent the
/// same Empathy mechanic, so type-id matching is more durable than
/// buff-id matching when destiny/portrait swaps are active.
pub const EMPATHY_TYPE_ID: i32 = 30800141;
/// Default buff id used when Kakania first acquires Empathy (before any
/// destiny/portrait variant is active). Insight I's battle-start passive
/// 30800141 applies this canonical id.
pub const EMPATHY_DEFAULT_BUFF_ID: i32 = 30800141;
const EMPATHY_ACT_ID: i32 = 770;

/// Returns `true` if the given `buff_id` belongs to the Empathy bufftype
/// family (any of 30800141 / 30800142 / 30800143 or future portrait
/// variants), via config lookup.
fn is_empathy_buff(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| b.type_id == EMPATHY_TYPE_ID)
        .unwrap_or(false)
}

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
                    .find(|buff| buff.buff_id.map(is_empathy_buff).unwrap_or(false))
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

    /// Refresh in-memory cumulative totals from the live BuffMgr.
    ///
    /// `init(&Fight)` reads from `entity.buffs`, but
    /// `set_instance_act_common_params` only writes to `BuffMgr`'s
    /// internal state — the per-entity `BuffInfo` snapshot does NOT
    /// receive the storage update. When the round simulator advances
    /// to the next round and calls `Mechanics::init` again, reading
    /// from `entity.buffs` resets the value to whatever was on the
    /// pre-round snapshot. Reading from `BuffMgr` instead picks up
    /// the live state from the round-being-simulated. Call this AFTER
    /// the standard `init(fight)` so it overrides the stale values
    /// with whatever the runtime BuffMgr knows.
    pub fn sync_from_buff_mgr(&mut self, buff_mgr: &BuffMgr) {
        let mut seen = std::collections::HashSet::new();
        for (uid, instance) in buff_mgr.all_instances() {
            if !is_empathy_buff(instance.buff_id) {
                continue;
            }
            seen.insert(uid);
            let value = parse_empathy_value(&instance.act_common_params).unwrap_or(0);
            if value > 0 {
                self.values.insert(uid, value);
            } else {
                self.values.remove(&uid);
            }
        }
        self.values.retain(|uid, _| seen.contains(uid));
    }

    /// Insight I rule: "10% of that damage is stored as Empathy".
    /// TODO: portrait/destiny variants may scale this — buff 30800143's
    /// features `770#101#300#30800162#20#100#150` suggest different
    /// rate/cap params. Parse those from the active variant's features
    /// when destiny/portrait support lands.
    pub fn compute_storage_amount(damage: i32) -> i32 {
        damage.max(0) / 10
    }

    /// Insight I rule: "can store up to 20% of Kakania's Max HP".
    /// TODO: scale via active variant's features (see above) once
    /// destiny/portrait support is wired.
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

        let (_buff_id, buff_uid) = ensure_empathy_buff(buff_mgr, kakania_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            kakania_uid,
            buff_uid,
            &build_empathy_params(next, cap),
        );

        next
    }

    pub fn sync_buff_state(
        &mut self,
        buff_mgr: &mut BuffMgr,
        target_uid: i64,
        current_total: i32,
        target_max_hp: i32,
    ) {
        let cap = Self::storage_cap(target_max_hp);
        let current_total = current_total.max(0).min(cap);
        if current_total > 0 {
            self.values.insert(target_uid, current_total);
        } else {
            self.values.remove(&target_uid);
        }

        let (_buff_id, buff_uid) = ensure_empathy_buff(buff_mgr, target_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            target_uid,
            buff_uid,
            &build_empathy_params(current_total, cap),
        );
    }

    /// When an Empathy holder takes incoming skill damage, emit the
    /// cumulative `StorageInjury` marker before the damage packet and
    /// update the preview buff state so later behavior slots see the
    /// stored total immediately.
    pub fn inject_storage_injury_for_damage_emissions(
        &mut self,
        buff_mgr: &mut BuffMgr,
        source_uid: i64,
        target_uid: i64,
        target_max_hp: i32,
        effects: Vec<ActEffect>,
    ) -> Vec<ActEffect> {
        if source_uid == target_uid
            || target_uid == 0
            || target_max_hp <= 0
            || !has_empathy_buff(buff_mgr, target_uid)
        {
            return effects;
        }

        let cap = Self::storage_cap(target_max_hp);
        let mut out = Vec::with_capacity(effects.len());
        for effect in effects {
            let should_inject = effect.target_id == Some(target_uid)
                && is_incoming_damage_effect_type(effect.effect_type);
            if !should_inject {
                out.push(effect);
                continue;
            }

            let damage = effect.effect_num.unwrap_or(0).max(0);
            let storage = Self::compute_storage_amount(damage);
            let current_total = self.apply_storage(buff_mgr, target_uid, storage, target_max_hp);
            let (buff_id, buff_uid) = ensure_empathy_buff(buff_mgr, target_uid);
            out.push(self.emit_storage_injury(
                target_uid,
                current_total,
                buff_id,
                buff_uid,
                target_uid,
                cap,
            ));
            out.push(effect);
        }

        out
    }

    pub fn emit_storage_injury(
        &self,
        target_uid: i64,
        amount: i32,
        buff_id: i32,
        buff_uid: i64,
        from_uid: i64,
        cap: i32,
    ) -> ActEffect {
        ActEffect {
            effect_type: Some(EffectType::StorageInjury as i32),
            target_id: Some(target_uid),
            effect_num: Some(amount.max(0)),
            buff: Some(BuffInfo {
                buff_id: Some(buff_id),
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
        buff_id: i32,
        buff_uid: i64,
        from_uid: i64,
        cap: i32,
    ) -> ActEffect {
        ActEffect {
            effect_type: Some(EffectType::BuffUpdate as i32),
            target_id: Some(target_uid),
            effect_num: Some(0),
            buff: Some(BuffInfo {
                buff_id: Some(buff_id),
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

pub fn has_empathy_buff(buff_mgr: &BuffMgr, target_uid: i64) -> bool {
    buff_mgr
        .find_instance_by_type_id(target_uid, EMPATHY_TYPE_ID)
        .is_some()
}

fn is_incoming_damage_effect_type(effect_type: Option<i32>) -> bool {
    matches!(
        effect_type,
        Some(t)
            if t == EffectType::Damage as i32
                || t == EffectType::Crit as i32
                || t == EffectType::DamageExtra as i32
                || t == EffectType::OriginDamage as i32
                || t == EffectType::OriginCrit as i32
                || t == EffectType::AdditionalDamage as i32
                || t == EffectType::AdditionalDamageCrit as i32
                || t == EffectType::FixedDamage as i32
                || t == EffectType::DamageFromLostHp as i32
                || t == EffectType::EnchantBurnDamage as i32
                || t == EffectType::EnchantDepresseDamage as i32
                || t == EffectType::DeadlyPoisonOriginDamage as i32
                || t == EffectType::DeadlyPoisonOriginCrit as i32
    )
}

/// Returns the active Empathy `(buff_id, buff_uid)` for `target_uid`,
/// matching by `EMPATHY_TYPE_ID` so portrait/rank variants
/// (30800142/30800143) are handled. If no instance exists yet,
/// creates one using `EMPATHY_DEFAULT_BUFF_ID` (canonical 30800141).
fn ensure_empathy_buff(buff_mgr: &mut BuffMgr, target_uid: i64) -> (i32, i64) {
    if let Some(existing) = buff_mgr.find_instance_by_type_id(target_uid, EMPATHY_TYPE_ID) {
        return (existing.buff_id, existing.uid);
    }

    buff_mgr.add(target_uid, EMPATHY_DEFAULT_BUFF_ID, target_uid, 0, 0);
    buff_mgr
        .find_instance_by_type_id(target_uid, EMPATHY_TYPE_ID)
        .map(|buff| (buff.buff_id, buff.uid))
        .unwrap_or((EMPATHY_DEFAULT_BUFF_ID, 0))
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
