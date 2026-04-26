//! DOT (damage-over-time) round-end settlement.
//!
//! At the end of each round, every active poison-family buff on every alive
//! entity emits a single 162-wrapped fightStep:
//!
//! ```text
//! ActType::Effect, actId=buff_id, fromId=caster_uid, toId=victim_uid
//!   actEffect:
//!     effectType=213/255   (Poison / DeadlyPoison marker; effectNum = buff_id)
//!     effectType=130       (OriginDamage; effectNum = caster.atk * permille / 1000)
//! ```
//!
//! All inner wrappers for the round are gathered into one outer empty
//! container fightStep — see LIVE battle3 r2 step[17], r3 step[16].
//!
//! The settlement scans each entity's buff list for `buff_act` rows whose
//! `type` matches `Poison` (id 803) or `DeadlyPoison` (id 844). The
//! per-tick damage is `caster.atk × permille / 1000` where `permille`
//! is `parts[1]` of the feature entry (e.g. `803#200#0` → 20% atk per
//! stack). Multiple stacks (`BuffInstance::layer`) emit one inner
//! fightStep each.
//!
//! `LockPoison` (id 810) does NOT itself tick — buff 30980131's feature
//! is the bare token `810`, and it only flags the target as
//! lock-poisoned for downstream cleanse-resistance checks. Lock-poison
//! BUFFs that DO tick (e.g. 30980111) carry both `810` and `803#…`
//! features and surface here through the `803` entry.
//!
//! Crit emissions (effectType=131 OriginCrit) are NOT replicated here
//! — they require deterministic caster crit-rate rolling that the
//! replay path can't reproduce without LIVE-side RNG. The result is
//! all DOT damage emits as et=130; this still closes most of the
//! battle3 step-count gap because every tick adds one wrapper to the
//! parent container regardless of crit/non-crit shape.

use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::{
    context::FightContext,
    fight_step::{ActEffectBuilder, effect_container_step, wrap_step},
    round::step_shape::build_effect_step,
    skill::get_entity,
    types::effects::EffectType,
};

/// Build the round-end DOT settlement step (one outer container holding
/// every poison-family tick across alive entities), or `None` if no
/// stacks tick this round.
pub fn build_round_end_dot_step(ctx: &FightContext<'_>) -> Option<FightStep> {
    let mut wrappers: Vec<ActEffect> = Vec::new();

    for victim_uid in iter_alive_uids(ctx.fight) {
        let buffs = ctx.managers.buff_mgr.get(victim_uid).to_vec();
        for instance in buffs {
            let Some((marker_et, permille)) = parse_dot_features(instance.buff_id) else {
                continue;
            };
            let Some(caster) = get_entity(ctx.fight, instance.from_uid) else {
                continue;
            };
            let caster_atk = caster
                .attr
                .as_ref()
                .and_then(|a| a.attack)
                .unwrap_or(0);
            if caster_atk <= 0 {
                continue;
            }
            let damage = caster_atk * permille / 1000;
            if damage <= 0 {
                continue;
            }

            let stacks = instance.layer.max(1);
            for _ in 0..stacks {
                let inner = effect_container_step(
                    instance.from_uid,
                    victim_uid,
                    instance.buff_id,
                    vec![
                        ActEffectBuilder::new(marker_et, victim_uid)
                            .effect_num(instance.buff_id)
                            .build(),
                        ActEffectBuilder::new(EffectType::OriginDamage as i32, victim_uid)
                            .effect_num(damage)
                            .build(),
                    ],
                );
                wrappers.push(wrap_step(inner));
            }
        }
    }

    if wrappers.is_empty() {
        None
    } else {
        Some(build_effect_step(wrappers))
    }
}

/// Iterate every alive entity uid in `attacker.entitys + sub_entitys` then
/// `defender.entitys + sub_entitys` order — matching the LIVE settlement
/// emission order (allies first, then defenders).
fn iter_alive_uids(fight: &sonettobuf::Fight) -> Vec<i64> {
    let mut uids = Vec::new();
    if let Some(side) = &fight.attacker {
        for e in side.entitys.iter().chain(side.sub_entitys.iter()) {
            if e.current_hp.unwrap_or(0) > 0
                && let Some(uid) = e.uid
            {
                uids.push(uid);
            }
        }
    }
    if let Some(side) = &fight.defender {
        for e in side.entitys.iter().chain(side.sub_entitys.iter()) {
            if e.current_hp.unwrap_or(0) > 0
                && let Some(uid) = e.uid
            {
                uids.push(uid);
            }
        }
    }
    uids
}

/// Returns `(marker_effect_type, permille)` if the buff carries a
/// poison-family DOT feature, else `None`.
///
/// Matches:
/// - `Poison` (act 803) → `(EffectType::Poison, parts[1])`
/// - `DeadlyPoison` (act 844) → `(EffectType::DeadlyPoison, parts[1])`
///
/// Skips `LockPoison` (act 810) — see module docs.
fn parse_dot_features(buff_id: i32) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    if buff.features.is_empty() {
        return None;
    }
    for entry in buff.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let act_type = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type.as_str())?;
        match act_type {
            "Poison" => {
                let permille: i32 = parts.get(1)?.trim().parse().ok()?;
                if permille > 0 {
                    return Some((EffectType::Poison as i32, permille));
                }
            }
            "DeadlyPoison" => {
                let permille: i32 = parts.get(1)?.trim().parse().ok()?;
                if permille > 0 {
                    return Some((EffectType::DeadlyPoison as i32, permille));
                }
            }
            _ => continue,
        }
    }
    None
}
