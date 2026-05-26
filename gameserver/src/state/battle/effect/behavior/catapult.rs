use rand::{rngs::StdRng, seq::SliceRandom};
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{buff, SkillExecutor, targets::alive_enemies},
    types::condition::ConditionType,
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, _beh_type: BehaviourType,
) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let primary_stacks: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let buff_id: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
    let catapult_stacks: i32 = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
    let catapult_cap: i32 = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);

    let has_bloodpool = mechanics.bloodtithe.has_bloodpool();
    let target = targets.into_iter().next().unwrap_or(0);
    let mut out = Vec::new();

    for _ in 0..primary_stacks.max(0) {
        out.extend(buff::apply(
            buff::BuffApplySpec::new(buff_id)
                .caster(entity_uid)
                .target(target)
                .count(1)
                .bloodpool(has_bloodpool)
                .skill(0)
                .condition(0, &ConditionType::None),
            executor, fight, managers, mechanics,
        ).into_iter().map(|e| Event::SerializedActEffect { effect: e }));
    }

    let mut all_enemies = alive_enemies(fight, entity_uid);
    all_enemies.shuffle(rng);

    for enemy_uid in all_enemies.into_iter().take(catapult_cap.max(0) as usize) {
        for _ in 0..catapult_stacks.max(0) {
            out.extend(buff::apply(
                buff::BuffApplySpec::new(buff_id)
                    .caster(entity_uid)
                    .target(enemy_uid)
                    .count(1)
                    .bloodpool(has_bloodpool)
                    .skill(0)
                    .condition(0, &ConditionType::None),
                executor, fight, managers, mechanics,
            ).into_iter().map(|e| Event::SerializedActEffect { effect: e }));
        }
    }

    out
}
