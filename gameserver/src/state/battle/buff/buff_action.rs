use crate::state::battle::{effect::condition::Hook, event::Event, manager::fight_data_mgr::Managers};
use sonettobuf::Fight;
use super::buff_act_type::BuffActType;
use super::buff_act;

#[derive(Debug, Clone)]
pub struct BuffAction {
    pub act_type: BuffActType,
    pub hook: Hook,
    pub params: String,
}

impl BuffAction {
    pub fn new(id: i32) -> Option<Self> {
        Some(Self { act_type: BuffActType::from_id(id)?, hook: Hook::EnterFight, params: String::new() })
    }

    pub fn execute(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64, carrier_buff_id: i32) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
        buff_act::execute(self, fight, managers, entity_uid, carrier_buff_id)
    }
}
