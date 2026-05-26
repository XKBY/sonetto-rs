use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{average_life, bloodlust, change_power, SkillExecutor},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    _fight: &Fight, _managers: &mut Managers, _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let amount: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    targets.into_iter().flat_map(|target| {
        let effects = match beh_type {
            BehaviourType::_20010Bloodlust => bloodlust(target, amount),
            BehaviourType::_20011AverageLife => average_life(target),
            BehaviourType::_50017ChangePower | BehaviourType::_50037ChangePower => change_power(target, amount),
            _ => vec![],
        };
        effects.into_iter().map(|e| Event::SerializedActEffect { effect: e })
    }).collect()
}
