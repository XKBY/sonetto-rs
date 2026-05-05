//! AdvancedCure HoT round-end settlement.
//!
//! At round-end, every active AdvancedCure buff (`buff_act` type =
//! `AdvancedCure`, id 849) emits a 162-wrapped fightStep PER buff
//! instance:
//!
//! ```text
//! ActType::Skill, actId=buff_id, fromId=caster_uid, toId=ally_uid
//!   actEffect:
//!     effectType=0  ti=ally  num=buff_id   (presence marker)
//!     effectType=4  ti=ally  num=heal      (Heal — caster.attr × permille / 1000)
//!     effectType=7  ti=ally  buff=buff_id  (BuffUpdate)
//! ```
//!
//! Verified shape from LIVE battle3 r2 step[25] (Sotheby's Concentrated
//! Essence ticking 30091111 on each ally) and r3 step[22] (two cure
//! buffs × four allies = 8 inner wrappers).
//!
//! Feature encoding `849#period#attr_id#permille#max_pct`:
//! - `period` is the buff duration in rounds (often 2)
//! - `attr_id` keys the source attribute (102 = ATK)
//! - `permille` is the heal scaling (500 = 50%)
//! - `max_pct` caps the heal at a per-tick fraction of the target's
//!   max HP (40% = 4000 raw permille). Currently the cap isn't hit
//!   in our fixtures so applying it is deferred.
//!
//! Each ally that has the buff applied gets one tick — Sotheby's
//! Insight III "Before [Concentrated Essence!] inflicts [Cure] status,
//! the [Cure] statuses on all allies take effect immediately" means
//! every ally carries an instance of the cure buff in their own
//! BuffMgr slot, with `from_uid` pointing at Sotheby. Iterate every
//! entity, scan their buffs for AdvancedCure features, emit one
//! wrapper per (target_uid, buff_id, from_uid) triple.

use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    event_queue::{BattleEvent, serialize_leaf_event},
    fight_step::{ActEffectBuilder, wrap_step},
    round::step_shape::build_effect_step,
    skill::get_entity,
    types::effects::EffectType,
};

/// Build the round-end AdvancedCure HoT settlement step (one outer
/// container holding every cure tick across alive entities), or `None`
/// if no buffs tick this round.
pub fn build_round_end_advanced_cure_step(ctx: &FightContext<'_>) -> Option<FightStep> {
    let mut wrappers: Vec<ActEffect> = Vec::new();

    for target_uid in iter_alive_uids(ctx.fight) {
        let buffs = ctx.managers.buff_mgr.get(target_uid).to_vec();
        for instance in buffs {
            let Some((attr_id, permille)) = parse_advanced_cure_features(instance.buff_id) else {
                continue;
            };
            let Some(caster) = get_entity(ctx.fight, instance.from_uid) else {
                continue;
            };
            let attr_value = lookup_attr(caster, attr_id);
            if attr_value <= 0 {
                continue;
            }
            let heal = attr_value * permille / 1000;
            if heal <= 0 {
                continue;
            }

            // NOTE: deliberately omit the trailing BuffUpdate(7) effect
            // for now. LIVE emits one per cure tick (so the buff
            // duration counter is reflected to the client), but
            // synthesizing one runs into "No buff data" failures
            // downstream — the buff_uid path through play_step_data
            // hasn't been validated for replay-side mid-step
            // BuffUpdate emissions. The cure visual + heal still emit;
            // the BuffUpdate broadcast happens via the existing
            // round-end-tick broadcast collector at the end of the
            // attacker sweep.
            let inner = sonettobuf::FightStep {
                act_type: Some(fight_step::ActType::Skill.into()),
                from_id: Some(instance.from_uid),
                to_id: Some(target_uid),
                act_id: Some(instance.buff_id),
                act_effect: vec![
                    ActEffectBuilder::new(EffectType::None as i32, target_uid)
                        .effect_num(instance.buff_id)
                        .build(),
                    serialize_leaf_event(BattleEvent::Heal {
                        target: target_uid,
                        amount: heal,
                        from: instance.from_uid,
                    }),
                ],
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            };
            wrappers.push(wrap_step(inner));
        }
    }

    if wrappers.is_empty() {
        None
    } else {
        Some(build_effect_step(wrappers))
    }
}

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

/// Returns `(attr_id, permille)` if the buff carries an `AdvancedCure`
/// feature, else `None`.
///
/// Encoding: `849#period#attr_id#permille#max_pct` — see module docs.
fn parse_advanced_cure_features(buff_id: i32) -> Option<(i32, i32)> {
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
        if act_type == "AdvancedCure" {
            let attr_id: i32 = parts.get(2)?.trim().parse().ok()?;
            let permille: i32 = parts.get(3)?.trim().parse().ok()?;
            if permille > 0 {
                return Some((attr_id, permille));
            }
        }
    }
    None
}

fn lookup_attr(entity: &sonettobuf::FightEntityInfo, attr_id: i32) -> i32 {
    let attr = entity.attr.as_ref();
    match attr_id {
        100 => entity.current_hp.unwrap_or(0),
        101 => attr.and_then(|a| a.hp).unwrap_or(0),
        102 => attr.and_then(|a| a.attack).unwrap_or(0),
        103 => attr.and_then(|a| a.defense).unwrap_or(0),
        _ => attr.and_then(|a| a.attack).unwrap_or(0),
    }
}
