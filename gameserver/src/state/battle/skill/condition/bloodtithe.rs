use super::super::super::types::condition::ConditionType;
use crate::state::battle::mechanics::bloodtithe::BloodtitheState;

pub fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
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

pub fn check(condition: &ConditionType, bloodtithe: &BloodtitheState) -> Option<bool> {
    match condition {
        ConditionType::BloodPoolMax { min, max } => {
            let pool_max = bloodtithe.get_max(1); // 1 = attacker side
            Some(pool_max >= *min && pool_max <= *max)
        }
        ConditionType::BloodPool => Some(bloodtithe.has_bloodpool()),
        _ => None,
    }
}
