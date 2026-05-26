use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, bloodtithe as bloodtithe_behavior},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, _managers: &mut Managers, mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, _entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let target = targets.into_iter().next().unwrap_or(0);
    let amount: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let effects = match beh_type {
        BehaviourType::_60190BloodPoolMaxChange => {
            bloodtithe_behavior::pool_max_change(fight, &mut mechanics.bloodtithe, target, amount)
        }
        BehaviourType::_60191BloodPoolValueChange => {
            bloodtithe_behavior::pool_value_change(fight, &mut mechanics.bloodtithe, target, amount)
        }
        BehaviourType::_60199ConsumeBloodPoolHeal => {
            tracing::warn!("unimplemented bloodtithe behaviour: {:?}", beh_type);
            vec![]
        }
        _ => vec![],
    };
    effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }).collect()
}
