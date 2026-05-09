//! Kakania — `Id, Ego and Superego` (EX) and the `Empathy` Mental
//! resource. Generic Empathy state lives in
//! `mechanics::empathy::EmpathyState` (storage HashMap, init, sync,
//! and the cumulative `apply_storage_with_threshold` mutator); the
//! kit-level rules her shared kit calls back into — Insight I 50%
//! damage redirect, the per-skill `StorageInjury` injection, the
//! Insight III storage-threshold heal trigger, and the Insight III
//! bonus-damage bounce — all live here.

use sonettobuf::{ActEffect, Fight};

use crate::state::battle::{
    fight_step::{ActEffectBuilder, FightStepBuilder},
    hero::HeroId,
    manager::buff_mgr::BuffMgr,
    mechanics::empathy::{
        EmpathyState, active_injury_bank_params, ensure_empathy_buff, has_empathy_buff,
        is_incoming_damage_effect_type,
    },
    skill::targets::{alive_allies, alive_enemies, get_entity, get_team_type},
    types::effects::EffectType,
    utils::{apply_real_hurt_fix, find_uid_by_hero_id},
};

#[allow(dead_code)]
pub fn is_kakania(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Kakania.model_id())
}

/// Locate Kakania anywhere in the fight (either side). Returns
/// `None` when she isn't on the field.
pub fn find_uid(fight: &Fight) -> Option<i64> {
    find_uid_by_hero_id(fight, HeroId::Kakania.model_id())
}

/// When an Empathy holder takes incoming skill damage, emit the
/// cumulative `StorageInjury` marker before the damage packet and
/// update the preview buff state so later behavior slots see the
/// stored total immediately.
pub fn inject_storage_injury_for_damage_emissions(
    state: &mut EmpathyState,
    buff_mgr: &mut BuffMgr,
    fight: &Fight,
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

    let cap = EmpathyState::storage_cap(buff_mgr, target_uid, target_max_hp);
    let mut out = Vec::with_capacity(effects.len());
    for effect in effects {
        let should_inject = effect.target_id == Some(target_uid)
            && is_incoming_damage_effect_type(effect.effect_type);
        if !should_inject {
            out.push(effect);
            continue;
        }

        let damage = effect.effect_num.unwrap_or(0).max(0);
        let storage = EmpathyState::compute_storage_amount(damage);
        let (current_total, thresholds_crossed) =
            state.apply_storage_with_threshold(buff_mgr, target_uid, storage, target_max_hp);
        let (buff_id, buff_uid) = ensure_empathy_buff(buff_mgr, target_uid);
        out.push(state.emit_storage_injury(
            target_uid,
            current_total,
            buff_id,
            buff_uid,
            target_uid,
            cap,
        ));
        out.push(effect);
        out.extend(build_insight_iii_threshold_heals(
            state,
            buff_mgr,
            fight,
            target_uid,
            target_max_hp,
            thresholds_crossed,
        ));
    }

    out
}

/// Insight I redirect: when an enemy damages one of Kakania's allies,
/// divert 50% of that packet to Kakania as `DamageFromAbsorb` and bank
/// 10% of the absorbed amount as Empathy before the original damage.
pub fn inject_damage_redirect(
    state: &mut EmpathyState,
    preview_buff_mgr: &mut BuffMgr,
    live_buff_mgr: &mut BuffMgr,
    fight: &Fight,
    source_uid: i64,
    effects: Vec<ActEffect>,
) -> Vec<ActEffect> {
    let Some(kakania_uid) = find_uid(fight) else {
        return effects;
    };
    let Some(kakania) = get_entity(fight, kakania_uid) else {
        return effects;
    };
    let Some(kakania_team) = get_team_type(fight, kakania_uid) else {
        return effects;
    };
    let Some(source_team) = get_team_type(fight, source_uid) else {
        return effects;
    };
    let kakania_max_hp = kakania.attr.as_ref().and_then(|attr| attr.hp).unwrap_or(0);
    if source_uid == kakania_uid
        || source_team == kakania_team
        || kakania.current_hp.unwrap_or(0) <= 0
        || kakania_max_hp <= 0
        || !has_empathy_buff(preview_buff_mgr, kakania_uid)
    {
        return effects;
    }

    let cap = EmpathyState::storage_cap(preview_buff_mgr, kakania_uid, kakania_max_hp);
    let mut out = Vec::with_capacity(effects.len().saturating_mul(3));
    for mut effect in effects {
        let Some(target_uid) = effect.target_id else {
            out.push(effect);
            continue;
        };
        let Some(target_team) = get_team_type(fight, target_uid) else {
            out.push(effect);
            continue;
        };
        if target_uid == source_uid
            || target_uid == kakania_uid
            || target_team != kakania_team
            || !is_incoming_damage_effect_type(effect.effect_type)
        {
            out.push(effect);
            continue;
        }

        let original_damage = effect.effect_num.unwrap_or(0).max(0);
        let requested_absorb = original_damage / 2;
        let remaining_storage = cap.saturating_sub(state.current(kakania_uid).max(0));
        let absorb_cap = remaining_storage.saturating_mul(10);
        let absorbed_damage = requested_absorb.min(absorb_cap);
        if absorbed_damage <= 0 {
            out.push(effect);
            continue;
        }

        let storage = EmpathyState::compute_storage_amount(absorbed_damage);
        let (current_total, thresholds_crossed) = state.apply_storage_with_threshold(
            preview_buff_mgr,
            kakania_uid,
            storage,
            kakania_max_hp,
        );
        state.sync_buff_state(live_buff_mgr, kakania_uid, current_total, kakania_max_hp);

        let (buff_id, buff_uid) = ensure_empathy_buff(preview_buff_mgr, kakania_uid);
        out.push(state.emit_storage_injury(
            kakania_uid,
            current_total,
            buff_id,
            buff_uid,
            kakania_uid,
            cap,
        ));
        out.push(
            ActEffectBuilder::new(EffectType::DamageFromAbsorb as i32, kakania_uid)
                .effect_num(absorbed_damage)
                .build(),
        );
        effect.effect_num = Some(original_damage.saturating_sub(absorbed_damage));
        out.push(effect);
        out.extend(build_insight_iii_threshold_heals(
            state,
            preview_buff_mgr,
            fight,
            kakania_uid,
            kakania_max_hp,
            thresholds_crossed,
        ));
    }

    out
}

