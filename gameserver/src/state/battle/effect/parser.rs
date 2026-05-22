use super::{SkillEffect, condition};

pub fn parse(effect_id: i32, owner_uid: i64) -> Option<SkillEffect> {
    let cfg = config::configs::get();
    let row = cfg.skill_effect.iter().find(|e| e.id == effect_id)?;

    let slots = [
        (&row.condition1,  &row.condition_target1,  &row.behavior1,  &row.behavior_target1),
        (&row.condition2,  &row.condition_target2,  &row.behavior2,  &row.behavior_target2),
        (&row.condition3,  &row.condition_target3,  &row.behavior3,  &row.behavior_target3),
        (&row.condition4,  &row.condition_target4,  &row.behavior4,  &row.behavior_target4),
        (&row.condition5,  &row.condition_target5,  &row.behavior5,  &row.behavior_target5),
        (&row.condition6,  &row.condition_target6,  &row.behavior6,  &row.behavior_target6),
        (&row.condition7,  &row.condition_target7,  &row.behavior7,  &row.behavior_target7),
        (&row.condition8,  &row.condition_target8,  &row.behavior8,  &row.behavior_target8),
        (&row.condition9,  &row.condition_target9,  &row.behavior9,  &row.behavior_target9),
        (&row.condition10, &row.condition_target10, &row.behavior10, &row.behavior_target10),
        (&row.condition11, &row.condition_target11, &row.behavior11, &row.behavior_target11),
        (&row.condition12, &row.condition_target12, &row.behavior12, &row.behavior_target12),
        (&row.condition13, &row.condition_target13, &row.behavior13, &row.behavior_target13),
        (&row.condition14, &row.condition_target14, &row.behavior14, &row.behavior_target14),
        (&row.condition15, &row.condition_target15, &row.behavior15, &row.behavior_target15),
        (&row.condition16, &row.condition_target16, &row.behavior16, &row.behavior_target16),
        (&row.condition17, &row.condition_target17, &row.behavior17, &row.behavior_target17),
        (&row.condition18, &row.condition_target18, &row.behavior18, &row.behavior_target18),
        (&row.condition19, &row.condition_target19, &row.behavior19, &row.behavior_target19),
        (&row.condition20, &row.condition_target20, &row.behavior20, &row.behavior_target20),
    ];

    let behaviours = slots
        .iter()
        .filter_map(|(cond, cond_target, beh, beh_target)| {
            if beh.is_empty() { return None; }
            let cond_target_id: i32 = cond_target.parse().ok()?;
            let cond = condition::parse(cond, cond_target_id, owner_uid)?;
            let beh_target_id: i32 = beh_target.parse().unwrap_or(0);
            Some((cond, beh.to_string(), beh_target_id))
        })
        .collect();

    Some(SkillEffect { behaviours })
}
