use super::{
    manager::buff_mgr::{BuffMgr, next_buff_uid_for_target, next_slave_buff_uid_for_target},
    manager::ex_point_mgr::ExPointMgr,
    mechanics::bloodtithe::BloodtitheState,
    skill::get_entity,
    types::{buff::BuffLayerType, career::CareerType},
};

use once_cell::sync::Lazy;
use sonettobuf::{
    ActEffect, BuffInfo, Fight, FightEntityInfo, FightHurtInfo, FightStep,
    effect_type_enum::EffectType, fight_hurt_info::DamageFromType, fight_step,
};
use std::{collections::HashMap, sync::Mutex};

static BLOOD_POOL_EX_TRACKER: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BLOOD_POOL_EX_ACCUM_TRACKER: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

//skill_behaviour table
pub enum VfxConfig {
    Heal = 20001,
    Damage = 30006,
    Moxie = 20002,
}

#[allow(dead_code)]
pub enum EffectTag {
    None = 0,
    Skill = 1,
    SkillEffect = 2,
    Buff = 3,
    Additional = 4,
    AbsorbHurt = 5,
    ShareHurt = 6,
}

#[allow(dead_code)]
pub enum DamageType {
    Reality = 1,
    Mental = 2,
}

#[allow(dead_code)]
pub fn buff_add(target_uid: i64, from_uid: i64, buff_id: i32, layer: i32) -> ActEffect {
    buff_add_with_count(target_uid, from_uid, buff_id, layer, 0)
}

pub fn buff_add_with_count(
    target_uid: i64,
    from_uid: i64,
    buff_id: i32,
    layer: i32,
    count: i32,
) -> ActEffect {
    let params = buff_get_act_common_params(buff_id);
    ActEffect {
        effect_type: Some(EffectType::Buffadd as i32),
        target_id: Some(target_uid),
        effect_num: Some(buff_id),
        buff: Some(create_buff(
            target_uid, buff_id, from_uid, count, layer, &params, false,
        )),
        ..Default::default()
    }
}

pub fn buff_update(
    target_uid: i64,
    from_uid: i64,
    buff_id: i32,
    buff_uid: i64,
    count: i32,
    layer: i32,
) -> ActEffect {
    let params = buff_get_act_common_params(buff_id);
    ActEffect {
        effect_type: Some(EffectType::Buffupdate as i32),
        target_id: Some(target_uid),
        effect_num: Some(0),
        buff: Some(BuffInfo {
            buff_id: Some(buff_id),
            duration: Some(0),
            uid: Some(buff_uid),
            ex_info: Some(0),
            from_uid: Some(from_uid),
            count: Some(count),
            act_common_params: Some(params.to_string()),
            layer: Some(layer),
            r#type: Some(BuffLayerType::Normal as i32),
            act_info: vec![],
        }),
        ..Default::default()
    }
}

pub fn buff_add_slave(target_uid: i64, from_uid: i64, buff_id: i32, layer: i32) -> ActEffect {
    let cfg = config::configs::get();
    let params = buff_get_act_common_params(buff_id);
    // For stackable buffs (layer > 1), initial count is 1; otherwise use effect_count from config
    let initial_count = if layer > 1 {
        1
    } else {
        cfg.skill_buff
            .iter()
            .find(|b| b.id == buff_id)
            .map(|b| b.effect_count)
            .unwrap_or(0)
    };
    ActEffect {
        effect_type: Some(EffectType::Buffadd as i32),
        target_id: Some(target_uid),
        effect_num: Some(buff_id),
        buff: Some(create_buff(
            target_uid,
            buff_id,
            from_uid,
            initial_count,
            layer,
            &params,
            true,
        )),
        ..Default::default()
    }
}

/// Damage with visual effect
pub fn damage(target_uid: i64, amount: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Damage as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        config_effect: Some(VfxConfig::Damage as i32),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn damage_buff(target_uid: i64, amount: i32, buff_id: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Damage as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        buff_act_id: Some(buff_id),
        ..Default::default()
    }
}

