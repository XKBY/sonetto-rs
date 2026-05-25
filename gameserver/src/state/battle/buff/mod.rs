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

    pub fn fire_hook_attr_fix(
        &self,
        _hook: Hook,
        _fight: &Fight,
        _managers: &Managers,
        _entity_uid: i64,
    ) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
        // Buff actions are not attr-fix behaviours. The eval pass returns an
        // empty map; this exists so the HookEntry dispatch shape is uniform
        // across Buff and SkillEffect payloads.
        std::collections::HashMap::new()
    }
}
