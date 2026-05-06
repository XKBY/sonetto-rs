use super::super::manager::buff_mgr::BuffMgr;
use super::super::{BehaviorType, ConditionType};
use super::condition;
use std::collections::HashSet;

/// Which conditions are active for this trigger event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriggerState {
    pub active_use_skill: bool,
    pub skill_id: i32,
    pub action_order_index: i32,
    pub used_ex_skill: bool,
    pub teammate_use_ex_skill: bool,
    pub trigger_bullet: bool,
    /// When true, only event-driven behavior conditions should execute.
    /// Static/start-of-fight condition rows are filtered out by executor.
    pub event_driven_only: bool,
    pub be_attacked: bool,
    /// Set when the passive owner took Mental DMG in this event (gates
    /// `ConditionType::HurtMagic`). Source-side discrimination uses the
    /// dealer's hero `dmgType` from `character.json` (2 = Mental).
    pub hurt_magic: bool,
    /// Set when the passive owner's ExPoint (Moxie / Faith / etc.) just
    /// decreased in this event (gates `ConditionType::LostExPoint`).
    /// Detected by scanning emitted `ExPointChange(111)` actEffects with
    /// negative `effect_num` targeting the owner.
    pub lost_ex_point: bool,
    pub hurt_not_restraint: bool,
    pub hurt_restraint: bool,
    pub teammate_injury_count: i32,
    pub teammate_injury_count_not_reset: i32,
    pub team_injury_count_round: bool,
    pub deleted_buff_ids: Vec<i32>,
    pub active_card_cast_uids: HashSet<i64>,
    /// Attacker-side bloodpool max snapshot. `None` means the caller didn't
    /// populate it, so BloodPool/BloodPoolMax conditions fall back to
    /// always-pass for backward compatibility.
    pub bloodpool_max_attacker: Option<i32>,
    pub bloodpool_value_attacker: Option<i32>,
}

impl TriggerState {
    pub fn on_active_use_skill(skill_id: i32) -> Self {
        Self {
            active_use_skill: true,
            skill_id,
            action_order_index: 0,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            ..Default::default()
        }
    }

    pub fn with_action_order_index(mut self, action_order_index: i32) -> Self {
        self.action_order_index = action_order_index;
        self
    }

    pub fn with_used_ex_skill(mut self, used_ex_skill: bool) -> Self {
        self.used_ex_skill = used_ex_skill;
        self
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

    pub fn with_round_active_card_cast_uids(mut self, uids: &HashSet<i64>) -> Self {
        self.active_card_cast_uids = uids.clone();
        self
    }

    pub fn inherit_round_active_card_casts_from_phase(mut self, phase: &PhaseFilter) -> Self {
        if let PhaseFilter::Combat(event) = phase {
            self.active_card_cast_uids = event.active_card_cast_uids.clone();
        }
        self
    }

    pub fn no_act_round_for(&self, owner_uid: i64) -> bool {
        owner_uid == 0 || !self.active_card_cast_uids.contains(&owner_uid)
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

    pub fn check(&self, condition: &ConditionType, behavior_target: i32, owner_uid: i64) -> bool {
        if let Self::Combat(event) = self {
            return self.check_combat(condition, event, owner_uid);
        }
        self.check_non_combat(condition, behavior_target)
    }

    fn check_combat(
        &self,
        condition: &ConditionType,
        event: &TriggerState,
        owner_uid: i64,
    ) -> bool {
        condition::fold(condition, &mut |cond| {
            self.check_combat_leaf(cond, event, owner_uid)
        })
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

    fn check_combat_leaf(
        &self,
        condition: &ConditionType,
        event: &TriggerState,
        owner_uid: i64,
    ) -> bool {
        if let Some(pass) = condition::eval_trigger_state_condition(
            condition,
            condition::TriggerStateConditionContext { event, owner_uid },
            condition::TriggerStateConditionOptions {
                include_none: true,
                include_combat_none: true,
                include_hurt_magic: true,
                include_lost_ex_point: true,
            },
        ) {
            return pass;
        }

        match condition {
            // Non-event conditions always pass in combat
            ConditionType::HasBuffId { .. }
            | ConditionType::NoBuffId { .. }
            | ConditionType::HasBuffGroup { .. }
            | ConditionType::NoBuffGroup { .. }
            | ConditionType::HasTypeIdBuffMoreThan { .. }
            | ConditionType::TypeIdBuffCountMoreThan { .. }
            | ConditionType::TypeIdBuffCountLessThan { .. }
            | ConditionType::HasTypeIdBuffEqual { .. }
            | ConditionType::LifeLess { .. }
            | ConditionType::LifeMore { .. }
            | ConditionType::TargetCareer { .. }
            | ConditionType::TargetIsSelf
            | ConditionType::TargetIsTeamNoMe
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
            | ConditionType::HeroRoundInterval { .. }
            | ConditionType::TargetCount { .. } => true,

            // Bloodpool state conditions — evaluated against snapshot when provided,
            // always-pass otherwise for sites that don't populate the snapshot.
            ConditionType::BloodPool => event
                .bloodpool_value_attacker
                .map(|v| v > 0)
                .unwrap_or(true),
            ConditionType::BloodPoolMax { min, max } => event
                .bloodpool_max_attacker
                .map(|pool_max| pool_max >= *min && pool_max <= *max)
                .unwrap_or(true),

            // Not yet implemented in combat
            _ => false,
        }
    }
}
