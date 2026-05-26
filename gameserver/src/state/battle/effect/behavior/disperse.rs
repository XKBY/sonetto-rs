use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{buff, SkillExecutor},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    targets.into_iter().flat_map(|target| {
        let effects = match beh_type {
            BehaviourType::_30003Disperse1
            | BehaviourType::_30004Disperse2
            | BehaviourType::_30008Disperse1
            | BehaviourType::_30009Disperse2
            | BehaviourType::_30016Disperse3
            | BehaviourType::_30017Disperse4
            | BehaviourType::_90002Disperse2 => buff::disperse(fight, managers, target),
            BehaviourType::_60010DisperseForce2 | BehaviourType::_60011DisperseForce1 => {
                let buff_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                buff::disperse_force(fight, managers, target, buff_id)
            }
            BehaviourType::_20003Purify1 | BehaviourType::_20004Purify2 | BehaviourType::_20020PurifyX => {
                buff::purify(fight, managers, target)
            }
            BehaviourType::_50014ConsumeBuffByTypeId | BehaviourType::_50016ConsumeBuffByTypeId2 => {
                let type_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let count: i32 = raw.split('#').nth(2).and_then(|v| v.parse().ok()).unwrap_or(1);
                buff::consume_by_type(fight, managers, target, type_id, 0, count)
            }
            _ => vec![],
        };
        effects.into_iter().map(|e| Event::SerializedActEffect { effect: e })
    }).collect()
}
