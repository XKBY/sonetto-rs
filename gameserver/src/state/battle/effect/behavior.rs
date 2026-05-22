use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;

pub fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    raw.split('|').flat_map(|entry| {
        let id: i32 = entry.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match behaviour_type(id) {
            Some(BehaviourType::AddExPoint | BehaviourType::AttrFixExPoint) => ex_point::execute(managers, targets.clone(), entry),
            Some(BehaviourType::AddBuff) => add_buff::execute(fight, managers, targets.clone(), entry),
            _ => vec![],
        }
    }).collect()
}
