use sonettobuf::{ActEffect, BuffActInfo};

use crate::state::battle::{skill::SkillExecutor, types::effects::EffectType};

use super::{EffectContext, channel::queue_monitor_triggers};

/// Behavior: RaspberryAddCount — accumulates HP into the Raspberry counter
/// and emits the 109/350/108 triplet per target.
/// MonitorContinueChannel trigger scanning is handled separately in channel.rs.
pub fn add_count(
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    attr_id: i32,
    rate: i32,
) -> anyhow::Result<Vec<ActEffect>> {
    // Queue MonitorContinueChannel triggers on first accumulation tick.
    if ctx.mechanics.shadow_cloak.raspberry_accum == 0 && ctx.mechanics.channel.is_active() {
        queue_monitor_triggers(ctx, executor);
    }

    let attr_value = match attr_id {
        100 => ctx.target_hp(),
        101 => ctx.target_max_hp(),
        _ => 0,
    };

    let gain = attr_value * rate / 1000;
    if gain == 0 {
        return Ok(vec![]);
    }

    ctx.mechanics.shadow_cloak.raspberry_accum += gain;
    let accum = ctx.mechanics.shadow_cloak.raspberry_accum;
    let max_cap = ctx.mechanics.shadow_cloak.raspberry_max;

    let current_hp = ctx.target_hp();
    let base_max_hp = ctx
        .target_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let new_max_hp = base_max_hp + accum;

    let cfg = config::configs::get();
    let buff_uid = ctx
        .buff_mgr()
        .get(ctx.target_uid())
        .iter()
        .find(|b| {
            cfg.skill_buff
                .iter()
                .find(|sb| sb.id == b.buff_id)
                .map(|sb| sb.type_id == 31250151)
                .unwrap_or(false)
        })
        .map(|b| b.uid)
        .unwrap_or(0);

    Ok(vec![
        ActEffect {
            effect_type: Some(EffectType::CurrentHpChange as i32),
            target_id: Some(ctx.target_uid()),
            effect_num: Some(current_hp),
            ..Default::default()
        },
        ActEffect {
            effect_type: Some(EffectType::BuffActInfoUpdate as i32),
            target_id: Some(ctx.target_uid()),
            reserve_id: Some(buff_uid),
            buff_act_info: Some(BuffActInfo {
                act_id: Some(1042),
                param: vec![accum, max_cap],
                ..Default::default()
            }),
            ..Default::default()
        },
        ActEffect {
            effect_type: Some(EffectType::MaxHpChange as i32),
            target_id: Some(ctx.target_uid()),
            effect_num: Some(new_max_hp),
            buff_act_id: Some(1042),
            ..Default::default()
        },
    ])
}
