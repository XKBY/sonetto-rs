use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;

#[derive(Debug, Clone)]
pub struct Behaviour {
    pub raw: String,
    pub target: i32,
}

pub fn is_attr_fix(raw: &str) -> bool {
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    matches!(
        behaviour_type(id),
        Some(BehaviourType::_10004AttrFix)
            | Some(BehaviourType::_10011AttrFixBuff)
            | Some(BehaviourType::_60033AttrFixByLoseHp)
    )
}

pub fn parse(raw: &str, beh_target: i32) -> Vec<Behaviour> {
    raw.split('|')
        .filter(|s| !s.is_empty())
        .map(|seg| Behaviour { raw: seg.to_string(), target: beh_target })
        .collect()
}

pub fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32, count: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_20002AddExPoint) => ex_point::execute(managers, targets, raw, count),
        Some(BehaviourType::_1AddBuff) => add_buff::execute(fight, managers, targets, raw, count),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute(managers, targets, raw, count),
        Some(other) => { tracing::warn!("unimplemented behaviour type: {:?}", other); vec![] }
        None => vec![],
    }
}

pub fn execute_reversed(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_1AddBuff) => add_buff::execute_reversed(managers, targets, raw),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute_reversed(managers, targets, raw),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_attr_fix_recognizes_attr_fix_family() {
        assert!(is_attr_fix("10004#102#50"));         // _10004AttrFix
        assert!(is_attr_fix("10011#102#50#1"));        // _10011AttrFixBuff
        assert!(is_attr_fix("60033#100#205#75#8"));    // _60033AttrFixByLoseHp
    }

    #[test]
    fn is_attr_fix_rejects_non_attr_fix() {
        assert!(!is_attr_fix("1#100"));        // _1AddBuff
        assert!(!is_attr_fix("20002#10"));     // _20002AddExPoint
        assert!(!is_attr_fix("10006#1000"));   // _10006Damage
    }

    #[test]
    fn is_attr_fix_handles_malformed_input() {
        assert!(!is_attr_fix(""));
        assert!(!is_attr_fix("abc"));
        assert!(!is_attr_fix("999999999"));
    }
}

