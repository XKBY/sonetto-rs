use crate::state::battle::skill::PhaseFilter;
use crate::state::battle::skill::targets::resolve_behavior_targets;
use crate::state::battle::types::behavior::BehaviorType;
use sonettobuf::Fight;

/// Read-heavy context for behavior execution.
pub struct BehaviorContext<'a> {
    pub caster_uid: i64,
    pub target_uid: i64,
    pub skill_id: i32,
    pub slot: u8,
    pub behavior_target: i32,
    pub condition_target: i32,
    pub logic_target: i32,
    pub phase: &'a PhaseFilter,
    pub fight: &'a Fight,
}

impl<'a> BehaviorContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fight: &'a Fight,
        caster_uid: i64,
        target_uid: i64,
        skill_id: i32,
        slot: u8,
        behavior_target: i32,
        condition_target: i32,
        logic_target: i32,
        phase: &'a PhaseFilter,
    ) -> Self {
        Self {
            caster_uid,
            target_uid,
            skill_id,
            slot,
            behavior_target,
            condition_target,
            logic_target,
            phase,
            fight,
        }
    }

    pub fn resolve_targets(&self, self_targeted: bool, behavior: &BehaviorType) -> Vec<i64> {
        if self_targeted {
            return vec![self.caster_uid];
        }
        resolve_behavior_targets(
            self.fight,
            self.caster_uid,
            self.target_uid,
            self.behavior_target,
            self.condition_target,
            self.logic_target,
            matches!(behavior, BehaviorType::AddBuff { .. }),
        )
    }
}
