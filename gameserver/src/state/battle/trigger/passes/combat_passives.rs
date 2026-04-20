use sonettobuf::FightStep;

use crate::state::battle::{
    context::FightContext,
    passives::collector::CollectedPassives,
    trigger::combat::{TriggerEvent, run_combat_passives_pass},
};

use super::TriggerPass;

pub struct CombatPassivesPass;

impl TriggerPass for CombatPassivesPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        run_combat_passives_pass(ctx, collected, event)
    }
}
