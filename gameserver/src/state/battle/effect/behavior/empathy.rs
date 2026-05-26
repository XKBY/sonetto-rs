use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    _fight: &Fight, _managers: &mut Managers, _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    _targets: Vec<i64>, _entity_uid: i64, _raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    tracing::warn!("unimplemented empathy behaviour: {:?}", beh_type);
    vec![]
}
