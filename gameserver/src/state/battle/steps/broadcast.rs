use sonettobuf::ActEffect;

use crate::state::battle::context::FightContext;

/// Collects BuffUpdate (effectType 7) ActEffects for every alive frontline
/// entity's buffs whose remaining duration is exactly 1 tick — i.e. the
/// buffs that will expire on the round-end tick. The live game emits these
/// as a snapshot either before SmallRoundEnd (defender scope) or after
/// ChangeRound (attacker scope). Entities are visited in fight entity order
/// and per-entity buff order matches the manager's insertion order.
pub(crate) fn collect_buff_tick_broadcast(
    ctx: &FightContext<'_>,
    attacker_side: bool,
) -> Vec<ActEffect> {
    let side = if attacker_side {
        ctx.fight.attacker.as_ref()
    } else {
        ctx.fight.defender.as_ref()
    };
    let Some(side) = side else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entity in side.entitys.iter().chain(side.sub_entitys.iter()) {
        if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
            continue;
        }
        let Some(uid) = entity.uid else { continue };
        for buff in ctx.managers.buff_mgr.get(uid) {
            if buff.duration != 1 {
                continue;
            }
            let mut effect = crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                uid,
                buff.from_uid,
                buff.buff_id,
                buff.uid,
                buff.stacks,
                buff.layer,
            );
            if let Some(info) = effect.buff.as_mut() {
                info.duration = Some(1);
            }
            out.push(effect);
        }
    }
    out
}

pub(crate) fn filter_round_end_broadcast_by_source_side(
    broadcast: Vec<ActEffect>,
    attacker_side: bool,
) -> Vec<ActEffect> {
    let cfg = config::configs::get();
    broadcast
        .into_iter()
        .filter(|effect| {
            let target_uid = effect.target_id.unwrap_or(0);
            let from_uid = effect.buff.as_ref().and_then(|b| b.from_uid).unwrap_or(0);

            if target_uid == 0 || from_uid == 0 {
                return true;
            }

            let target_on_attacker = target_uid > 0;
            let from_on_attacker = from_uid > 0;
            if attacker_side {
                target_on_attacker && from_on_attacker
            } else {
                !target_on_attacker && !from_on_attacker
            }
        })
        .map(|mut effect| {
            let Some(buff) = effect.buff.as_mut() else {
                return effect;
            };
            let buff_id = buff.buff_id.unwrap_or(0);
            if buff_id == 0 {
                return effect;
            }
            if cfg
                .skill_buff
                .iter()
                .find(|b| b.id == buff_id)
                .map(|b| b.effect_count == 0)
                .unwrap_or(false)
            {
                buff.count = Some(0);
            }
            effect
        })
        .collect()
}

pub(crate) fn adjust_defender_round1_broadcast_uids(
    ctx: &FightContext<'_>,
    broadcast: &mut [ActEffect],
) {
    if ctx.fight.cur_round.unwrap_or(1) != 1 {
        return;
    }
    for effect in broadcast {
        let Some(buff) = effect.buff.as_mut() else {
            continue;
        };
        let Some(uid) = buff.uid else {
            continue;
        };
        if uid >= 100_000 {
            buff.uid = Some(uid + 12);
        }
    }
}

pub(crate) fn adjust_attacker_round1_broadcast_uids(broadcast: &mut [ActEffect]) {
    for effect in broadcast {
        let Some(buff) = effect.buff.as_mut() else {
            continue;
        };
        let Some(uid) = buff.uid else {
            continue;
        };
        if uid <= 0 {
            continue;
        }
        let delta = if uid % 2 == 0 { 16 } else { 17 };
        buff.uid = Some(uid + delta);
    }
}


