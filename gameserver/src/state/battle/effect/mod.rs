pub mod behavior;
pub mod behaviour_type;
pub mod condition;
pub mod condition_eval;
pub mod condition_type;
pub mod parser;
pub mod target;

use condition_eval::ConditionEval;
use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr, fight_data_mgr::Managers},
    mechanics::bloodtithe::BloodtitheState,
};
use sonettobuf::Fight;
use crate::state::battle::event::Event;

pub struct SkillEffect {
    pub(crate) behaviours: Vec<(condition::Condition, String, i32)>,
}

impl std::fmt::Debug for SkillEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SkillEffect({})", self.behaviours.len())
    }
}

impl Clone for SkillEffect {
    fn clone(&self) -> Self { Self { behaviours: vec![] } }
}

impl SkillEffect {
    pub fn on_enter_fight(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let matching: Vec<_> = self.behaviours
            .iter()
            .filter(|(cond, _, _)| {
                matches!(cond.hook, condition::Hook::EnterFight)
                    && cond.check(ConditionEval {
                        fight,
                        buff_mgr: &buff_mgr,
                        entity_mgr: &entity_mgr,
                        bloodtithe: &bloodtithe,
                        caster_uid: entity_uid,
                        target_uid: entity_uid,
                        condition_target: 0,
                        has_trigger_state: false,
                        active_card_cast_uids: None,
                    })
            })
            .map(|(_, beh, beh_target)| (beh.clone(), *beh_target))
            .collect();
        matching.into_iter()
            .flat_map(|(beh, beh_target)| behavior::execute(fight, managers, entity_uid, &beh, beh_target))
            .collect()
    }

    pub fn on_dead(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let matching: Vec<_> = self.behaviours
            .iter()
            .filter(|(cond, _, _)| {
                matches!(cond.hook, condition::Hook::Dead)
                    && cond.check(ConditionEval {
                        fight,
                        buff_mgr: &buff_mgr,
                        entity_mgr: &entity_mgr,
                        bloodtithe: &bloodtithe,
                        caster_uid: entity_uid,
                        target_uid: entity_uid,
                        condition_target: 0,
                        has_trigger_state: false,
                        active_card_cast_uids: None,
                    })
            })
            .map(|(_, beh, beh_target)| (beh.clone(), *beh_target))
            .collect();
        matching.into_iter()
            .flat_map(|(beh, beh_target)| behavior::execute(fight, managers, entity_uid, &beh, beh_target))
            .collect()
    }
}
