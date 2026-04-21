use sonettobuf::FightStep;

use crate::state::battle::{
    context::FightContext, passives::collector::CollectedPassives, trigger::combat::TriggerEvent,
};

use super::TriggerPass;

pub struct HpSyncPass;

impl TriggerPass for HpSyncPass {
    /// Placeholder: HP sync trigger pass is intentionally not implemented yet.
    fn run(
        &self,
        _ctx: &mut FightContext<'_>,
        _event: &TriggerEvent,
        _collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        vec![]
    }
}
