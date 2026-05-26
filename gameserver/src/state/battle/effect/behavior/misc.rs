use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{PendingMonsterChange, PendingSummon, SkillExecutor},
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

pub fn execute(
    _fight: &Fight, _managers: &mut Managers, _mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, _rng: &mut StdRng,
    caster_uid: i64, targets: Vec<i64>, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_60008Summon | BehaviourType::_60013SummonSp | BehaviourType::_60056SummonSp2 => {
            let monster_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            executor.pending_summons.push(PendingSummon { caster_uid, monster_id });
        }
        BehaviourType::_40006MonsterChange | BehaviourType::_40008MonsterChangeClearSelfCard => {
            let new_monster_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            for &t in &targets {
                executor.pending_monster_changes.push(PendingMonsterChange { target_uid: t, new_monster_id });
            }
        }
        BehaviourType::_60015Kill
        | BehaviourType::_60018Kill
        | BehaviourType::_60019KillTargets
        | BehaviourType::_20012HealCantCrit
        | BehaviourType::_20016HealCantCrit
        | BehaviourType::_20018HealCantCrit
        | BehaviourType::_100017IgnoreSkillConfigDamageRate => {}
        _ => {}
    }
    vec![]
}
