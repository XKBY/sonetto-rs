mod apply;
pub mod buff_act;
pub mod buff_act_type;
pub mod buff_action;
pub mod utils;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshPolicy {
    /// Re-applying this buff removes an active sibling selected through
    /// bufftype.exclude_types, then emits a fresh add.
    ReplaceOnExcludedOverlap,

    /// Re-applying this buff removes the same buff_id, then emits a fresh
    /// add.
    ReplaceOnSelfRefresh,

    /// Re-applying this buff keeps the existing uid and emits update
    /// semantics.
    #[default]
    UpdateInPlace,
}

pub use apply::{apply_buff_effects, pre_buff_effects};

use buff_action::BuffHook;
use crate::state::battle::{event::Event, manager::fight_data_mgr::Managers};
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
    pub fn on_enter_fight(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::EnterFight))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_dead(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::Dead))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_battle_start(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::BattleStart))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_round_end(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::RoundEnd))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_use_card(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::UseCard))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_move_card(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::MoveCard))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_compose_card(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::ComposeCard))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }

    pub fn on_buff_add(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.actions.iter()
            .filter(|a| matches!(a.hook, BuffHook::BuffAdd))
            .flat_map(|a| a.execute(fight, managers, entity_uid, self.buff_id))
            .collect()
    }
}
