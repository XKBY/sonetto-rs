use super::ConditionType;
use crate::state::battle::skill::targets::get_entity;
use sonettobuf::Fight;

pub fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
    match cond_type {
        "TargetCareer" => {
            let ids = parts[1..]
                .iter()
                .flat_map(|p| p.split('#'))
                .filter_map(|v| v.parse().ok())
                .collect();
            Some(ConditionType::TargetCareer { career_ids: ids })
        }
        "CareerCheck" => Some({
            ConditionType::CareerCheck {
                param: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }
        }),
        "PerHasTargetCareerList" => Some(ConditionType::PerHasTargetCareerList {
            careers: parts
                .get(1)
                .map(|p| p.split(',').filter_map(|v| v.parse().ok()).collect())
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

pub fn check(
    condition: &ConditionType,
    fight: &Fight,
    _caster_uid: i64,
    target_uid: i64,
) -> Option<bool> {
    match condition {
        ConditionType::TargetCareer { career_ids } => {
            let career = get_entity(fight, target_uid).and_then(|e| e.career);
            Some(career.map(|c| career_ids.contains(&c)).unwrap_or(false))
        }
        ConditionType::CareerCheck { param } => {
            // CareerCheck is evaluated against the resolved condition target.
            // For example, Pickles 30630151 uses conditionTarget=128 (adjacent ally).
            let career = get_entity(fight, target_uid)
                .and_then(|e| e.career)
                .unwrap_or(-1);
            let is_mineral = career == 1;
            Some(if *param == 0 { is_mineral } else { !is_mineral })
        }
        ConditionType::PerHasTargetCareerList { .. } => Some(false), // combat only
        _ => None,
    }
}