pub fn damage_with_hurt(
    target_uid: i64,
    amount: i32,
    config_effect: i32,
    skill_id: i32,
    from_uid: i64,
) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Damage as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        config_effect: Some(config_effect),
        hurt_info: Some(FightHurtInfo {
            damage: Some(amount),
            reduce_hp: Some(0),
            hurt_effect: Some(EffectType::Damage as i32),
            damage_from_type: Some(DamageFromType::SkillEffect as i32),
            config_effect: Some(config_effect),
            effect_id: Some(skill_id),
            skill_id: Some(skill_id),
            from_uid: Some(from_uid),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Damage details
pub fn hurt_detail(
    target_uid: i64,
    damage: i32,
    skill_id: i32,
    damage_type: i32,
    hurt_effect: i32,
) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Fighthurtdetail as i32),
        target_id: Some(target_uid),
        hurt_info: Some(FightHurtInfo {
            damage: Some(damage),
            reduce_hp: Some(damage),
            reduce_shield: Some(0),
            career_restraint: Some(false),
            critical: Some(false),
            assassinate: Some(false),
            hurt_effect: Some(hurt_effect),
            damage_from_type: Some(damage_type),
            config_effect: Some(VfxConfig::Damage as i32),
            effect_id: Some(skill_id),
            skill_id: Some(skill_id),
            from_uid: Some(target_uid),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn hurt_detail_buff(
    target_uid: i64,
    damage: i32,
    skill_id: i32,
    damage_type: i32,
    buff_id: i32,
) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Fighthurtdetail as i32),
        target_id: Some(target_uid),
        hurt_info: Some(FightHurtInfo {
            damage: Some(damage),
            reduce_hp: Some(damage),
            reduce_shield: Some(0),
            career_restraint: Some(false),
            critical: Some(false),
            assassinate: Some(false),
            hurt_effect: Some(EffectType::Damage as i32),
            damage_from_type: Some(damage_type),
            config_effect: Some(0),
            buff_act_id: Some(buff_id),
            effect_id: Some(skill_id),
            skill_id: Some(skill_id),
            from_uid: Some(target_uid),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Moxie change with visual effect
pub fn moxie_change(target_uid: i64, amount: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(crate::state::battle::types::effects::EffectType::ExPointChange as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        config_effect: Some(VfxConfig::Moxie as i32),
        ..Default::default()
    }
}

/// Changes your max hp not current
#[allow(dead_code)]
pub fn max_hp_change(target_uid: i64, amount: i32, buff_id: Option<i32>) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Maxhpchange as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        buff_act_id: buff_id,
        ..Default::default()
    }
}

/// Changes your current hp not max
#[allow(dead_code)]
pub fn current_hp_change(target_uid: i64, amount: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Currenthpchange as i32),
        target_id: Some(target_uid),
        effect_num: Some(amount),
        ..Default::default()
    }
}

pub fn attr_update(target_uid: i64) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Attr as i32),
        target_id: Some(target_uid),
        effect_num: Some(0),
        ..Default::default()
    }
}

pub fn effect_none(target_uid: i64) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::None as i32),
        target_id: Some(target_uid),
        ..Default::default()
    }
}

pub fn master_halo(target: i64) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Masterhalo as i32),
        target_id: Some(target),
        ..Default::default()
    }
}

pub fn slave_halo(target: i64) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Slavehalo as i32),
        target_id: Some(target),
        ..Default::default()
    }
}

fn create_buff(
    target_uid: i64,
    buff_id: i32,
    from_uid: i64,
    count: i32,
    layer: i32,
    params: &str,
    slave: bool,
) -> BuffInfo {
    let cfg = config::configs::get();
    let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == buff_id);
    let duration = buff_cfg.map(|b| b.during_time).unwrap_or(0);

    let uid_val = if slave {
        next_slave_buff_uid_for_target(target_uid)
    } else {
        next_buff_uid_for_target(target_uid)
    };
    BuffInfo {
        buff_id: Some(buff_id),
        duration: Some(duration),
        uid: Some(uid_val),
        ex_info: Some(0),
        from_uid: Some(from_uid),
        count: Some(count),
        act_common_params: Some(params.to_string()),
        layer: Some(layer),
        r#type: Some(BuffLayerType::Normal as i32),
        act_info: vec![],
    }
}

pub fn buff_has_bloodpool(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    if buff.features.is_empty() {
        return false;
    }
    buff.features.split('|').any(|entry| {
        let act_id: i32 = entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        cfg.buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "BloodPoolTag")
            .unwrap_or(false)
    })
}

