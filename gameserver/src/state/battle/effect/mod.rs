pub mod behavior;
pub mod behaviour_type;
pub mod condition;
pub mod condition_eval;
pub mod condition_type;
pub mod parser;
pub mod target;

use condition_eval::ConditionEval;
use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    mechanics::bloodtithe::BloodtitheState,
};
use sonettobuf::Fight;
use crate::state::battle::event::Event;

pub struct SkillEffect {
    behaviours: Vec<(condition::Condition, String, i32)>,
}

impl SkillEffect {
    pub fn on_enter_fight(&self, fight: &Fight, entity_uid: i64) -> Vec<Event> {
        let buff_mgr = BuffMgr::default();
        let ex_point_mgr = EntityMgr::default();
        let bloodtithe = BloodtitheState::default();
        self.behaviours
            .iter()
            .filter(|(cond, _, _)| {
                matches!(cond.hook, condition::Hook::EnterFight)
                    && cond.check(ConditionEval {
                        fight,
                        buff_mgr: &buff_mgr,
                        ex_point_mgr: &ex_point_mgr,
                        bloodtithe: &bloodtithe,
                        caster_uid: entity_uid,
                        target_uid: entity_uid,
                        condition_target: 0,
                        has_trigger_state: false,
                        active_card_cast_uids: None,
                    })
            })
            .flat_map(|(_, beh, beh_target)| behavior::execute(fight, entity_uid, beh, *beh_target))
            .collect()
    }

    pub fn on_dead(&self, fight: &Fight, entity_uid: i64) -> Vec<Event> {
        let buff_mgr = BuffMgr::default();
        let ex_point_mgr = EntityMgr::default();
        let bloodtithe = BloodtitheState::default();
        self.behaviours
            .iter()
            .filter(|(cond, _, _)| {
                matches!(cond.hook, condition::Hook::Dead)
                    && cond.check(ConditionEval {
                        fight,
                        buff_mgr: &buff_mgr,
                        ex_point_mgr: &ex_point_mgr,
                        bloodtithe: &bloodtithe,
                        caster_uid: entity_uid,
                        target_uid: entity_uid,
                        condition_target: 0,
                        has_trigger_state: false,
                        active_card_cast_uids: None,
                    })
            })
            .flat_map(|(_, beh, beh_target)| behavior::execute(fight, entity_uid, beh, *beh_target))
            .collect()
    }
}
