mod bloodtithe;
pub mod buff;
mod career;
mod combat;
mod enter_fight;
mod ex_point;
mod life;
mod misc;

pub mod parser;

use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr},
    mechanics::bloodtithe::BloodtitheState,
};
use sonettobuf::Fight;

pub use crate::state::battle::types::condition::ConditionType;

#[derive(Clone, Copy)]
pub struct ConditionEval<'a> {
    fight: &'a Fight,
    buff_mgr: &'a BuffMgr,
    ex_point_mgr: &'a ExPointMgr,
    bloodtithe: &'a BloodtitheState,
    caster_uid: i64,
    target_uid: i64,
    has_trigger_state: bool,
}

impl<'a> ConditionEval<'a> {
    pub fn new(
        fight: &'a Fight,
        buff_mgr: &'a BuffMgr,
        ex_point_mgr: &'a ExPointMgr,
        bloodtithe: &'a BloodtitheState,
        caster_uid: i64,
    ) -> Self {
        Self {
            fight,
            buff_mgr,
            ex_point_mgr,
            bloodtithe,
            caster_uid,
            target_uid: caster_uid,
            has_trigger_state: false,
        }
    }

    pub fn for_target(mut self, target_uid: i64) -> Self {
        self.target_uid = target_uid;
        self
    }

    pub fn with_trigger_state(mut self, has_trigger_state: bool) -> Self {
        self.has_trigger_state = has_trigger_state;
        self
    }

    pub fn check(&self, condition: &ConditionType) -> bool {
        if self.has_trigger_state && is_grouped_combat_event_condition(condition) {
            return true;
        }

        if !self.has_trigger_state
            && matches!(
                condition,
                ConditionType::PerDecrExPoint { .. }
                    | ConditionType::UseExSkill
                    | ConditionType::ActiveUseSkill
                    | ConditionType::TeammateUseExSkill
                    | ConditionType::ActiveUseSkillId { .. }
                    | ConditionType::TriggerBullet
                    | ConditionType::BeAttacked
                    | ConditionType::HurtNotRestraint
                    | ConditionType::HurtRestraint
                    | ConditionType::TeammateInjuryCount
                    | ConditionType::TeamInjuryCountRound
                    | ConditionType::BuffIdDel { .. }
                    | ConditionType::NoActRound
            )
        {
            return false;
        }

        enter_fight::check(
            condition,
            self.fight,
            self.buff_mgr,
            self.ex_point_mgr,
            self.bloodtithe,
            self.caster_uid,
            self.target_uid,
            self.has_trigger_state,
        )
        .or_else(|| buff::check(condition, self.buff_mgr, self.target_uid))
        .or_else(|| career::check(condition, self.fight, self.caster_uid, self.target_uid))
        .or_else(|| life::check(condition, self.fight, self.caster_uid))
        .or_else(|| ex_point::check(condition, self.ex_point_mgr, self.caster_uid))
        .or_else(|| combat::check(condition))
        .or_else(|| bloodtithe::check(condition, self.bloodtithe))
        .or_else(|| {
            misc::check(
                condition,
                self.fight,
                self.buff_mgr,
                self.caster_uid,
                self.target_uid,
            )
        })
        .unwrap_or(false)
    }

    pub fn check_with_random_target(
        &self,
        random_target_uid: i64,
        condition: &ConditionType,
    ) -> bool {
        match condition {
            ConditionType::EnterFightAnd(conds) => conds.iter().all(|cond| {
                let leaf_target = if matches!(cond, ConditionType::Random { .. }) {
                    random_target_uid
                } else {
                    self.target_uid
                };
                self.for_target(leaf_target)
                    .with_trigger_state(true)
                    .check(cond)
            }),
            ConditionType::EnterFightOr(conds) => conds.iter().any(|cond| {
                let leaf_target = if matches!(cond, ConditionType::Random { .. }) {
                    random_target_uid
                } else {
                    self.target_uid
                };
                self.for_target(leaf_target)
                    .with_trigger_state(true)
                    .check(cond)
            }),
            _ => self.with_trigger_state(true).check(condition),
        }
    }
}

