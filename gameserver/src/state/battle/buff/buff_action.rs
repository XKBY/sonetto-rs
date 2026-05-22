use crate::state::battle::{event::Event, manager::fight_data_mgr::Managers};
use sonettobuf::Fight;
use super::buff_act_type::BuffActType;
use super::buff_act;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuffHook {
    #[default]
    EnterFight,
    Dead,
    BattleStart,
    RoundEnd,
    UseCard,
    MoveCard,
    ComposeCard,
    BuffAdd,
}

#[derive(Debug, Clone)]
pub struct BuffAction {
    pub act_type: BuffActType,
    pub hook: BuffHook,
    pub params: String,
}

impl BuffAction {
    pub fn new(id: i32) -> Option<Self> {
        Some(Self { act_type: BuffActType::from_id(id)?, hook: BuffHook::default(), params: String::new() })
    }

    pub fn execute(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64, carrier_buff_id: i32) -> Vec<Event> {
        buff_act::execute(self, fight, managers, entity_uid, carrier_buff_id)
    }
}
