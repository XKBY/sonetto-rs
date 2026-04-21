use sonettobuf::FightStep;

use crate::state::battle::{
    context::FightContext, passives::collector::CollectedPassives, trigger::combat::TriggerEvent,
};

mod blood_pool_sync;
mod blood_value_use_skill;
mod card_energy_sync;
mod combat_passives;
mod ex_point_sync;
mod hp_sync;

pub use blood_pool_sync::BloodPoolSyncPass;
pub use blood_value_use_skill::{BloodValueUseSkillPass, sync_blood_value_baseline};
pub use card_energy_sync::{CardEnergySyncPass, build_belief_gain_step};
pub use combat_passives::CombatPassivesPass;
pub use ex_point_sync::ExPointSyncPass;
pub use hp_sync::HpSyncPass;

/// One pass in the post-skill trigger pipeline.
/// Passes run in declaration order.
pub trait TriggerPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        collected: &CollectedPassives,
    ) -> Vec<FightStep>;
}