/// Evaluate a (possibly composite) condition by applying `leaf(cond)` at each
/// non-composite node. `EnterFightAnd` folds with logical AND, `EnterFightOr`
/// folds with logical OR. Non-composite `cond`s are evaluated directly.
pub fn fold<F>(condition: &ConditionType, leaf: &mut F) -> bool
where
    F: FnMut(&ConditionType) -> bool,
{
    match condition {
        ConditionType::EnterFightAnd(conds) => conds.iter().all(|cond| fold(cond, leaf)),
        ConditionType::EnterFightOr(conds) => conds.iter().any(|cond| fold(cond, leaf)),
        other => leaf(other),
    }
}

/// Compatibility shim for condition walkers that still pass the full context piecemeal.
#[allow(clippy::too_many_arguments)]
pub fn check_condition(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    ex_point_mgr: &ExPointMgr,
    bloodtithe: &BloodtitheState,
    caster_uid: i64,
    target_uid: i64,
    has_trigger_state: bool,
    condition: &ConditionType,
) -> bool {
    ConditionEval::new(fight, buff_mgr, ex_point_mgr, bloodtithe, caster_uid)
        .for_target(target_uid)
        .with_trigger_state(has_trigger_state)
        .check(condition)
}

fn is_grouped_combat_event_condition(condition: &ConditionType) -> bool {
    matches!(
        condition,
        ConditionType::CombatNone
            | ConditionType::ActiveUseSkill
            | ConditionType::ActiveUseSkillId { .. }
            | ConditionType::UseExSkill
            | ConditionType::TeammateUseExSkill
            | ConditionType::TriggerBullet
            | ConditionType::BeAttacked
            | ConditionType::HurtNotRestraint
            | ConditionType::HurtRestraint
            | ConditionType::TeammateInjuryCount
            | ConditionType::TeamInjuryCountRound
            | ConditionType::BuffIdDel { .. }
            | ConditionType::NoActRound
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonettobuf::{FightEntityInfo, FightTeam};

    fn build_fight() -> Fight {
        Fight {
            attacker: Some(FightTeam {
                entitys: vec![FightEntityInfo {
                    uid: Some(1),
                    career: Some(1),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            defender: Some(FightTeam {
                entitys: vec![FightEntityInfo {
                    uid: Some(-1),
                    career: Some(4),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn assert_false_without_trigger_state(condition: ConditionType) {
        let mut ex_point_mgr = ExPointMgr::new();
        ex_point_mgr.set_recent_decr_ex_point(1, 3);
        let fight = build_fight();
        let buff_mgr = BuffMgr::new();
        let bloodtithe = BloodtitheState::new();

        assert!(!check_condition(
            &fight,
            &buff_mgr,
            &ex_point_mgr,
            &bloodtithe,
            1,
            -1,
            false,
            &condition,
        ));
    }

    #[test]
    fn per_decr_ex_point_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::PerDecrExPoint { threshold: 1 });
    }

    #[test]
    fn use_ex_skill_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::UseExSkill);
    }

    #[test]
    fn teammate_use_ex_skill_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeammateUseExSkill);
    }

    #[test]
    fn active_use_skill_id_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::ActiveUseSkillId {
            skill_ids: vec![1234],
        });
    }

    #[test]
    fn be_attacked_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::BeAttacked);
    }

    #[test]
    fn hurt_not_restraint_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::HurtNotRestraint);
    }

    #[test]
    fn hurt_restraint_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::HurtRestraint);
    }

    #[test]
    fn teammate_injury_count_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeammateInjuryCount);
    }

    #[test]
    fn team_injury_count_round_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeamInjuryCountRound);
    }

    #[test]
    fn buff_id_del_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::BuffIdDel {
            buff_ids: vec![4150003],
        });
    }

    #[test]
    fn trigger_bullet_is_true_with_trigger_state() {
        let ex_point_mgr = ExPointMgr::new();
        let fight = build_fight();
        let buff_mgr = BuffMgr::new();
        let bloodtithe = BloodtitheState::new();

        assert!(check_condition(
            &fight,
            &buff_mgr,
            &ex_point_mgr,
            &bloodtithe,
            1,
            -1,
            true,
            &ConditionType::TriggerBullet,
        ));
    }
}
