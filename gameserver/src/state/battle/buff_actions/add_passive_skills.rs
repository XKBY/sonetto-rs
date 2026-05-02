//! Handler for buff_act 865 AddPassiveSkills.
//!
//! This feature adds virtual passive skill ids while the source buff remains
//! active. Runtime consumers use it to expand effective passive-skill sets and
//! to infer follow-up/precast behavior from those injected passives.

use sonettobuf::{ActEffect, Fight};

use crate::state::battle::context::FightContext;
use crate::state::battle::types::effects::EffectType;

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 865;

pub fn for_each_add_passive_skill_id(buff_id: i32, mut f: impl FnMut(i32)) {
    crate::state::battle::utils::for_each_buff_feature_chain(buff_id, |act_type, parts| {
        if act_type != "AddPassiveSkills" {
            return;
        }
        for raw in parts.iter().skip(1) {
            for piece in raw.split(',') {
                let Ok(skill_id) = piece.trim().parse::<i32>() else {
                    continue;
                };
                if skill_id > 0 {
                    f(skill_id);
                }
            }
        }
    });
}

pub fn for_each_add_passive_skill_id_for_entity(
    fight: &Fight,
    owner_uid: i64,
    buff_id: i32,
    mut f: impl FnMut(i32),
) {
    for_each_add_passive_skill_id(buff_id, |skill_id| {
        let resolved = crate::state::battle::skill::euphoria::resolve_with_euphoria(
            fight, owner_uid, skill_id,
        );
        f(resolved);
    });
}

/// Walk a recursive effect tree, find every `BuffAdd` that targets
/// `target_uid`, and accumulate the `AddPassiveSkills` grants those
/// buffs carry. `skip_skill_id` filters out the host skill itself
/// (so it doesn't get re-fired as its own followup).
///
/// This is the "what passives chain off the buffs my skill just
/// emitted" lookup. Used by the magic-circle aura embedder to find
/// the trailing reactive set after a `selfSkills` cast.
pub fn collect_grants_from_emitted_buffs(
    fight: &Fight,
    effects: &[ActEffect],
    target_uid: i64,
    skip_skill_id: i32,
) -> Vec<i32> {
    fn walk(effects: &[ActEffect], out: &mut Vec<(i64, i32)>) {
        for effect in effects {
            if effect.effect_type == Some(EffectType::BuffAdd as i32)
                && let (Some(target), Some(buff_id)) = (effect.target_id, effect.effect_num)
            {
                out.push((target, buff_id));
            }
            if let Some(step) = effect.fight_step.as_ref() {
                walk(&step.act_effect, out);
            }
        }
    }
    let mut added_buffs = Vec::new();
    walk(effects, &mut added_buffs);

    let mut out = Vec::new();
    for (added_target_uid, buff_id) in added_buffs {
        if added_target_uid != target_uid || buff_id <= 0 {
            continue;
        }
        for_each_add_passive_skill_id_for_entity(fight, target_uid, buff_id, |skill_id| {
            if skill_id != skip_skill_id && !out.contains(&skill_id) {
                out.push(skill_id);
            }
        });
    }
    out
}

/// Walk every active buff currently on `host_uid` whose `from_uid`
/// satisfies `source_filter`, harvesting their `AddPassiveSkills`
/// grants into `out`.
///
/// The `source_filter` is the safety net that keeps the walker from
/// snagging unrelated host buffs. The magic-circle caller passes
/// the circle creator's uid so only buffs the creator sourced (the
/// circle's selfBuff and anything it chains into) get walked —
/// host-private state like channel buffs is left alone.
pub fn collect_grants_from_active_buffs(
    ctx: &FightContext<'_>,
    host_uid: i64,
    source_filter: impl Fn(i64) -> bool,
    skip_skill_id: i32,
    out: &mut Vec<i32>,
) {
    for instance in ctx.managers.buff_mgr.get(host_uid) {
        if !source_filter(instance.from_uid) {
            continue;
        }
        for_each_add_passive_skill_id_for_entity(
            ctx.fight,
            host_uid,
            instance.buff_id,
            |skill_id| {
                if skill_id != skip_skill_id && !out.contains(&skill_id) {
                    out.push(skill_id);
                }
            },
        );
    }
}
