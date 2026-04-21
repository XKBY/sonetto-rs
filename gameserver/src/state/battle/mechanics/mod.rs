pub mod bloodtithe;
pub mod channel;
pub mod injury_counter;
pub mod magic_circle;
pub mod nuodika;
pub mod round_end;
pub mod shadowcloak;

use bloodtithe::BloodtitheState;
use channel::ChannelState;
use shadowcloak::ShadowCloakState;

use crate::state::battle::manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr};
use sonettobuf::{Fight, FightStep};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mechanics {
    pub bloodtithe: BloodtitheState,
    pub channel: ChannelState,
    pub shadow_cloak: ShadowCloakState,
}

impl Mechanics {
    pub fn new() -> Self {
        Self {
            bloodtithe: BloodtitheState::new(),
            channel: ChannelState::new(),
            shadow_cloak: ShadowCloakState::new(),
        }
    }

    pub fn init(&mut self, fight: &Fight) {
        self.channel.init(fight);
        self.shadow_cloak.init(fight);
    }

    pub fn on_bloodpool_init(&self) -> Option<FightStep> {
        self.bloodtithe.bloodpool_init_step()
    }

    pub fn on_pre_raspberry(&self) -> Option<FightStep> {
        self.bloodtithe.bloodtithe_sync_step()
    }

    pub fn on_raspberry(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &mut ExPointMgr,
    ) -> Option<FightStep> {
        self.bloodtithe
            .raspberry_step(fight, buff_mgr, ex_point_mgr, &mut self.shadow_cloak)
    }

    pub fn on_post_raspberry(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &ExPointMgr,
    ) -> Option<FightStep> {
        self.shadow_cloak.sync_step(fight, buff_mgr, ex_point_mgr)
    }

}
