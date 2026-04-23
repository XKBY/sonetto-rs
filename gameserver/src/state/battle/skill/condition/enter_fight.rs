use crate::state::battle::mechanics::bloodtithe::BloodtitheState;

use super::super::super::manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr};
use super::ConditionType;

pub fn parse(id: i32, cond_type: &str) -> Option<ConditionType> {
    // ID 6 is tagged `type=None` in config but is semantically the
    // "Unconditional battle-start" gate (not a combat always-pass). Treat it
    // as EnterFight here so Combat-phase passes correctly reject it; use 210
    // for the true combat-None gate.
    if matches!(id, 5 | 5021 | 6) {
        return Some(ConditionType::EnterFight { condition_id: id });
    }
    match cond_type {
        "EnterFight" => Some(ConditionType::EnterFight { condition_id: id }),
        "None" => match id {
            // These None conditions are combat triggers, not battle-start passives
            210 => Some(ConditionType::CombatNone),
            _ => Some(ConditionType::None),
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
