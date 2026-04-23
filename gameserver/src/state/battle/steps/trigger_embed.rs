use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::fight_step::wrap_step;

pub(crate) fn trigger_step_to_embedded_effect(trigger_step: FightStep) -> ActEffect {
    // Trigger steps are usually wrapped as EFFECT -> 162(fightStep=...).
    // Reuse that existing 162 directly to avoid creating a second wrapper.
    if trigger_step.act_type == Some(fight_step::ActType::Effect as i32)
        && trigger_step.act_effect.len() == 1
        && let Some(effect) = trigger_step.act_effect.first().cloned()
        && effect.effect_type == Some(162)
    {
        return effect;
    }
    wrap_step(trigger_step)
}

pub(crate) fn trigger_step_origin_uid(trigger_step: &FightStep) -> Option<i64> {
    if trigger_step.act_type == Some(fight_step::ActType::Effect as i32)
        && let Some(inner_from) = trigger_step
            .act_effect
            .iter()
            .find_map(|e| e.fight_step.as_ref().and_then(|s| s.from_id))
    {
        return Some(inner_from);
    }
    trigger_step.from_id
}

pub(crate) fn find_trigger_insert_index(effects: &[ActEffect]) -> usize {
    fn is_damage_effect(effect_type: i32) -> bool {
        effect_type == crate::state::battle::types::effects::EffectType::Damage as i32
            || effect_type == crate::state::battle::types::effects::EffectType::Crit as i32
            || effect_type == crate::state::battle::types::effects::EffectType::DamageExtra as i32
            || effect_type == crate::state::battle::types::effects::EffectType::OriginDamage as i32
            || effect_type == crate::state::battle::types::effects::EffectType::OriginCrit as i32
    }

    let mut idx = 0usize;
    while idx < effects.len() {
        let effect_type = effects[idx].effect_type.unwrap_or(0);
        if !is_damage_effect(effect_type) {
            break;
        }
        idx += 1;
    }
    idx
}

pub(crate) fn normalize_player_skill_effect_order(step: &mut FightStep) {
    if step.act_type != Some(fight_step::ActType::Skill as i32) {
        return;
    }
    if step.act_effect.len() < 3 {
        return;
    }

    fn is_damage_effect(effect_type: i32) -> bool {
        effect_type == crate::state::battle::types::effects::EffectType::Damage as i32
            || effect_type == crate::state::battle::types::effects::EffectType::Crit as i32
            || effect_type == crate::state::battle::types::effects::EffectType::DamageExtra as i32
            || effect_type == crate::state::battle::types::effects::EffectType::OriginDamage as i32
            || effect_type == crate::state::battle::types::effects::EffectType::OriginCrit as i32
    }

    let mut idx = 0usize;
    while idx < step.act_effect.len()
        && is_damage_effect(step.act_effect[idx].effect_type.unwrap_or(0))
    {
        idx += 1;
    }
    if idx == 0 || idx >= step.act_effect.len() {
        return;
    }

    let prefix = step.act_effect[..idx].to_vec();
    let mut wrappers = Vec::new();
    let mut tail = Vec::new();
    for effect in step.act_effect[idx..].iter().cloned() {
        if effect.effect_type == Some(162) {
            wrappers.push(effect);
        } else {
            tail.push(effect);
        }
    }
    if wrappers.is_empty() || tail.is_empty() {
        return;
    }

    wrappers.sort_by_key(|effect| {
        let inner_act_type = effect
            .fight_step
            .as_ref()
            .and_then(|s| s.act_type)
            .unwrap_or(0);
        if inner_act_type == fight_step::ActType::Effect as i32 {
            0
        } else {
            1
        }
    });

    let mut reordered = prefix;
    reordered.extend(wrappers);
    reordered.extend(tail);
    step.act_effect = reordered;
}

pub(crate) fn flatten_self_nested_skill_effects(step: &mut FightStep) {
    if step.act_type != Some(fight_step::ActType::Skill as i32) {
        return;
    }
    let host_act_id = step.act_id;
    let host_from = step.from_id;
    let mut flattened = Vec::with_capacity(step.act_effect.len());
    for mut effect in std::mem::take(&mut step.act_effect) {
        if effect.effect_type == Some(162)
            && let Some(inner) = effect.fight_step.take()
        {
            let inner_has_damage = inner.act_effect.iter().any(|e| {
                e.effect_type.is_some_and(|t| {
                    t == crate::state::battle::types::effects::EffectType::Damage as i32
                        || t == crate::state::battle::types::effects::EffectType::Crit as i32
                        || t == crate::state::battle::types::effects::EffectType::OriginDamage
                            as i32
                        || t == crate::state::battle::types::effects::EffectType::OriginCrit as i32
                })
            });
            let is_same_skill = inner.act_type == Some(fight_step::ActType::Skill as i32)
                && inner.act_id == host_act_id;
            // Flatten when the nested skill duplicates the host: either it
            // carries damage effects (original case) or it's a self-targeted
            // nested wrapper (e.g. 31200133 → 31200133 from=self to=self) that
            // just re-packs buff effects already represented at the top level.
            let is_self_target_duplicate = is_same_skill
                && inner.from_id == host_from
                && inner.to_id == host_from;
            if is_same_skill && (inner_has_damage || is_self_target_duplicate) {
                flattened.extend(inner.act_effect);
                continue;
            }
            effect.fight_step = Some(inner);
        }
        flattened.push(effect);
    }
    step.act_effect = flattened;
}

pub(crate) fn insert_trigger_into_matching_nested(
    host: &mut FightStep,
    embedded: ActEffect,
) -> bool {
    let Some(trigger_step) = embedded.fight_step.as_ref() else {
        return false;
    };
    let trigger_from = trigger_step.from_id.unwrap_or(0);
    if trigger_from == 0 {
        return false;
    }
    insert_trigger_into_matching_nested_inner(host, trigger_from, &embedded)
}

fn insert_trigger_into_matching_nested_inner(
    host: &mut FightStep,
    trigger_from: i64,
    embedded: &ActEffect,
) -> bool {
    for idx in (0..host.act_effect.len()).rev() {
        let Some(child_step) = host.act_effect[idx].fight_step.as_mut() else {
            continue;
        };
        if child_step.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }

        if insert_trigger_into_matching_nested_inner(child_step, trigger_from, embedded) {
            return true;
        }

        if child_step.from_id == Some(trigger_from) {
            let insert_at = find_trigger_insert_index(&child_step.act_effect);
            child_step.act_effect.insert(insert_at, embedded.clone());
            return true;
        }
    }
    false
}