pub fn for_each_buff_feature_chain(buff_id: i32, mut f: impl FnMut(&str, &[&str])) {
    let cfg = config::configs::get();
    let mut stack = vec![buff_id];
    let mut seen = std::collections::HashSet::new();

    while let Some(current_buff_id) = stack.pop() {
        if current_buff_id <= 0 || !seen.insert(current_buff_id) {
            continue;
        }
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == current_buff_id) else {
            continue;
        };
        if buff_cfg.features.is_empty() {
            continue;
        }

        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id = parts
                .first()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            if act_type == "SubBuff" {
                for raw in parts.iter().skip(1) {
                    for piece in raw.split(',') {
                        let Ok(child_buff_id) = piece.trim().parse::<i32>() else {
                            continue;
                        };
                        if child_buff_id > 0 {
                            stack.push(child_buff_id);
                        }
                    }
                }
            }

            f(act_type, &parts);
        }
    }
}

pub fn buff_del(target: i64, buff_uid: i64, buff_id: i32, from_uid: i64) -> ActEffect {
    let cfg = config::configs::get();
    let layer = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| if b.features.is_empty() { 0 } else { 1 })
        .unwrap_or(0);
    ActEffect {
        effect_type: Some(EffectType::Buffdel as i32),
        target_id: Some(target),
        buff: Some(sonettobuf::BuffInfo {
            uid: Some(buff_uid),
            buff_id: Some(buff_id),
            from_uid: Some(from_uid),
            layer: Some(layer),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn buff_get_raspberry_params(buff_id: i32) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let is_raspberry = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "Raspberry")
            .unwrap_or(false);
        if is_raspberry {
            let rate: i32 = parts.get(1)?.trim().parse().ok()?;
            Some((act_id, rate))
        } else {
            None
        }
    })
}

pub fn damage_with_buff_act(
    target: i64,
    amount: i32,
    buff_act_id: i32,
    from_uid: i64,
    buff_uid: i64,
) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Damage as i32),
        target_id: Some(target),
        effect_num: Some(amount),
        buff_act_id: Some(buff_act_id),
        hurt_info: Some(sonettobuf::FightHurtInfo {
            damage: Some(amount),
            hurt_effect: Some(EffectType::Damage as i32),
            damage_from_type: Some(EffectTag::Buff as i32),
            buff_act_id: Some(buff_act_id),
            from_uid: Some(from_uid),
            buff_uid: Some(buff_uid as i32),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn buff_get_act_common_params(buff_id: i32) -> String {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return String::new();
    };
    for entry in buff.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts
            .first()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let is_overflow = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "ExPointOverflowBank")
            .unwrap_or(false);
        if is_overflow {
            return format!("{}#0", act_id);
        }
    }
    String::new()
}

