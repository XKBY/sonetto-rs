use sonettobuf::FightStep;
use crate::state::battle::{
    context::FightContext,
    event::Event,
    fight_step::{ActEffectBuilder, FightStepBuilder},
};

pub fn apply_event(event: &Event, ctx: &mut FightContext) -> Option<FightStep> {
    match event {
        Event::ExPointChange { target, delta } => {
            ctx.managers.entity_mgr.add_ex_point(*target, *delta);
            Some(FightStepBuilder::effect()
                .with(ActEffectBuilder::ex_point_change(*target, *delta))
                .build())
        }
        Event::PowerChange { delta } => {
            ctx.managers.cloth_mgr.apply_power(ctx.fight, *delta);
            Some(FightStepBuilder::effect()
                .with(ActEffectBuilder::power_change(None, *delta, None))
                .build())
        }
        _ => None,
    }
}