pub fn build_insight_iii_threshold_heals(
    _state: &EmpathyState,
    buff_mgr: &BuffMgr,
    fight: &Fight,
    holder_uid: i64,
    holder_max_hp: i32,
    thresholds_crossed: i32,
) -> Vec<ActEffect> {
    if thresholds_crossed <= 0 {
        return Vec::new();
    }

    let heal_amount = EmpathyState::insight_iii_heal_amount(buff_mgr, holder_uid, holder_max_hp)
        .saturating_mul(thresholds_crossed);
    if heal_amount <= 0 {
        return Vec::new();
    }

    alive_allies(fight, holder_uid)
        .into_iter()
        .map(|ally_uid| {
            ActEffectBuilder::new(EffectType::InjuryBankHeal as i32, ally_uid)
                .effect_num(heal_amount)
                .build()
        })
        .collect()
}

pub fn build_insight_iii_bounce(
    state: &EmpathyState,
    buff_mgr: &BuffMgr,
    fight: &Fight,
    holder_uid: i64,
) -> Option<ActEffect> {
    let current_empathy = state.current(holder_uid);
    if current_empathy <= 0 {
        return None;
    }

    let params = active_injury_bank_params(buff_mgr, holder_uid)?;
    let (config_effect, multiplier_permille) =
        parse_insight_iii_bounce_behavior(params.insight_iii_bounce_skill_id)?;

    let bonus = current_empathy.saturating_mul(multiplier_permille) / 1000;
    let bounce_effects = alive_enemies(fight, holder_uid)
        .into_iter()
        .map(|enemy_uid| {
            ActEffectBuilder::origin_damage(
                enemy_uid,
                apply_real_hurt_fix(buff_mgr, enemy_uid, bonus),
                Some(config_effect),
            )
        })
        .collect::<Vec<_>>();
    if bounce_effects.is_empty() {
        return None;
    }

    Some(
        FightStepBuilder::skill(holder_uid, holder_uid, params.insight_iii_bounce_skill_id)
            .with_many(bounce_effects)
            .wrap(),
    )
}

pub fn inject_insight_iii_bounces_for_heal_emissions(
    state: &EmpathyState,
    buff_mgr: &BuffMgr,
    fight: &Fight,
    effects: Vec<ActEffect>,
) -> Vec<ActEffect> {
    let mut out = Vec::with_capacity(effects.len());
    for mut effect in effects {
        if let Some(step) = effect.fight_step.as_mut() {
            let inner = std::mem::take(&mut step.act_effect);
            step.act_effect =
                inject_insight_iii_bounces_for_heal_emissions(state, buff_mgr, fight, inner);
        }
        let should_inject = effect
            .target_id
            .filter(|target_uid| has_empathy_buff(buff_mgr, *target_uid))
            .is_some()
            && matches!(
                effect.effect_type,
                Some(t)
                    if t == EffectType::Heal as i32
                        || t == EffectType::InjuryBankHeal as i32
            );
        let target_uid = effect.target_id.unwrap_or(0);
        out.push(effect);
        if should_inject
            && let Some(bounce) = build_insight_iii_bounce(state, buff_mgr, fight, target_uid)
        {
            out.push(bounce);
        }
    }

    out
}

/// Insight III bounce-skill behavior payload `(config_effect, multiplier_permille)`,
/// parsed from the bounce skill's `OriginDamageFromInjuryBankBuff`-typed
/// behavior (e.g. `60052#1000` on `30800161`, `60052#1200` on `30800162`).
fn parse_insight_iii_bounce_behavior(bounce_skill_id: i32) -> Option<(i32, i32)> {
    if bounce_skill_id <= 0 {
        return None;
    }
    let cfg = config::configs::get();
    let bounce = cfg.skill_effect.iter().find(|s| s.id == bounce_skill_id)?;
    for beh in [
        bounce.behavior1.as_str(),
        bounce.behavior2.as_str(),
        bounce.behavior3.as_str(),
        bounce.behavior4.as_str(),
        bounce.behavior5.as_str(),
    ] {
        let parts: Vec<&str> = beh.split('#').collect();
        let beh_id: i32 = parts
            .first()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        if beh_id == 0 {
            continue;
        }
        let is_bounce = cfg
            .skill_behavior
            .iter()
            .find(|b| b.id == beh_id)
            .map(|b| b.r#type == "OriginDamageFromInjuryBankBuff")
            .unwrap_or(false);
        if !is_bounce {
            continue;
        }
        let multiplier = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        return Some((beh_id, multiplier));
    }
    None
}
