use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    buff_actions::{EffectContext, lost_life as lost_life_handler},
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_60215InjurySaveDamage => {
            tracing::warn!("unimplemented behaviour: {:?}", beh_type);
            return vec![];
        }
        _ => {}
    }

    let rate: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = Vec::new();
    for target in targets {
        let mut effect_ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
        let effects = lost_life_handler::apply(&mut effect_ctx, Some(&executor.pending_attr_bonus), rate, 0);
        out.extend(effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }));
    }
    out
}