/// Check if any active buff on the caster has AttrOnlyCalDamageReplaceAttr feature.
/// Returns (source_attr, replace_attr, permille) if found.
/// Feature format: buff_act_id#source_attr#replace_attr#permille
/// Sum flat Attr bonus for a given attr_id from all active buffs on entity.
/// Handles buff_act type "Attr": feature format "100#attr_id#amount"
/// Also handles "AttrByLostHp" (853): scales bonus by lost HP ratio.
pub fn get_attr_bonus(
    buff_mgr: &BuffMgr,
    fight: &sonettobuf::Fight,
    entity_uid: i64,
    attr_id: i32,
) -> i32 {
    let cfg = config::configs::get();
    let mut total = 0i32;
    let mut stacked_key_total: std::collections::HashMap<String, i32> =
        std::collections::HashMap::new();
    let mut stacked_key_layer_bonus: std::collections::HashMap<String, i32> =
        std::collections::HashMap::new();
    let mut handled_stacked_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for buff in buff_mgr.get(entity_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        let Some(buff_type_cfg) = cfg.skill_bufftype.iter().find(|t| t.id == buff_cfg.type_id)
        else {
            continue;
        };
        let include_type = buff_type_cfg
            .include_types
            .split('#')
            .next()
            .unwrap_or_default()
            .trim();
        if matches!(include_type, "10" | "12" | "14" | "15") {
            let sign_key = format!("{}__{}", buff_cfg.features, buff_cfg.type_id);
            *stacked_key_total.entry(sign_key.clone()).or_insert(0) += 1;
            if buff.layer > 0 {
                *stacked_key_layer_bonus.entry(sign_key).or_insert(0) += buff.layer - 1;
            }
        }
    }

    for buff in buff_mgr.get(entity_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        let buff_type_cfg = cfg.skill_bufftype.iter().find(|t| t.id == buff_cfg.type_id);
        let include_type = buff_type_cfg
            .and_then(|t| t.include_types.split('#').next())
            .unwrap_or_default()
            .trim();
        let is_stacked = matches!(include_type, "10" | "12" | "14" | "15");
        let sign_key = format!("{}__{}", buff_cfg.features, buff_cfg.type_id);
        let stack_multiplier = if is_stacked {
            if !handled_stacked_keys.insert(sign_key.clone()) {
                continue;
            }
            let base = stacked_key_total.get(&sign_key).copied().unwrap_or(1);
            let layer_bonus = stacked_key_layer_bonus.get(&sign_key).copied().unwrap_or(0);
            (base + layer_bonus).max(1)
        } else {
            (if buff.stacks > 0 { buff.stacks } else { 1 }).max(1)
        };

        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id: i32 = match parts.first().and_then(|v| v.trim().parse().ok()) {
                Some(id) => id,
                None => continue,
            };
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            match act_type {
                "Attr" => {
                    // format: act_id#attr_id#amount
                    let feat_attr: i32 = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                        Some(v) => v,
                        None => continue,
                    };
                    if feat_attr != attr_id {
                        continue;
                    }
                    let amount: i32 = parts
                        .get(2)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if is_stacked && attr_id == 206 {
                        // Live-like behavior for stacked DropDmg-style shields:
                        // apply one instance per hit, not full stack multiplication.
                        total += amount;
                    } else {
                        total += amount * stack_multiplier;
                    }
                }
                "AttrOnlyCalDamageAttack"
                | "AttrOnlyCalDamageBeAttacked"
                | "AttrOnlyCalDamageAttackType"
                | "AttrOnlyCalDamageBeAttackedType" => {
                    // format typically: act_id#attr_id#amount#[optional_type]
                    let feat_attr: i32 = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                        Some(v) => v,
                        None => continue,
                    };
                    if feat_attr != attr_id {
                        continue;
                    }
                    let amount: i32 = parts
                        .get(2)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if is_stacked && attr_id == 206 {
                        total += amount;
                    } else {
                        total += amount * stack_multiplier;
                    }
                }
                "AttrByLostHp" => {
                    // format: act_id#source_attr#attr_ids(comma)#amounts(comma)#max_stacks
                    // attrs and amounts are comma-separated pairs
                    let attr_ids_str = match parts.get(2) {
                        Some(v) => *v,
                        None => continue,
                    };
                    let amounts_str = match parts.get(3) {
                        Some(v) => *v,
                        None => continue,
                    };
                    let max_stacks: i32 = parts
                        .get(4)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1);

                    let attr_ids: Vec<i32> = attr_ids_str
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    let amounts: Vec<i32> = amounts_str
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();

                    let pos = match attr_ids.iter().position(|&a| a == attr_id) {
                        Some(p) => p,
                        None => continue,
                    };
                    let base_amount = match amounts.get(pos) {
                        Some(&a) => a,
                        None => continue,
                    };

                    // Scale by lost HP ratio * max_stacks
                    if let Some(entity) = get_entity(fight, entity_uid) {
                        let max_hp = entity.attr.as_ref().and_then(|a| a.hp).unwrap_or(1).max(1);
                        let cur_hp = entity.current_hp.unwrap_or(max_hp);
                        let lost_permille = ((max_hp - cur_hp).max(0) * 1000) / max_hp;
                        let stacks = (lost_permille * max_stacks / 1000).min(max_stacks);
                        total += base_amount * stacks * stack_multiplier;
                    }
                }
                _ => {}
            }
        }
    }
    total
}

pub fn get_attr_replace_damage(buff_mgr: &BuffMgr, caster_uid: i64) -> Option<(i32, i32, i32)> {
    let cfg = config::configs::get();
    for buff in buff_mgr.get(caster_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id: i32 = match parts.first().and_then(|v| v.trim().parse().ok()) {
                Some(id) => id,
                None => continue,
            };
            let is_replace = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| {
                    a.r#type == "AttrOnlyCalDamageReplaceAttr"
                        || a.r#type == "AttrOnlyCalDamageReplaceAttrADCreator"
                })
                .unwrap_or(false);
            if is_replace {
                let source_attr = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                let replace_attr = match parts.get(2).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                let permille = match parts.get(3).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                return Some((source_attr, replace_attr, permille));
            }
        }
    }
    None
}

