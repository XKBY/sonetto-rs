use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::manager::traits::Manager;
use crate::state::battle::mechanics::Mechanics;

use rand::rngs::StdRng;
use sonettobuf::Fight;
use std::ptr::NonNull;

#[derive(Copy, Clone)]
struct RngPtr(NonNull<StdRng>);

// SAFETY: this is a non-owning pointer to the round-local RNG. Access still requires
// `&mut FightContext`, so callers maintain exclusivity when dereferencing it.
unsafe impl Send for RngPtr {}

pub struct FightContext<'a> {
    pub fight: &'a mut Fight,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    rng: Option<RngPtr>,
}

impl<'a> FightContext<'a> {
    pub fn new(
        fight: &'a mut Fight,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
    ) -> Self {
        Self {
            fight,
            managers,
            mechanics,
            rng: None,
        }
    }

    pub fn with_rng(mut self, rng: &mut StdRng) -> Self {
        self.rng = Some(RngPtr(NonNull::from(rng)));
        self
    }

    pub fn rng_ptr(&self) -> Option<NonNull<StdRng>> {
        self.rng.map(|ptr| ptr.0)
    }

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
