mod apply;
pub mod buff_act;
pub mod buff_act_type;
pub mod buff_action;
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
}

impl Buff {
    pub fn fire_hook(&self, hook: Hook, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| a.hook == hook)
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }
}
