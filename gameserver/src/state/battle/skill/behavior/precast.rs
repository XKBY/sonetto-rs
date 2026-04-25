use sonettobuf::Fight;

use crate::state::battle::{
    buff_actions::add_passive_skills::for_each_add_passive_skill_id_for_entity,
    manager::fight_data_mgr::Managers,
    skill::{
        cache::SKILL_CACHE, euphoria::resolve_skill_effect_id_for_entity, targets::get_entity,
    },
    types::{behavior::BehaviorType, condition::ConditionType},
};

fn find_self_buff_prep_skills(passive_skills: &[i32]) -> Vec<i32> {
    let cfg = config::configs::get();
    let mut out: Vec<i32> = Vec::new();
    for &sid in passive_skills {
        if sid <= 0 {
            continue;
        }
        // Some rows are directly a "self-buff prep skill" entry.
        if let Some(row) = cfg.skill_effect.iter().find(|s| s.id == sid) {
            let expect_behavior = format!("1#{}", sid);
            if row.condition1.starts_with("660008#1")
                && row.behavior1 == expect_behavior
                && !out.contains(&sid)
            {
                out.push(sid);
            }
        }
        let mut best_for_sid: Option<i32> = None;
        for delta in 1..=20 {
            let candidate = sid + delta;
            if let Some(row) = cfg.skill_effect.iter().find(|s| s.id == candidate) {
                let expect_behavior = format!("1#{}", candidate);
                if row.condition1.starts_with("660008#1") && row.behavior1 == expect_behavior {
                    best_for_sid = Some(best_for_sid.map_or(candidate, |cur| cur.max(candidate)));
                }
            }
        }
        if let Some(candidate) = best_for_sid
            && !out.contains(&candidate)
        {
            out.push(candidate);
        }
    }
    out
}

pub(crate) fn collect_precast_skills_for_caster(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
) -> Vec<i32> {
    let mut passive_candidates: Vec<i32> = get_entity(fight, caster_uid)
        .map(|e| e.passive_skill.clone())
        .unwrap_or_default();

    // Buff feature 865(AddPassiveSkills) contributes virtual passive skills while buff is active.
    for instance in managers.buff_mgr.get(caster_uid) {
        for_each_add_passive_skill_id_for_entity(fight, caster_uid, instance.buff_id, |skill_id| {
            if !passive_candidates.contains(&skill_id) {
                passive_candidates.push(skill_id);
            }
        });
    }

    find_self_buff_prep_skills(&passive_candidates)
}

pub(crate) fn infer_precast_per_decr_seed_cap(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
    prep_skill_ids: &[i32],
) -> Option<i32> {
    let active = managers.buff_mgr.get(caster_uid);
    let mut best: Option<i32> = None;

    for &skill_id in prep_skill_ids {
        let effect_id = resolve_skill_effect_id_for_entity(fight, caster_uid, skill_id);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else {
            continue;
        };
        for row in rows {
            let is_per_decr = matches!(row.condition, ConditionType::PerDecrExPoint { .. });
            if !is_per_decr {
                continue;
            }
            let BehaviorType::AddBuff { buff_id, .. } = row.behavior else {
                continue;
            };
            let cap = active
                .iter()
                .find(|b| b.buff_id == buff_id)
                .map(|b| b.layer.max(b.stacks))
                .unwrap_or(0);
            if cap <= 0 {
                // Some prep skills are injected by AddPassiveSkills features.
                // When no direct target buff exists yet, seed from the source
                // passive buff's current stack/layer value.
                let mut source_cap = 0;
                for source in active {
                    let mut matched = false;
                    for_each_add_passive_skill_id_for_entity(
                        fight,
                        caster_uid,
                        source.buff_id,
                        |sid| {
                            if !matched && sid == skill_id {
                                matched = true;
                            }
                        },
                    );
                    if matched {
                        source_cap = source.layer.max(source.stacks).max(0);
                        if source_cap > 0 {
                            break;
                        }
                    }
                }
                if source_cap <= 0 {
                    continue;
                }
                best = Some(best.map_or(source_cap, |v| v.min(source_cap)));
                continue;
            }
            best = Some(best.map_or(cap, |v| v.min(cap)));
        }
    }

    best
}
