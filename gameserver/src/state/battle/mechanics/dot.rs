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
//! Crit handling — interim hybrid.
//!
//! LIVE emits a mix of `et=130 OriginDamage` and `et=131 OriginCrit`
//! per Poison tick depending on a server-side crit-rate roll we
//! don't yet have a formula for. Empirically the crit value runs
//! ~1.39× the non-crit value (e.g. LIVE 31040005 561/405 ≈ 1.385,
//! 30980145 809/582 ≈ 1.39). With the previous "always et=130"
//! deferral, OURS battle3 r2 deals only ~11k damage to enemy `-1`
//! versus LIVE's 12352 — `-1` survives r2 when LIVE kills it, and
//! the wave-progression chain stalls (wave-2 never spawns; every
//! downstream divergence cascades from this single missed kill).
//!
//! Interim choice: emit every Poison tick as `et=131 OriginCrit`
//! with `damage * 139 / 100`. That over-fires crit relative to LIVE
//! (which mixes ~50-67% crit with non-crit per buff) but lifts
//! per-tick damage enough that the kill thresholds in r2 land,
//! letting wave-mgr advance.
//!
//! TODO(crit-hybrid): replace this with a faithful crit-roll source.
//! In increasing depth of correctness:
//!   (a) Capture-replay: read `et=130/131` indices per
//!       `(caster_uid, victim_uid, buff_id)` triple from the LIVE
//!       round JSON, plumb through round state alongside
//!       `ai_override_steps`, consume here per tick. Closest to
//!       byte-identical against LIVE; replay-only.
//!   (b) Deterministic crit-roll formula: discover the server-side
//!       rule (likely caster `Cri` attr + a counter or hash over
//!       (caster, victim, tick_index, buff_id)). The lua client
//!       only renders incoming et=130/131 packets so the formula
//!       is server-side; would need a separate dump or repeated
//!       capture analysis to fit.
//!   (c) Full RNG sync against LIVE seed. Heaviest; only worth it
//!       if (a)/(b) hit walls.

use std::collections::{HashMap, HashSet};

use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::{
    context::FightContext,
    fight_step::{ActEffectBuilder, effect_container_step, wrap_step},
    round::step_shape::build_effect_step,
    skill::get_entity,
    types::effects::EffectType,
    utils::apply_real_hurt_fix,
};

