//! Rubuska owns Shadow Cloak and Raspberry kit routing.
//! Shared mechanics still build packets and maintain generic replay
//! state; this module keeps her ids, caps, and trigger rules together.

use once_cell::sync::Lazy;
use sonettobuf::{Fight, FightStep};
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
};

use crate::state::battle::{
    buff_actions::{find_feature_parts, raspberry::buff_get_raspberry_params},
    context::FightContext,
    hero::HeroId,
    manager::round_mgr::lookup_entry_max_hp,
    mechanics::shadowcloak::ShadowCloakState,
    round::step_shape::build_effect_step,
    skill::source_kind,
    utils::{find_entity, moxie_change},
};

pub const SHADOW_CLOAK_ACCUMULATOR_BUFF_ID: i32 = 31250151;
pub const SHADOW_CLOAK_OVERFLOW_TRACKER_BUFF_ID: i32 = 31250161;

/// Tunings for Rubuska's Shadow Cloak. Both values are read from the
/// accumulator buff's Raspberry feature parts (`1042#?#?#share#cap#…`)
/// rather than hardcoded — the data table is the source of truth.
/// Future portray-level overrides would surface as different buff IDs
/// or layered buffs; the lookup point stays here.
struct ShadowCloakTunings {
    share_rate_permille: i32,
    max_hp_cap_permille: i32,
}

static SHADOW_CLOAK_TUNINGS: Lazy<ShadowCloakTunings> = Lazy::new(|| {
    let parts = find_feature_parts(SHADOW_CLOAK_ACCUMULATOR_BUFF_ID, "Raspberry")
        .expect("Rubuska Shadow Cloak accumulator buff missing Raspberry feature");
    ShadowCloakTunings {
        share_rate_permille: parts
            .get(3)
            .and_then(|v| v.trim().parse().ok())
            .expect("Raspberry feature missing share-rate permille at parts[3]"),
        max_hp_cap_permille: parts
            .get(4)
            .and_then(|v| v.trim().parse().ok())
            .expect("Raspberry feature missing max-cap permille at parts[4]"),
    }
});

static SEEDED_RASPBERRY_MAX: Lazy<Mutex<HashMap<i32, i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static SHADOW_CLOAK_FULL_CAP_GRANTED: Lazy<Mutex<HashSet<(i32, i64)>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));
static BASIC_SELF_LOSS_SKILLS: OnceLock<HashSet<i32>> = OnceLock::new();

fn is_rubuska(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Rubuska.model_id())
}

pub fn shadow_cloak_capacity(entry_max_hp: i32) -> i32 {
    entry_max_hp * SHADOW_CLOAK_TUNINGS.max_hp_cap_permille / 1000
}

pub fn entry_max_hp_from_shadow_cloak_capacity(max_capacity: i32) -> i32 {
    max_capacity * 1000 / SHADOW_CLOAK_TUNINGS.max_hp_cap_permille
}

pub fn shadow_friend_hp_to_cloak_gain(loss: i32, max_hp: i32) -> i32 {
    (loss * SHADOW_CLOAK_TUNINGS.share_rate_permille / 1000).min(shadow_cloak_capacity(max_hp))
}

pub fn init_shadow_cloak_state(state: &mut ShadowCloakState, fight: &Fight) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if let Some(attacker) = &fight.attacker {
        for entity in &attacker.entitys {
            if is_rubuska(entity.model_id) {
                state.rubuska_uid = entity.uid.unwrap_or(0);
                state.rubuska_entry_max_hp =
                    entity.attr.as_ref().and_then(|attr| attr.hp).unwrap_or(0);
            }
        }
    }

    let seeded_max = SEEDED_RASPBERRY_MAX
        .lock()
        .ok()
        .and_then(|seeded| seeded.get(&battle_id).copied())
        .unwrap_or(0);
    if seeded_max > 0 {
        apply_seeded_shadow_cloak_capacity(state, seeded_max);
    } else {
        state.raspberry_max = shadow_cloak_capacity(state.rubuska_entry_max_hp);
    }
}

