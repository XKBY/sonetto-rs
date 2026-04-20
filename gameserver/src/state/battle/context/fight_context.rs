use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::manager::traits::Manager;
use crate::state::battle::mechanics::Mechanics;

use sonettobuf::Fight;

pub struct FightContext<'a> {
    pub fight: &'a mut Fight,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
}

impl<'a> FightContext<'a> {
    pub fn sync(&mut self) {
        self.managers.entity_mgr.rebuild_cache(self.fight);
        self.managers.calculate_mgr.update_cache(self.fight);
    }

    pub fn on_round_end(&mut self) {
        self.managers.buff_mgr.on_round_end();
        self.managers.calculate_mgr.on_round_end();
        self.sync();
    }
}
