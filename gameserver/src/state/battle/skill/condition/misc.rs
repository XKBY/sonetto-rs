use super::ConditionEval;
use super::super::super::ConditionType;
use super::action::Condition;
use crate::state::battle::{
    manager::buff_mgr::BuffMgr,
    round_state::simulated_round,
    skill::targets::{alive_allies, alive_enemies, get_entity},
    utils::check_career_restraint,
};
use sonettobuf::Fight;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Misc;

impl Condition for Misc {
    fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        parse(parts, cond_type)
    }

    fn check(condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        check(
            condition,
            ctx.fight,
            ctx.buff_mgr,
            ctx.caster_uid,
            ctx.target_uid,
        )
    }
}

static ALLOW_HRI_EVAL: AtomicBool = AtomicBool::new(false);

pub struct HriEvalGuard;

impl HriEvalGuard {
    pub fn enter() -> Self {
        ALLOW_HRI_EVAL.store(true, Ordering::Relaxed);
        Self
    }
}

impl Drop for HriEvalGuard {
    fn drop(&mut self) {
        ALLOW_HRI_EVAL.store(false, Ordering::Relaxed);
    }
}

pub fn hero_round_interval_matches(start_round: i32, period: i32, cur_round: i32) -> bool {
    if start_round <= 0 || period <= 0 || cur_round < start_round {
        return false;
    }
    if start_round == period {
        return cur_round == start_round;
    }
    (cur_round - start_round) % period == 0
}

fn deterministic_roll_permille(fight: &Fight, caster_uid: i64, target_uid: i64, salt: i32) -> i32 {
    let seed = fight.cur_round.unwrap_or(1) as i64
        + fight.version.unwrap_or(0) as i64
        + fight.battle_id.unwrap_or(0) as i64;
    let x = seed as i128 + caster_uid as i128 * 31 + target_uid as i128 * 17 + salt as i128 * 13;
    x.rem_euclid(1000) as i32
}

pub fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
    match cond_type {
        "Random" => Some(ConditionType::Random {
            permille: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "TeammateAlive" => Some(ConditionType::TeammateAlive {
            // live data commonly encodes: #0 => teammate alive, #1 => teammate dead/missing
            expect_dead: parts
                .get(1)
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0)
                == 1,
        }),
        "HurtRestraint" => Some(ConditionType::HurtRestraint),
        "HurtNumType" => Some(ConditionType::HurtNumType {
            type_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "ExSkillLevel" => Some(ConditionType::ExSkillLevel {
            levels: parts[1..]
                .iter()
                .flat_map(|p| p.split(','))
                .filter_map(|v| v.parse().ok())
                .collect(),
        }),
        "InMagicCircleId" => Some(ConditionType::InMagicCircleId {
            circle_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "TargetCount" => Some(ConditionType::TargetCount {
            value: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            mode: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        _ => None,
    }
}

pub fn check(
    condition: &ConditionType,
    fight: &Fight,
    _buff_mgr: &BuffMgr,
    caster_uid: i64,
    target_uid: i64,
) -> Option<bool> {
    match condition {
        ConditionType::TeammateAlive { expect_dead } => {
            let has_teammate_alive = alive_allies(fight, caster_uid)
                .iter()
                .any(|&uid| uid != caster_uid);
            Some(if *expect_dead {
                !has_teammate_alive
            } else {
                has_teammate_alive
            })
        }
        ConditionType::HurtRestraint => {
            let attacker_career = get_entity(fight, caster_uid).and_then(|e| e.career);
            let defender_career = get_entity(fight, target_uid).and_then(|e| e.career);
            match (attacker_career, defender_career) {
                (Some(a), Some(d)) => Some(check_career_restraint(a, d)),
                _ => Some(false),
            }
        }
        ConditionType::TargetCount { value, mode } => {
            // Branch skill behavior by count of available enemy targets.
            let target_count = alive_enemies(fight, caster_uid).len() as i32;
            let pass = match mode {
                // mode=1 is a threshold/split compare used by skills like 31140121:
                // - value=0 => single-target branch (only 1 enemy alive)
                // - value>=1 => multi-target branch (enemy_count >= value)
                1 => {
                    if *value == 0 {
                        target_count == 1
                    } else {
                        target_count >= *value
                    }
                }
                _ => target_count == *value,
            };
            Some(pass)
        }
        ConditionType::Random { permille } => {
            Some(deterministic_roll_permille(fight, caster_uid, target_uid, *permille) < *permille)
        }
        ConditionType::HeroRoundInterval {
            start_round,
            period,
        } => {
            if !ALLOW_HRI_EVAL.load(Ordering::Relaxed) {
                Some(false)
            } else {
                Some(hero_round_interval_matches(
                    *start_round,
                    *period,
                    simulated_round(),
                ))
            }
        }
        // combat only
        ConditionType::HurtNumType { .. }
        | ConditionType::ExSkillLevel { .. }
        | ConditionType::InMagicCircleId { .. } => Some(false),
        _ => None,
    }
}
