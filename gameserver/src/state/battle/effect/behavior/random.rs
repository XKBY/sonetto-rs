use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{random, SkillExecutor},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_20021AddBuffRanId => {
            let parts: Vec<&str> = raw.split('#').collect();
            let pool_buff_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let count: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            targets.into_iter().flat_map(|target| {
                random::add_buff_ran_id(executor, rng, fight, managers, mechanics, entity_uid, target, pool_buff_id, count)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| Event::SerializedActEffect { effect: e })
            }).collect()
        }
        BehaviourType::_20022AddBuffRanTypeId | BehaviourType::_20023AddBuffRanTypeGroup => {
            tracing::warn!("unimplemented behaviour type: {:?}", beh_type);
            vec![]
        }
        _ => vec![],
    }
}
