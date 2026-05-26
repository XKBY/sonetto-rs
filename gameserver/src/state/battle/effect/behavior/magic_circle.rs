use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
    buff_actions::EffectContext,
    mechanics::magic_circle as magic_circle_mechanic,
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_50019AddMagicCircle | BehaviourType::_60163MagicCircleAddRound => {
            let circle_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let target = targets.into_iter().next().unwrap_or(0);
            let mut ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
            magic_circle_mechanic::add_magic_circle(&mut ctx, executor, fight, entity_uid, circle_id)
                .unwrap_or_default()
                .into_iter()
                .map(|e| Event::SerializedActEffect { effect: e })
                .collect()
        }
        BehaviourType::_60076MagicCircleAttr
        | BehaviourType::_50020RemoveAllMagicCircle
        | BehaviourType::_50021RemoveMagicCircleById
        | BehaviourType::_60270UpdateWangQiMagicCircle
        | BehaviourType::_60272ChangeElectricMagicCircleProgress => vec![],
        _ => vec![],
    }
}
