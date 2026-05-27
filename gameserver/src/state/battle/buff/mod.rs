mod apply;
pub mod apply_skill;
pub mod buff_act;
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
    pub actions: Vec<buff_act::BuffAction>,
    pub proto_buff: sonettobuf::BuffInfo,
    pub buff_type: Option<config::skill_bufftype::SkillBufftype>,
    pub layer: i32,
    pub refresh_policy: RefreshPolicy,
    pub attr_bonus_refs: Vec<(i64, i32, i32)>,
}

impl Buff {
    pub fn fire_hook(&mut self, hook: Hook, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let mut events = Vec::new();
        for a in self.actions.iter().filter(|a| a.hooks.contains(&hook)) {
            let (evts, refs) = a.execute(fight, managers, entity_uid, self.buff_id);
            events.extend(evts);
            self.attr_bonus_refs.extend(refs);
        }
        events
    }
}
