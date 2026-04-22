use super::super::manager::buff_mgr::BuffMgr;
use super::super::{BehaviorType, ConditionType};
use super::condition::{self, buff::deleted_matches};

/// Which conditions are active for this trigger event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriggerState {
    pub active_use_skill: bool,
    pub skill_id: i32,
    pub used_ex_skill: bool,
    pub teammate_use_ex_skill: bool,
    pub trigger_bullet: bool,
    /// When true, only event-driven behavior conditions should execute.
    /// Static/start-of-fight condition rows are filtered out by executor.
    pub event_driven_only: bool,
    pub be_attacked: bool,
    pub hurt_not_restraint: bool,
    pub hurt_restraint: bool,
    pub teammate_injury_count: bool,
    pub team_injury_count_round: bool,
    pub deleted_buff_ids: Vec<i32>,
}

impl TriggerState {
    pub fn on_use_card() -> Self {
        Self {
            active_use_skill: true,
            skill_id: 0,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            ..Default::default()
        }
    }
    pub fn on_active_use_skill(skill_id: i32) -> Self {
        Self {
            active_use_skill: true,
            skill_id,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            ..Default::default()
        }
    }
    #[allow(dead_code)]
    pub fn on_attack() -> Self {
        Self {
            hurt_not_restraint: true,
            ..Default::default()
        }
    }
    #[allow(dead_code)]
    pub fn on_take_damage() -> Self {
        Self {
            be_attacked: true,
            ..Default::default()
        }
    }