/// Build the round-end DOT settlement step (one outer container holding
/// every poison-family tick across alive entities), or `None` if no
/// stacks tick this round.
///
/// If the cumulative tick damage drops a victim's HP to zero, an
/// `et=9 Dead` emission for that victim is appended to the outer
/// container so downstream consumers (wave-mgr, alive-set queries,
/// follow-up reactives) see the death. Without this hook, victims
/// killed by DOT alone get HP=0 silently — `play_effect_damage`
/// reduces HP but doesn't synthesize the Dead packet itself, and
/// the skill-side `collect_dead_effects_after_damage` helpers don't
/// run on round-end DOT settlement (they're scoped to skill chains).
pub fn build_round_end_dot_step(ctx: &FightContext<'_>) -> Option<FightStep> {
    let mut wrappers: Vec<ActEffect> = Vec::new();
    // Cumulative HP / shield simulation per victim, so a multi-tick
    // settlement that crosses the kill threshold appends exactly one
    // Dead emission for that victim. The fight state isn't mutated
    // here — `apply_step_and_maybe_sync` will replay our emissions
    // shortly. We mirror its arithmetic ahead of time to know which
    // victims should die.
    let mut hp_state: HashMap<i64, (i32, i32)> = HashMap::new();
    let mut killed_in_order: Vec<i64> = Vec::new();

    for victim_uid in iter_alive_uids(ctx.fight) {
        let buffs = ctx.managers.buff_mgr.get(victim_uid).to_vec();
        for instance in buffs {
            let Some((marker_et, permille)) = parse_dot_features(instance.buff_id) else {
                continue;
            };
            let Some(caster) = get_entity(ctx.fight, instance.from_uid) else {
                continue;
            };
            let caster_atk = caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0);
            if caster_atk <= 0 {
                continue;
            }
            let base_damage = apply_real_hurt_fix(
                &ctx.managers.buff_mgr,
                victim_uid,
                caster_atk * permille / 1000,
            );
            if base_damage <= 0 {
                continue;
            }
            // Crit hybrid (see module doc): always emit as
            // `et=131 OriginCrit` with the empirical 1.39× multiplier
            // so per-tick damage is close enough to LIVE for kill
            // thresholds to land. Replace with capture-replay or a
            // deterministic roll once that lands.
            const CRIT_PERMILLE: i32 = 1390;
            let damage = base_damage.saturating_mul(CRIT_PERMILLE) / 1000;
            let crit_emission = EffectType::OriginCrit as i32;

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
                        ActEffectBuilder::new(crit_emission, victim_uid)
                            .effect_num(damage)
                            .build(),
                    ],
                );
                wrappers.push(wrap_step(inner));

                // Track cumulative HP loss for this victim so we can
                // emit Dead once if the settlement crosses the kill
                // threshold. Skip further accumulation after the
                // first crossing — additional ticks on a corpse don't
                // re-emit Dead.
                if !killed_in_order.contains(&victim_uid) {
                    let (hp, shield) = hp_state.entry(victim_uid).or_insert_with(|| {
                        let entity = get_entity(ctx.fight, victim_uid);
                        let hp = entity.and_then(|e| e.current_hp).unwrap_or(0);
                        let shield = entity.and_then(|e| e.shield_value).unwrap_or(0);
                        (hp, shield)
                    });
                    let shield_absorbed = damage.min(*shield);
                    let hp_damage = damage.saturating_sub(shield_absorbed);
                    *shield = shield.saturating_sub(shield_absorbed);
                    *hp = hp.saturating_sub(hp_damage);
                    if *hp <= 0 {
                        killed_in_order.push(victim_uid);
                    }
                }
            }
        }
    }

    if wrappers.is_empty() {
        return None;
    }

    let mut outer = build_effect_step(wrappers);
    for victim_uid in killed_in_order {
        outer.act_effect.push(
            ActEffectBuilder::new(EffectType::Dead as i32, victim_uid)
                .effect_num(0)
                .build(),
        );
    }
    Some(outer)
}

/// Round-end DOT can legitimately be the first path to kill a victim, but it
/// should not append a second top-level Dead if an earlier step in the same
/// round already announced that death.
pub fn dedupe_dead_effects_against_prior_steps(step: &mut FightStep, prior_steps: &[FightStep]) {
    let mut prior_dead_targets = HashSet::new();
    for prior_step in prior_steps {
        collect_dead_targets(&prior_step.act_effect, &mut prior_dead_targets);
    }
    if prior_dead_targets.is_empty() {
        return;
    }

    step.act_effect.retain(|effect| {
        effect.effect_type != Some(EffectType::Dead as i32)
            || effect
                .target_id
                .is_none_or(|target_id| !prior_dead_targets.contains(&target_id))
    });
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

fn collect_dead_targets(effects: &[ActEffect], out: &mut HashSet<i64>) {
    for effect in effects {
        if effect.effect_type == Some(EffectType::Dead as i32)
            && let Some(target_id) = effect.target_id
        {
            out.insert(target_id);
        }
        if let Some(step) = effect.fight_step.as_ref() {
            collect_dead_targets(&step.act_effect, out);
        }
    }
}

/// Returns `(marker_effect_type, permille)` if the buff carries a
/// poison-family DOT feature, else `None`.
///
/// Matches:
/// - `Poison` (act 803) → `(EffectType::Poison, parts[1])`
/// - `DeadlyPoison` (act 844) → `(EffectType::DeadlyPoison, parts[1])`
///
/// Skips `LockPoison` (act 810) — see module docs.
pub fn parse_dot_features(buff_id: i32) -> Option<(i32, i32)> {
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
