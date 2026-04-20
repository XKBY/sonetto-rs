use super::ConditionType;
use crate::state::battle::manager::ex_point_mgr::ExPointMgr;

pub fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
    match cond_type {
        "PerExPoint" => Some(ConditionType::PerExPoint {
            threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "PerDecrExPoint" => Some(ConditionType::PerDecrExPoint {
            threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "ExpointMoreThan" => Some(ConditionType::ExpointMoreThan {
            threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "ExpointLessThan" => Some(ConditionType::ExpointLessThan {
            threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        _ => None,
    }
}

pub fn check(
    condition: &ConditionType,
    ex_point_mgr: &ExPointMgr,
    caster_uid: i64,
) -> Option<bool> {
    match condition {
        ConditionType::PerExPoint { threshold } => {
            Some(ex_point_mgr.get_ex_point(caster_uid) >= *threshold)
        }
        ConditionType::PerDecrExPoint { threshold } => {
            Some(ex_point_mgr.get_recent_decr_ex_point(caster_uid) >= *threshold)
        }
        ConditionType::ExpointMoreThan { threshold } => {
            Some(ex_point_mgr.get_ex_point(caster_uid) >= *threshold)
        }
        ConditionType::ExpointLessThan { threshold } => {
            Some(ex_point_mgr.get_ex_point(caster_uid) <= *threshold)
        }
        _ => None,
    }
}