    /// Populate `deleted_buff_ids` from the live per-step buff-deletion
    /// tracker on `BuffMgr`. Used by mid-step passive/trigger sites that don't
    /// have an explicit `TriggerEvent` to pull a list from. If the caller has
    /// already set `deleted_buff_ids` explicitly, those are preserved.
    pub fn with_buff_mgr(mut self, buff_mgr: &BuffMgr) -> Self {
        if self.deleted_buff_ids.is_empty() {
            self.deleted_buff_ids = buff_mgr.step_deleted_buff_ids().to_vec();
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhaseFilter {
    BattleStart,
    EnterFight,
    Unconditional,
    ConsumeBlood,
    Combat(TriggerState),
}

impl PhaseFilter {
    pub fn battle_start() -> Self {
        Self::BattleStart
    }
    pub fn enter_fight() -> Self {
        Self::EnterFight
    }
    pub fn unconditional() -> Self {
        Self::Unconditional
    }
    pub fn consume_blood() -> Self {
        Self::ConsumeBlood
    }
    pub fn combat() -> Self {
        Self::Combat(TriggerState::default())
    }
    pub fn combat_with(state: TriggerState) -> Self {
        Self::Combat(state)
    }

    #[allow(dead_code)]
    pub fn is_combat(&self) -> bool {
        matches!(self, Self::Combat(_))
    }
    pub fn is_consume_blood(&self) -> bool {
        matches!(self, Self::ConsumeBlood)
    }

    pub fn allows_behavior(&self, behavior: &BehaviorType) -> bool {
        if self.is_consume_blood() {
            return matches!(
                behavior,
                BehaviorType::ConsumeBloodAddBuff { .. }
                    | BehaviorType::ConsumeBloodAddBuff2 { .. }
            );
        }
        true
    }

    pub fn check(&self, condition: &ConditionType, behavior_target: i32) -> bool {
        if let Self::Combat(event) = self {
            return self.check_combat(condition, event);
        }
        self.check_non_combat(condition, behavior_target)
    }

    fn check_combat(&self, condition: &ConditionType, event: &TriggerState) -> bool {
        condition::fold(condition, &mut |cond| self.check_combat_leaf(cond, event))
    }

    fn check_non_combat(&self, condition: &ConditionType, behavior_target: i32) -> bool {
        struct GroupFrame<'a> {
            conds: &'a [ConditionType],
            next_idx: usize,
            value: bool,
        }

        let mut groups: Vec<GroupFrame<'_>> = Vec::new();
        let mut current = condition;

        loop {
            let mut result = match current {
                ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => {
                    if conds.is_empty() {
                        false
                    } else {
                        groups.push(GroupFrame {
                            conds,
                            next_idx: 1,
                            value: false,
                        });
                        current = &conds[0];
                        continue;
                    }
                }
                _ => self.check_non_combat_leaf(current, behavior_target),
            };

            loop {
                let Some(frame) = groups.last_mut() else {
                    return result;
                };

                frame.value = frame.value || result;

                if frame.value {
                    result = true;
                    groups.pop();
                    continue;
                }

                if frame.next_idx < frame.conds.len() {
                    current = &frame.conds[frame.next_idx];
                    frame.next_idx += 1;
                    break;
                }

                result = false;
                groups.pop();
            }
        }
    }

    fn check_non_combat_leaf(&self, condition: &ConditionType, behavior_target: i32) -> bool {
        match condition {
            ConditionType::EnterFight { condition_id } => match self {
                Self::BattleStart => *condition_id == 5021,
                Self::EnterFight => *condition_id == 5,
                Self::Unconditional => *condition_id == 6,
                _ => false,
            },
            ConditionType::None => matches!(self, Self::Unconditional),
            ConditionType::TargetCareer { .. } => {
                matches!(self, Self::EnterFight) && behavior_target == 103
            }
            ConditionType::NoBuffId { .. } => self.is_consume_blood(),
            ConditionType::NoActRound => false,
            ConditionType::CareerCheck { .. } => {
                matches!(self, Self::Unconditional | Self::Combat(_))
            }
            _ => false,
        }
    }

    fn check_combat_leaf(&self, condition: &ConditionType, event: &TriggerState) -> bool {
        match condition {
            // Non-event conditions always pass in combat
            ConditionType::None
            | ConditionType::CombatNone
            | ConditionType::HasBuffId { .. }
            | ConditionType::NoBuffId { .. }
            | ConditionType::HasTypeIdBuffMoreThan { .. }
            | ConditionType::TypeIdBuffCountMoreThan { .. }
            | ConditionType::TypeIdBuffCountLessThan { .. }
            | ConditionType::HasTypeIdBuffEqual { .. }
            | ConditionType::LifeLess { .. }
            | ConditionType::LifeMore { .. }
            | ConditionType::TargetCareer { .. }
            | ConditionType::ExpointMoreThan { .. }
            | ConditionType::ExpointLessThan { .. }
            | ConditionType::Random { .. }
            | ConditionType::PerBuffIdCount { .. }
            | ConditionType::ExSkillLevel { .. }
            | ConditionType::PerExPoint { .. }
            | ConditionType::PerHasTargetCareerList { .. }
            | ConditionType::PerDecrExPoint { .. }
            | ConditionType::CareerCheck { .. }
            | ConditionType::TeammateAlive { .. }
            | ConditionType::BattleTagNum { .. }
            | ConditionType::TargetCount { .. } => true,

            // Event-driven conditions — only fire for matching event
            ConditionType::ActiveUseSkill => event.active_use_skill,
            ConditionType::ActiveUseSkillId { skill_ids } => {
                event.active_use_skill && skill_ids.contains(&event.skill_id)
            }
            ConditionType::UseExSkill => event.active_use_skill && event.used_ex_skill,
            ConditionType::TeammateUseExSkill => event.teammate_use_ex_skill,
            ConditionType::BeAttacked => event.be_attacked,
            ConditionType::HurtNotRestraint => event.hurt_not_restraint,
            ConditionType::HurtRestraint => event.hurt_restraint,
            ConditionType::TeammateInjuryCount => event.teammate_injury_count,
            ConditionType::TeamInjuryCountRound => event.team_injury_count_round,
            ConditionType::BuffIdDel { buff_ids } => {
                deleted_matches(&event.deleted_buff_ids, buff_ids)
            }
            ConditionType::TriggerBullet => event.trigger_bullet,
            ConditionType::NoActRound => !event.active_use_skill,

            // Not yet implemented in combat
            _ => false,
        }
    }
}
