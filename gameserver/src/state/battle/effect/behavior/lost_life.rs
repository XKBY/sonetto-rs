use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    buff_actions::{EffectContext, lost_life as lost_life_handler},
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, bloodtithe, buff},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let mut out = Vec::new();
    for target in targets {
        let effects = match beh_type {
            BehaviourType::_30005LostLife
            | BehaviourType::_30006LostLife
            | BehaviourType::_30010LostLifeNotFixed
            | BehaviourType::_30018LostLife
            | BehaviourType::_60226LostLife2 => {
                let parts: Vec<&str> = raw.split('#').collect();
                let mode: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let attr_id: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                let permille: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let behavior_id: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
                let floor_permille = buff::ban_lost_life_floor_permille(fight, managers, target);
                bloodtithe::lost_life(
                    fight,
                    &managers.buff_mgr,
                    &mut mechanics.bloodtithe,
                    entity_uid,
                    target,
                    mode,
                    attr_id,
                    permille,
                    behavior_id,
                    0,
                    floor_permille,
                )
            }
            BehaviourType::_60216DamageRealLostLife => {
                let parts: Vec<&str> = raw.split('#').collect();
                let buff_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let rate: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut effect_ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
                lost_life_handler::damage_real_lost_life(&mut effect_ctx, buff_id, rate, 0)
            }
            BehaviourType::_60213SurvivalHealth | BehaviourType::_60146OriginDamageByTeamAttr => {
                tracing::warn!("unimplemented lost_life behaviour: {:?}", beh_type);
                vec![]
            }
            _ => vec![],
        };
        out.extend(effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }));
    }
    out
}