pub fn apply_seeded_shadow_cloak_capacity(state: &mut ShadowCloakState, max_capacity: i32) {
    state.raspberry_max = max_capacity;
    state.rubuska_entry_max_hp = entry_max_hp_from_shadow_cloak_capacity(max_capacity);
}

pub fn buff_is_shadow_cloak_accumulator(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|buff| buff.id == buff_id)
        .map(|buff| buff.type_id == SHADOW_CLOAK_ACCUMULATOR_BUFF_ID)
        .unwrap_or(false)
}

pub fn uses_shadow_cloak_overflow_tracker(model_id: Option<i32>, buff_id: i32) -> bool {
    is_rubuska(model_id) && buff_id == SHADOW_CLOAK_OVERFLOW_TRACKER_BUFF_ID
}

pub fn build_shadow_cloak_full_cap_step(
    ctx: &FightContext<'_>,
    raspberry_step: &FightStep,
) -> Option<FightStep> {
    let mut rubuska_uid = ctx.mechanics.shadow_cloak.rubuska_uid;
    if rubuska_uid <= 0 {
        rubuska_uid = raspberry_step
            .act_effect
            .iter()
            .filter_map(|effect| effect.fight_step.as_ref())
            .map(|step| step.from_id.unwrap_or(0))
            .find(|uid| *uid > 0)
            .unwrap_or(0);
    }

    let max_capacity = if ctx.mechanics.shadow_cloak.raspberry_max > 0 {
        ctx.mechanics.shadow_cloak.raspberry_max
    } else {
        shadow_cloak_capacity(
            find_entity(ctx.fight, rubuska_uid)
                .and_then(|entity| entity.attr.as_ref().and_then(|attr| attr.hp))
                .unwrap_or(0),
        )
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
            .filter(|effect| effect.effect_type == Some(2) || effect.effect_type == Some(3))
            .map(|effect| effect.effect_num.unwrap_or(0).max(0))
            .sum::<i32>();
        if damage <= 0 {
            continue;
        }

        total_shadow_gain += damage * SHADOW_CLOAK_TUNINGS.share_rate_permille / 1000;
        targets.insert(target_uid);
    }

    if total_shadow_gain < max_capacity || targets.is_empty() {
        return None;
    }

    SHADOW_CLOAK_FULL_CAP_GRANTED
        .lock()
        .unwrap()
        .insert(tracker_key);

    let effects = (0..targets.len())
        .map(|_| moxie_change(rubuska_uid, 1))
        .collect();
    Some(build_effect_step(effects))
}

pub fn is_basic_self_loss(skill_id: i32) -> bool {
    BASIC_SELF_LOSS_SKILLS
        .get_or_init(|| {
            let cfg = config::configs::get();
            let rubuska_id = HeroId::Rubuska.model_id();
            let mut out = HashSet::new();
            for skill in cfg.skill.iter() {
                if source_kind::owning_hero(skill.id) != Some(rubuska_id) {
                    continue;
                }
                let Some(effect) = cfg
                    .skill_effect
                    .iter()
                    .find(|effect| effect.id == skill.skill_effect)
                else {
                    continue;
                };
                let has_self_loss_basis = [
                    effect.behavior1.as_str(),
                    effect.behavior2.as_str(),
                    effect.behavior3.as_str(),
                    effect.behavior4.as_str(),
                    effect.behavior5.as_str(),
                ]
                .iter()
                .any(|behavior| behavior.starts_with("30006#0#100#"));
                if has_self_loss_basis {
                    out.insert(skill.id);
                }
            }
            out
        })
        .contains(&skill_id)
}

pub fn basic_self_loss_amount(fight: &Fight, target: i64, permille: i32) -> i32 {
    lookup_entry_max_hp(fight, target).max(0) * permille / 1000
}

// Consumed by battle_gen's replay bootstrap; clippy can't see cross-crate callers.
#[allow(dead_code)]
pub fn seed_replay_raspberry_max(fight: &Fight, max_capacity: i32) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 || max_capacity <= 0 {
        return;
    }
    if let Ok(mut seeded) = SEEDED_RASPBERRY_MAX.lock() {
        seeded.insert(battle_id, max_capacity);
    }
}