pub fn get_exclude_buff_effects(buff_mgr: &BuffMgr, target: i64, buff_id: i32) -> Vec<ActEffect> {
    let mut effects = Vec::new();
    let cfg = config::configs::get();

    tracing::warn!(
        "get_exclude_buff_effects buff_id={} target={} existing_buffs={:?}",
        buff_id,
        target,
        buff_mgr
            .get(target)
            .iter()
            .map(|b| b.buff_id)
            .collect::<Vec<_>>()
    );

    // look up buff_type by the buff's typeId
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id);
    let type_id = buff.map(|b| b.type_id).unwrap_or(buff_id);

    let Some(buff_type) = cfg.skill_bufftype.iter().find(|t| t.id == type_id) else {
        return effects;
    };

    if buff_type.exclude_types.is_empty() {
        return effects;
    }

    let exclude_str = buff_type.exclude_types.trim_start_matches("2#");
    let excluded_type_ids: Vec<i32> = exclude_str
        .split(['，', ','])
        .filter_map(|v| v.trim().parse().ok())
        .collect();

    for instance in buff_mgr.get(target) {
        if excluded_type_ids.contains(&instance.type_id) {
            effects.push(buff_del(
                target,
                instance.uid,
                instance.buff_id,
                instance.from_uid,
            ));
        }
    }

    effects
}

pub fn find_entity(fight: &Fight, uid: i64) -> Option<&FightEntityInfo> {
    if let Some(a) = &fight.attacker {
        for e in a.entitys.iter().chain(a.sub_entitys.iter()) {
            if e.uid == Some(uid) {
                return Some(e);
            }
        }
    }
    if let Some(d) = &fight.defender {
        for e in d.entitys.iter().chain(d.sub_entitys.iter()) {
            if e.uid == Some(uid) {
                return Some(e);
            }
        }
    }
    None
}

pub fn buff_get_blood_pool_ex_point_params(buff_id: i32) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };

        match act_type {
            "BloodPoolCountAddExPoint" => {
                let threshold = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let amount = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                if threshold > 0 && amount > 0 {
                    return Some((threshold, amount));
                }
            }
            "ExPointOverflowBank" => {
                let threshold = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                if threshold > 0 {
                    return Some((threshold, 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn buff_uses_blood_pool_gain_accum(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    buff.features.split('|').any(|segment| {
        let parts: Vec<&str> = segment.split('#').collect();
        let Some(feature_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
            return false;
        };
        cfg.buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type == "BloodPoolCountAddExPoint")
            .unwrap_or(false)
    })
}

#[allow(dead_code)]
pub fn buff_get_blood_value_use_skill_params(buff_id: i32) -> Option<(i32, i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };
        if act_type != "BloodValueUseSkill" {
            continue;
        }
        let prerequisite_buff_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let blood_value_threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let wrapper_skill_id = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        if wrapper_skill_id > 0 {
            return Some((
                prerequisite_buff_id,
                blood_value_threshold,
                wrapper_skill_id,
            ));
        }
    }
    None
}

pub fn buff_get_nuodika_channel_params(buff_id: i32) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };
        if act_type != "NuoDiKaCastChannel" {
            continue;
        }
        let duration = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let max_points = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let points_per_trigger = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        let output_skill_id = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
        let counter_buff_id = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
        if threshold > 0 && points_per_trigger > 0 && output_skill_id > 0 {
            return Some((
                duration,
                threshold,
                max_points,
                points_per_trigger,
                output_skill_id,
                counter_buff_id,
            ));
        }
    }
    None
}

