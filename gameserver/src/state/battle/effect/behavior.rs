use sonettobuf::Fight;
use crate::state::battle::event::Event;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;

pub fn execute(fight: &Fight, entity_uid: i64, raw: &str, beh_target: i32) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let id: i32 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);

    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    match behaviour_type(id) {
        Some(BehaviourType::AddExPoint | BehaviourType::AttrFixExPoint) => ex_point::execute(targets, p1),
        _ => vec![],
    }
}
