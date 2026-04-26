use super::super::super::types::condition::ConditionType;
use super::ConditionEval;
use super::action::Condition;

pub(super) struct Bloodtithe;

impl Condition for Bloodtithe {
    fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        match cond_type {
            "BloodPoolMax" => Some(ConditionType::BloodPoolMax {
                min: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                max: parts
                    .get(2)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(i32::MAX),
            }),
            "BloodPool" => Some(ConditionType::BloodPool),
            _ => None,
        }
    }

    fn check(condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        match condition {
            ConditionType::BloodPoolMax { min, max } => {
                // 1 = attacker side
                let pool_max = ctx.bloodtithe.get_max(1);
                Some(pool_max >= *min && pool_max <= *max)
            }
            ConditionType::BloodPool => Some(ctx.bloodtithe.has_bloodpool()),
            _ => None,
        }
    }
}
