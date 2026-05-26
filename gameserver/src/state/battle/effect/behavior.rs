use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;
mod attr_modify;
mod stats;
mod disperse;
mod skill_rate;
mod misc;
mod catapult;
mod random;
mod magic_circle;
mod nuodika_damage;

#[derive(Debug, Clone)]
pub struct Behaviour {
    pub raw: String,
    pub target: i32,
}

pub fn is_attr_fix(raw: &str) -> bool {
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    matches!(
        behaviour_type(id),
        Some(BehaviourType::_10004AttrFix)
            | Some(BehaviourType::_10011AttrFixBuff)
            | Some(BehaviourType::_60033AttrFixByLoseHp)
    )
}

pub fn calculate_bonus(
    fight: &Fight,
    managers: &Managers,
    entity_uid: i64,
    raw: &str,
    beh_target: i32,
    count: i32,
) -> std::collections::HashMap<(i64, i32), i32> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let mut out = std::collections::HashMap::new();
    for uid in targets {
        let map = attr_modify::calculate_bonus(fight, managers, uid, raw, count);
        for (k, v) in map {
            *out.entry(k).or_insert(0) += v;
        }
    }
    out
}

pub fn parse(raw: &str, beh_target: i32) -> Vec<Behaviour> {
    raw.split('|')
        .filter(|s| !s.is_empty())
        .map(|seg| Behaviour { raw: seg.to_string(), target: beh_target })
        .collect()
}

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut crate::state::battle::mechanics::Mechanics,
    executor: &mut crate::state::battle::skill::SkillExecutor,
    rng: &mut rand::rngs::StdRng,
    entity_uid: i64,
    raw: &str,
    beh_target: i32,
    count: i32,
) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_20002AddExPoint) => ex_point::execute(managers, mechanics, executor, rng, targets, raw, count),
        Some(BehaviourType::_1AddBuff) => add_buff::execute(fight, managers, mechanics, executor, rng, targets, raw, count),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute(managers, targets, raw, count),
        Some(beh @ (BehaviourType::_20010Bloodlust
        | BehaviourType::_20011AverageLife
        | BehaviourType::_50017ChangePower
        | BehaviourType::_50037ChangePower)) => {
            stats::execute(fight, managers, mechanics, executor, rng, targets, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_30003Disperse1
        | BehaviourType::_30004Disperse2
        | BehaviourType::_30008Disperse1
        | BehaviourType::_30009Disperse2
        | BehaviourType::_30016Disperse3
        | BehaviourType::_30017Disperse4
        | BehaviourType::_90002Disperse2
        | BehaviourType::_60010DisperseForce2
        | BehaviourType::_60011DisperseForce1
        | BehaviourType::_20003Purify1
        | BehaviourType::_20004Purify2
        | BehaviourType::_20020PurifyX
        | BehaviourType::_50014ConsumeBuffByTypeId
        | BehaviourType::_50016ConsumeBuffByTypeId2)) => {
            disperse::execute(fight, managers, mechanics, executor, rng, targets, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_10001SkillRateUp
        | BehaviourType::_10002SkillRateUp1
        | BehaviourType::_10003SkillRateUp2
        | BehaviourType::_10009SkillRateUpExPoint
        | BehaviourType::_10012SkillRateUpBuffType
        | BehaviourType::_40015Rouge2MusicBlueBallSkillRateUp
        | BehaviourType::_60028ConsumePowerSkillRateUp)) => {
            skill_rate::execute(fight, managers, mechanics, executor, rng, entity_uid, targets, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_60008Summon
        | BehaviourType::_60013SummonSp
        | BehaviourType::_60056SummonSp2
        | BehaviourType::_40006MonsterChange
        | BehaviourType::_40008MonsterChangeClearSelfCard
        | BehaviourType::_60015Kill
        | BehaviourType::_60018Kill
        | BehaviourType::_60019KillTargets
        | BehaviourType::_20012HealCantCrit
        | BehaviourType::_20016HealCantCrit
        | BehaviourType::_20018HealCantCrit
        | BehaviourType::_100017IgnoreSkillConfigDamageRate)) => {
            misc::execute(fight, managers, mechanics, executor, rng, entity_uid, targets, raw, count, beh)
        }
        Some(beh @ BehaviourType::_60074CatapultBuff) => {
            catapult::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_20021AddBuffRanId | BehaviourType::_20022AddBuffRanTypeId | BehaviourType::_20023AddBuffRanTypeGroup)) => {
            random::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_50019AddMagicCircle | BehaviourType::_60163MagicCircleAddRound | BehaviourType::_60076MagicCircleAttr | BehaviourType::_50020RemoveAllMagicCircle | BehaviourType::_50021RemoveMagicCircleById | BehaviourType::_60270UpdateWangQiMagicCircle | BehaviourType::_60272ChangeElectricMagicCircleProgress)) => {
            magic_circle::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ BehaviourType::_60209NuoDiKaDamage) => {
            nuodika_damage::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(other) => { tracing::warn!("unimplemented behaviour type: {:?}", other); vec![] }
        None => vec![],
    }
}