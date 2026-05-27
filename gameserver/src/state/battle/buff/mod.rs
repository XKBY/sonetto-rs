mod apply;
pub mod apply_skill;
pub mod buff_act;
pub mod buff_act_type;
pub mod buff_action;
pub mod helper;
pub mod utils;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshPolicy {
    ReplaceOnExcludedOverlap,
    ReplaceOnSelfRefresh,
    #[default]
    UpdateInPlace,
}

pub use apply::{apply_buff_effects, pre_buff_effects};

use crate::state::battle::{effect::condition::Hook, event::Event, manager::fight_data_mgr::Managers};
use sonettobuf::Fight;

#[derive(Debug, Clone)]
pub struct Buff {
    pub buff_id: i32,
    pub duration: i32,
    pub stacks: i32,
    pub actions: Vec<buff_action::BuffAction>,
    pub proto_buff: sonettobuf::BuffInfo,
    pub buff_type: Option<config::skill_bufftype::SkillBufftype>,
    pub layer: i32,
    pub refresh_policy: RefreshPolicy,
    pub attr_bonus_refs: Vec<(i64, i32, i32)>, // (uid, attr_id, amount)
}

impl Buff {
    pub fn fire_hook(&mut self, hook: Hook, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let mut events = Vec::new();
        // Since we need to mutate `self.attr_bonus_refs`, but we also iterate over `self.actions`, we can avoid borrow checker issues by iterating over indices, or by cloning actions, but `actions.iter()` should be fine since we only mutate a different field.
        // The subagent on task 2 might have attempted to rewrite this, make sure it matches the target:
        for a in self.actions.iter().filter(|a| a.hook == hook) {
            let (evts, refs) = a.execute(fight, managers, entity_uid, self.buff_id);
            events.extend(evts);
            self.attr_bonus_refs.extend(refs);
        }
        events
    }
}
