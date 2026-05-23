use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;

#[derive(Clone)]
pub struct Behaviour {
    pub raw: String,
    pub target: i32,
}

pub fn parse(raw: &str, beh_target: i32) -> Vec<Behaviour> {
    raw.split('|')
        .filter(|s| !s.is_empty())
        .map(|seg| Behaviour { raw: seg.to_string(), target: beh_target })
        .collect()
}

pub fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_20002AddExPoint) => ex_point::execute(managers, targets, raw),
        Some(BehaviourType::_1AddBuff) => add_buff::execute(fight, managers, targets, raw),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute(managers, targets, raw),
        Some(other) => { tracing::warn!("unimplemented behaviour type: {:?}", other); vec![] }
        None => vec![],
    }
}