pub fn build_blood_pool_ex_point_step(
    bloodtithe: &mut BloodtitheState,
    fight: &Fight,
    buff_mgr: &BuffMgr,
    ex_point_mgr: &mut ExPointMgr,
) -> Option<FightStep> {
    if !bloodtithe.initialized {
        return None;
    }
    if bloodtithe.get_value(1) == 0 {
        return None;
    }

    let uids: Vec<i64> = fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
        .unwrap_or_default();

    let mut outer_effects = Vec::new();
    let battle_key = fight.battle_id.unwrap_or(0);
    let mut tracker = BLOOD_POOL_EX_TRACKER.lock().unwrap();

    for uid in uids {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let total = bloodtithe.get_value(team_type);
        let model_id = find_entity(fight, uid).and_then(|e| e.model_id);
        for instance in buff_mgr.get(uid) {
            if buff_uses_blood_pool_gain_accum(instance.buff_id) {
                continue;
            }
            if model_id == Some(3125) && instance.buff_id == 31250161 {
                // Rubuska's 806 tracker is Shadow Cloak-driven, not bloodpool-driven.
                continue;
            }
            let Some((threshold, amount)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
            else {
                continue;
            };
            let tracker_key = (battle_key, instance.uid);
            let Some(previous_total) = tracker.get(&tracker_key).copied() else {
                tracker.insert(tracker_key, total);
                continue;
            };
            let gain = ((total / threshold) - (previous_total / threshold)).max(0) * amount;
            tracker.insert(tracker_key, total);
            if gain <= 0 {
                continue;
            }

            ex_point_mgr.add_ex_point(uid, gain);

            let mut act_effect = Vec::new();
            for _ in 0..gain {
                act_effect.push(ActEffect {
                    effect_type: Some(0),
                    effect_num: Some(instance.buff_id),
                    buff_act_id: Some(1021),
                    target_id: Some(uid),
                    ..Default::default()
                });
                act_effect.push(ActEffect {
                    effect_type: Some(EffectType::Expointchange as i32),
                    effect_num: Some(1),
                    target_id: Some(uid),
                    ..Default::default()
                });
            }

            outer_effects.push(ActEffect {
                effect_type: Some(EffectType::Fightstep as i32),
                target_id: Some(0),
                fight_step: Some(FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    from_id: Some(uid),
                    to_id: Some(uid),
                    act_id: Some(instance.buff_id),
                    act_effect,
                    card_index: Some(0),
                    support_hero_id: Some(0),
                    fake_timeline: Some(false),
                    real_skill_type: Some(0),
                    real_skin_id: Some(0),
                }),
                ..Default::default()
            });
        }
    }

    if outer_effects.is_empty() {
        return None;
    }

    Some(FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        act_effect: outer_effects,
        ..Default::default()
    })
}

pub fn build_blood_pool_gain_ex_point_step(
    bloodtithe: &BloodtitheState,
    fight: &Fight,
    buff_mgr: &BuffMgr,
    ex_point_mgr: &mut ExPointMgr,
    gains_by_team: &[(i32, i32)],
    _gains_by_skill_team: &[(i32, i32, i32)],
) -> Option<FightStep> {
    let battle_key = fight.battle_id.unwrap_or(0);
    if battle_key == 0 {
        return None;
    }

    let gain_lookup: HashMap<i32, i32> = gains_by_team
        .iter()
        .copied()
        .filter(|(_, gain)| *gain > 0)
        .collect();
    if gain_lookup.is_empty() {
        return None;
    }

    let uids: Vec<i64> = fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
        .unwrap_or_default();

    let mut outer_effects = Vec::new();
    let mut accum_tracker = BLOOD_POOL_EX_ACCUM_TRACKER.lock().unwrap();
    for uid in uids {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let Some(delta) = gain_lookup.get(&team_type).copied() else {
            continue;
        };
        let current_total = bloodtithe.get_value(team_type).max(0);
        for instance in buff_mgr.get(uid) {
            if !buff_uses_blood_pool_gain_accum(instance.buff_id) {
                continue;
            }
            let Some((threshold, amount)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
            else {
                continue;
            };
            let tracker_key = (battle_key, instance.uid);
            let entry = accum_tracker
                .entry(tracker_key)
                .or_insert_with(|| current_total.rem_euclid(threshold));
            *entry += delta;
            let gain = (*entry / threshold).max(0) * amount;
            if gain <= 0 {
                continue;
            }
            *entry %= threshold;
            ex_point_mgr.add_ex_point(uid, gain);

            let mut act_effect = Vec::new();
            for _ in 0..gain {
                act_effect.push(ActEffect {
                    effect_type: Some(0),
                    effect_num: Some(instance.buff_id),
                    buff_act_id: Some(1021),
                    target_id: Some(uid),
                    ..Default::default()
                });
                act_effect.push(ActEffect {
                    effect_type: Some(EffectType::Expointchange as i32),
                    effect_num: Some(1),
                    target_id: Some(uid),
                    ..Default::default()
                });
            }

            outer_effects.push(ActEffect {
                effect_type: Some(EffectType::Fightstep as i32),
                target_id: Some(0),
                fight_step: Some(FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    from_id: Some(uid),
                    to_id: Some(uid),
                    act_id: Some(instance.buff_id),
                    act_effect,
                    card_index: Some(0),
                    support_hero_id: Some(0),
                    fake_timeline: Some(false),
                    real_skill_type: Some(0),
                    real_skin_id: Some(0),
                }),
                ..Default::default()
            });
        }
    }

    if outer_effects.is_empty() {
        return None;
    }

    Some(FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        act_effect: outer_effects,
        ..Default::default()
    })
}

