pub mod buff_replace;

use sonettobuf::Fight;
use crate::state::battle::{event::Event, manager::fight_data_mgr::Managers};
use super::{buff_action::BuffAction, buff_act_type::BuffActType};

pub fn execute(action: &BuffAction, fight: &Fight, managers: &mut Managers, entity_uid: i64, carrier_buff_id: i32) -> Vec<Event> {
    match action.act_type {
        BuffActType::_702BuffReplace => buff_replace::execute(fight, managers, entity_uid, &action.params, carrier_buff_id),
        other => {
            tracing::warn!("unimplemented buff act type: {:?}", other);
            vec![]
        }
    }
}
