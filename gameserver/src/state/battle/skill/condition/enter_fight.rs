use crate::state::battle::mechanics::bloodtithe::BloodtitheState;

use super::super::super::manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr};
use super::ConditionType;

pub fn parse(id: i32, cond_type: &str) -> Option<ConditionType> {
    match cond_type {
        "EnterFight" => Some(ConditionType::EnterFight { condition_id: id }),
        "None" => match id {
            // These None conditions are combat triggers, not battle-start passives
            210 => Some(ConditionType::CombatNone),
            _ => Some(ConditionType::None),
        },
        "" => match id {
            5 | 5021 | 6 => Some(ConditionType::EnterFight { condition_id: id }),
            _ => None,
        },
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn check(
    condition: &ConditionType,
    fight: &sonettobuf::Fight,
    buff_mgr: &BuffMgr,
    ex_point_mgr: &ExPointMgr,
    bloodtithe: &BloodtitheState,
    caster_uid: i64,
    target_uid: i64,
    has_trigger_state: bool,
) -> Option<bool> {
    match condition {
        ConditionType::None | ConditionType::CombatNone | ConditionType::EnterFight { .. } => {
            Some(true)
        }
        ConditionType::EnterFightAnd(_) | ConditionType::EnterFightOr(_) => {
            Some(super::fold(condition, &mut |cond| {
                super::check_condition(
                    fight,
                    buff_mgr,
                    ex_point_mgr,
                    bloodtithe,
                    caster_uid,
                    target_uid,
                    has_trigger_state,
                    cond,
                )
            }))
        }
        _ => None,
    }
}
