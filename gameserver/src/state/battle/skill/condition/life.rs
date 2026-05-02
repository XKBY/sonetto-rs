use crate::state::battle::skill::targets::get_entity;

use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;

pub(super) struct Life;

impl Condition for Life {
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        match cond_type {
            "LifeLess" => Some(ConditionType::LifeLess {
                threshold_permille: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "LifeMore" => Some(ConditionType::LifeMore {
                threshold_permille: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "MultiHpXIn" => Some(ConditionType::MultiHpXIn),
            _ => None,
        }
    }

    fn check(&self, condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        let condition_uid = ctx.resolve_entity_target_uid();
        match condition {
            ConditionType::LifeLess { threshold_permille } => Some(
                get_entity(ctx.fight, condition_uid)
                    .map(|e| {
                        let cur = e.current_hp.unwrap_or(0) as f32;
                        let max = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(1) as f32;
                        cur / max < (*threshold_permille as f32 / 1000.0)
                    })
                    .unwrap_or(false),
            ),
            ConditionType::LifeMore { threshold_permille } => Some(
                get_entity(ctx.fight, condition_uid)
                    .map(|e| {
                        let cur = e.current_hp.unwrap_or(0) as f32;
                        let max = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(1) as f32;
                        cur / max > (*threshold_permille as f32 / 1000.0)
                    })
                    .unwrap_or(false),
            ),
            ConditionType::MultiHpXIn => Some(false), // combat only
            _ => None,
        }
    }
}