// Consumed by battle_gen's replay bootstrap; clippy can't see cross-crate callers.
#[allow(dead_code)]
pub fn seed_blood_pool_ex_tracker(bloodtithe: &BloodtitheState, fight: &Fight, buff_mgr: &BuffMgr) {
    if !bloodtithe.initialized {
        return;
    }

    let battle_key = fight.battle_id.unwrap_or(0);
    if battle_key == 0 {
        return;
    }

    let mut tracker = BLOOD_POOL_EX_TRACKER.lock().unwrap();
    let mut accum_tracker = BLOOD_POOL_EX_ACCUM_TRACKER.lock().unwrap();
    for uid in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().filter_map(|e| e.uid))
    {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let total = bloodtithe.get_value(team_type);
        let model_id = find_entity(fight, uid).and_then(|e| e.model_id);
        for instance in buff_mgr.get(uid) {
            if buff_uses_blood_pool_gain_accum(instance.buff_id) {
                if let Some((threshold, _)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
                {
                    accum_tracker.insert((battle_key, instance.uid), total.rem_euclid(threshold));
                }
                continue;
            }
            if model_id == Some(3125) && instance.buff_id == 31250161 {
                continue;
            }
            if buff_get_blood_pool_ex_point_params(instance.buff_id).is_none() {
                continue;
            }
            tracker.insert((battle_key, instance.uid), total);
        }
    }
}

pub fn check_career_restraint(attacker_career: i32, defender_career: i32) -> bool {
    let attacker = CareerType::from(attacker_career);
    let defender = CareerType::from(defender_career);

    matches!(
        (attacker, defender),
        (CareerType::Mineral, CareerType::Beast) | // Mineral beats Beast
        (CareerType::Star, CareerType::Mineral) |  // Star beats Mineral
        (CareerType::Plant, CareerType::Star) |    // Plant beats Star
        (CareerType::Beast, CareerType::Plant) |   // Beast beats Plant
        (CareerType::Spirit, CareerType::Intellect) | // Spirit beats Intellect
        (CareerType::Intellect, CareerType::Spirit) // Intellect beats Spirit
    )
}

pub fn career_damage_multiplier(attacker_career: i32, defender_career: i32) -> i32 {
    let attacker = CareerType::from(attacker_career);
    let defender = CareerType::from(defender_career);

    if check_career_restraint(attacker as i32, defender as i32) {
        1300 // 130% = +30% bonus
    } else if check_career_restraint(defender as i32, attacker as i32) {
        700 // 70% = -30% penalty
    } else {
        1000 // 100% = no modifier
    }
}

#[allow(dead_code)]
pub fn modify_hero_attr(entity: &mut FightEntityInfo, attr_id: i32, amount_permille: i32) {
    use super::types::attr::AttrId;
    let attr = entity.attr.get_or_insert_with(Default::default);
    match AttrId::from(attr_id) {
        Some(AttrId::Hp) => attr.hp = Some(attr.hp.unwrap_or(0) + amount_permille),
        Some(AttrId::Attack) => {
            attr.attack = Some(attr.attack.unwrap_or(0) * (1000 + amount_permille) / 1000)
        }
        Some(AttrId::Cri) => {}    // stat only, no direct attr field
        Some(AttrId::CriDmg) => {} // stat only
        Some(AttrId::AddDmg) => {} // stat only
        _ => tracing::warn!("modify_hero_attr: unhandled attr_id={}", attr_id),
    }
}
