use sonettobuf::FightStep;
use crate::state::battle::{
    context::FightContext,
    event::Event,
    fight_step::{ActEffectBuilder, FightStepBuilder},
};

pub fn apply_event(event: &Event, ctx: &mut FightContext) {
    match event {
        Event::ExPointChange { target, delta, .. } => {
            ctx.managers.entity_mgr.add_ex_point(*target, *delta);
        }
        Event::PowerChange { delta } => {
            ctx.managers.cloth_mgr.apply_power(ctx.fight, *delta);
        }
        Event::RemoveBuff { target_uid, buff_id } => {
            ctx.managers.buff_mgr.remove_buff(*target_uid, *buff_id);
        }
        _ => {}
    }
}

pub fn events_to_steps(events: &[Event]) -> Vec<FightStep> {
    events.iter().filter_map(|event| match event {
        Event::ExPointChange { target, delta, emit_step: true } => {
            Some(FightStepBuilder::effect()
                .with(ActEffectBuilder::ex_point_change(*target, *delta))
                .build())
        }
        Event::PowerChange { delta } => {
            Some(FightStepBuilder::effect()
                .with(ActEffectBuilder::power_change(None, *delta, None))
                .build())
        }
        _ => None,
    }).collect()
}
