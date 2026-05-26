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
    executor: &mut SkillExecutor, _rng: &mut StdRng,
    caster_uid: i64, targets: Vec<i64>, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let p2: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);

    match beh_type {
        BehaviourType::_10001SkillRateUp
        | BehaviourType::_10002SkillRateUp1
        | BehaviourType::_10003SkillRateUp2
        | BehaviourType::_40015Rouge2MusicBlueBallSkillRateUp
        | BehaviourType::_60028ConsumePowerSkillRateUp => {
            // raw: id#rate
            for &t in &targets {
                executor.add_skill_rate_bonus(caster_uid, t, p1);
            }
        }
        BehaviourType::_10009SkillRateUpExPoint => {
            // raw: id#buff_type_id#rate — bonus = rate * stacks(buff_type_id on caster)
            let buff_type_id = p1;
            let rate = p2;
            if rate != 0 {
                let stacks = buff::sum_stacks_by_type(fight, managers, caster_uid, buff_type_id);
                if stacks > 0 {
                    for &t in &targets {
                        executor.add_skill_rate_bonus(caster_uid, t, rate.saturating_mul(stacks));
                    }
                }
            }
        }
        BehaviourType::_10012SkillRateUpBuffType => {
            // raw: id#rate#?#buff_type1#buff_type2...
            let rate = p1;
            if rate != 0 {
                let buff_types: Vec<i32> = parts.iter().skip(3).filter_map(|v| v.parse().ok()).collect();
                if !buff_types.is_empty() {
                    for &t in &targets {
                        if buff::has_any_type(fight, managers, t, &buff_types) {
                            executor.add_skill_rate_bonus(caster_uid, t, rate);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    vec![]
}
