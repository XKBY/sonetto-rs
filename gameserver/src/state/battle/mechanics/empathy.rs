use std::collections::HashMap;
use std::sync::OnceLock;

use sonettobuf::{ActEffect, BuffInfo, Fight};

use crate::state::battle::{
    buff_actions::injury_bank::{InjuryBankParams, buff_get_injury_bank_params},
    manager::buff_mgr::BuffMgr,
    types::{buff::BuffLayerType, effects::EffectType},
};

/// Empathy bufftype + canonical default buff id, derived once from data.
///
/// The bufftype family is identified by `skill_buff` rows whose features
/// carry the `InjuryBank` (770) act_type. The canonical default — used
/// for fresh acquisition before any destiny / portrait variant is active
/// — is the lowest-id row in that family. Both values come straight from
/// the data tables, so portrait / destiny variants are picked up
/// automatically as new entries land.
fn canonical_empathy_buff() -> (i32, i32) {
    static CACHE: OnceLock<(i32, i32)> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let cfg = config::configs::get();
        let mut best: Option<(i32, i32)> = None;
        for b in cfg.skill_buff.iter() {
            if buff_get_injury_bank_params(b.id).is_none() {
                continue;
            }
            let candidate = (b.id, b.type_id);
            best = Some(match best {
                None => candidate,
                Some(prev) if candidate.0 < prev.0 => candidate,
                Some(prev) => prev,
            });
        }
        best.unwrap_or((0, 0))
    })
}

pub fn empathy_type_id() -> i32 {
    canonical_empathy_buff().1
}

pub fn empathy_default_buff_id() -> i32 {
    canonical_empathy_buff().0
}

