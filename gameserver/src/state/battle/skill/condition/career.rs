use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;
use crate::state::battle::skill::targets::get_entity;

pub(super) struct Career;

impl Condition for Career {
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType> {
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
                    subtype_id: parts.first().and_then(|v| v.parse().ok()).unwrap_or(0),
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

    fn check(&self, condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        let condition_uid = ctx.resolve_entity_target_uid();
        match condition {
            ConditionType::TargetCareer { career_ids } => {
                let career = get_entity(ctx.fight, condition_uid).and_then(|e| e.career);
                Some(career.map(|c| career_ids.contains(&c)).unwrap_or(false))
            }
            ConditionType::CareerCheck {
                subtype_id: _,
                param,
            } => {
                // CareerCheck is evaluated against the resolved condition target.
                // For example, Pickles 30630151 uses conditionTarget=128 (adjacent ally).
                let career = get_entity(ctx.fight, condition_uid)
                    .and_then(|e| e.career)
                    .unwrap_or(-1);
                let is_mineral = career == 1;
                Some(if *param == 0 { is_mineral } else { !is_mineral })
            }
            ConditionType::PerHasTargetCareerList { .. } => Some(false), // combat only
            _ => None,
        }
    }
}
