use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;
mod attr_modify;

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

pub fn calculate_bonus(
    fight: &Fight,
    managers: &Managers,
    entity_uid: i64,
    raw: &str,
    beh_target: i32,
    count: i32,
) -> std::collections::HashMap<(i64, i32), i32> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let mut out = std::collections::HashMap::new();
    for uid in targets {
        let map = attr_modify::calculate_bonus(fight, managers, uid, raw, count);
        for (k, v) in map {
            *out.entry(k).or_insert(0) += v;
        }
    }
    out
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