/// `act_id` of the `InjuryBank` buff_act (currently `770`). Looked up
/// once from the buff_act table by act_type name so the value is
/// anchored to the data, not a hand-edited literal.
fn injury_bank_act_id() -> i32 {
    static CACHE: OnceLock<i32> = OnceLock::new();
    *CACHE.get_or_init(|| {
        config::configs::get()
            .buff_act
            .iter()
            .find(|a| a.r#type == "InjuryBank")
            .map(|a| a.id)
            .unwrap_or(0)
    })
}

/// Active Empathy params for `holder_uid`. Walks whichever variant is
/// currently on them via `buff_get_injury_bank_params`, falling back to
/// the canonical default when no instance is present yet.
pub(crate) fn active_injury_bank_params(
    buff_mgr: &BuffMgr,
    holder_uid: i64,
) -> Option<InjuryBankParams> {
    buff_mgr
        .find_instance_by_type_id(holder_uid, empathy_type_id())
        .and_then(|inst| buff_get_injury_bank_params(inst.buff_id))
        .or_else(|| buff_get_injury_bank_params(empathy_default_buff_id()))
}

/// Storage cap for `holder_uid` based on their currently-active Empathy
/// variant. The cap permille (parts[2] of the InjuryBank feature) is
/// `200` for canonical / Insight II buffs and `300` for the Tier-IV
/// `30800143` variant.
fn active_storage_cap(buff_mgr: &BuffMgr, holder_uid: i64, max_hp: i32) -> i32 {
    let permille = active_injury_bank_params(buff_mgr, holder_uid)
        .map(|p| p.storage_cap_permille)
        .unwrap_or(0);
    max_hp.max(0).saturating_mul(permille) / 1000
}

/// Returns `true` if the given `buff_id` belongs to the Empathy bufftype
/// family (any portrait / destiny variant), via config lookup.
fn is_empathy_buff(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| b.type_id == empathy_type_id())
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

    /// Insight I rule: "X% of that damage is stored as Empathy". The
    /// rate currently is `10%` for canonical/Insight II variants
    /// (`30800141` / `30800142`) and ships in `parts[6]` of the
    /// `InjuryBank` payload — Tier-IV (`30800143` -> `parts[6]=150`)
    /// would scale it to 15%. Until `InjuryBankParams` exposes the
    /// `parts[6]` slot, keep the canonical rate inline here.
    pub fn compute_storage_amount(damage: i32) -> i32 {
        damage.max(0) / 10
    }

    /// Insight I rule: "can store up to N% of Kakania's Max HP". `N` is
    /// `storage_cap_permille` in the active variant's `InjuryBank`
    /// feature (parts[2]) — `200` (20%) for canonical / Insight II,
    /// `300` (30%) for the Tier-IV `30800143` variant.
    pub fn storage_cap(buff_mgr: &BuffMgr, holder_uid: i64, max_hp: i32) -> i32 {
        active_storage_cap(buff_mgr, holder_uid, max_hp)
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
        self.apply_storage_with_threshold(buff_mgr, kakania_uid, amount, caster_max_hp)
            .0
    }

    /// Apply a storage gain and return both the new cumulative total
    /// and the number of Insight III storage thresholds crossed.
    pub fn apply_storage_with_threshold(
        &mut self,
        buff_mgr: &mut BuffMgr,
        kakania_uid: i64,
        amount: i32,
        caster_max_hp: i32,
    ) -> (i32, i32) {
        let cap = Self::storage_cap(buff_mgr, kakania_uid, caster_max_hp);
        let current = self.current(kakania_uid).max(0);
        let next = current.saturating_add(amount.max(0)).min(cap);
        self.values.insert(kakania_uid, next);

        let (_buff_id, buff_uid) = ensure_empathy_buff(buff_mgr, kakania_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            kakania_uid,
            buff_uid,
            &build_empathy_params(next, cap),
        );

        let crossed = Self::storage_threshold(buff_mgr, kakania_uid, caster_max_hp)
            .filter(|threshold| *threshold > 0)
            .map(|threshold| (next / threshold - current / threshold).max(0))
            .unwrap_or(0);

        (next, crossed)
    }

    pub fn sync_buff_state(
        &mut self,
        buff_mgr: &mut BuffMgr,
        target_uid: i64,
        current_total: i32,
        target_max_hp: i32,
    ) {
        let cap = Self::storage_cap(buff_mgr, target_uid, target_max_hp);
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

    pub fn storage_threshold(buff_mgr: &BuffMgr, holder_uid: i64, max_hp: i32) -> Option<i32> {
        let permille = active_injury_bank_params(buff_mgr, holder_uid)?
            .storage_threshold_permille
            .max(0);
        let threshold = max_hp.max(0).saturating_mul(permille) / 1000;
        (threshold > 0).then_some(threshold)
    }

    pub fn insight_iii_heal_amount(buff_mgr: &BuffMgr, holder_uid: i64, max_hp: i32) -> i32 {
        let permille = active_injury_bank_params(buff_mgr, holder_uid)
            .map(|p| p.heal_permille)
            .unwrap_or(0);
        max_hp.max(0).saturating_mul(permille) / 1000
    }
}

pub fn has_empathy_buff(buff_mgr: &BuffMgr, target_uid: i64) -> bool {
    buff_mgr
        .find_instance_by_type_id(target_uid, empathy_type_id())
        .is_some()
}

pub(crate) fn is_incoming_damage_effect_type(effect_type: Option<i32>) -> bool {
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
/// matching by `empathy_type_id()` so portrait/rank variants are
/// handled. If no instance exists yet, creates one using
/// `empathy_default_buff_id()` (canonical lowest-id member).
pub(crate) fn ensure_empathy_buff(buff_mgr: &mut BuffMgr, target_uid: i64) -> (i32, i64) {
    let type_id = empathy_type_id();
    if let Some(existing) = buff_mgr.find_instance_by_type_id(target_uid, type_id) {
        return (existing.buff_id, existing.uid);
    }

    let default_buff = empathy_default_buff_id();
    buff_mgr.add(target_uid, default_buff, target_uid, 0, 0);
    buff_mgr
        .find_instance_by_type_id(target_uid, type_id)
        .map(|buff| (buff.buff_id, buff.uid))
        .unwrap_or((default_buff, 0))
}

fn parse_empathy_value(params: &str) -> Option<i32> {
    let mut parts = params.split('#');
    let act_id = parts.next()?.trim().parse::<i32>().ok()?;
    if act_id != injury_bank_act_id() {
        return None;
    }
    parts.next()?.trim().parse::<i32>().ok()
}

fn build_empathy_params(current: i32, cap: i32) -> String {
    format!("{}#{}#{}", injury_bank_act_id(), current.max(0), cap.max(0))
}
