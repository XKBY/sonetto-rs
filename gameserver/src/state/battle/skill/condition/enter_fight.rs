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
    #[derive(Clone, Copy)]
    enum GroupMode {
        All,
        Any,
    }

    struct GroupFrame<'a> {
        conds: &'a [ConditionType],
        next_idx: usize,
        mode: GroupMode,
        value: bool,
    }

    match condition {
        ConditionType::None | ConditionType::CombatNone | ConditionType::EnterFight { .. } => {
            Some(true)
        }
        ConditionType::EnterFightAnd(_) | ConditionType::EnterFightOr(_) => {
            let mut groups: Vec<GroupFrame<'_>> = Vec::new();
            let mut current: &ConditionType = condition;

            let result = 'eval: loop {
                let mut value = match current {
                    ConditionType::None | ConditionType::CombatNone | ConditionType::EnterFight { .. } => true,
                    ConditionType::EnterFightAnd(conds) => {
                        if conds.is_empty() {
                            true
                        } else {
                            groups.push(GroupFrame {
                                conds,
                                next_idx: 1,
                                mode: GroupMode::All,
                                value: true,
                            });
                            current = &conds[0];
                            continue;
                        }
                    }
                    ConditionType::EnterFightOr(conds) => {
                        if conds.is_empty() {
                            false
                        } else {
                            groups.push(GroupFrame {
                                conds,
                                next_idx: 1,
                                mode: GroupMode::Any,
                                value: false,
                            });
                            current = &conds[0];
                            continue;
                        }
                    }
                    _ => super::check_condition(
                        fight,
                        buff_mgr,
                        ex_point_mgr,
                        bloodtithe,
                        caster_uid,
                        target_uid,
                        has_trigger_state,
                        current,
                    ),
                };

                loop {
                    let Some(frame) = groups.last_mut() else {
                        break 'eval value;
                    };

                    frame.value = match frame.mode {
                        GroupMode::All => frame.value && value,
                        GroupMode::Any => frame.value || value,
                    };

                    let short_circuit = match frame.mode {
                        GroupMode::All => !frame.value,
                        GroupMode::Any => frame.value,
                    };
                    if short_circuit {
                        value = frame.value;
                        groups.pop();
                        continue;
                    }

                    if frame.next_idx < frame.conds.len() {
                        current = &frame.conds[frame.next_idx];
                        frame.next_idx += 1;
                        break;
                    }

                    value = frame.value;
                    groups.pop();
                }
            };

            Some(result)
        }
        _ => None,
    }
}
