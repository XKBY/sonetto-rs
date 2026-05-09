//! Handlers for buff_act 1041 RaspberryBigSkill + 1042 Raspberry - Rubuska Shadow
//! Cloak mechanic (Shadow Friend HP conversion + accumulator feeding Shadow Cloak /
//! Crit DMG buffs).

use sonettobuf::{ActEffect, BuffActInfo};

use crate::state::battle::{
    fight_step::ActEffectBuilder, heroes::rubuska, skill::SkillExecutor, types::effects::EffectType,
};

use super::{EffectContext, monitor_continue::queue_monitor_triggers};

pub const BUFF_ACT_ID_RASPBERRY: i32 = 1042;
#[allow(dead_code)]
pub const BUFF_ACT_ID_RASPBERRY_BIG_SKILL: i32 = 1041;

/// Returns `(act_id, rate_permille)` from the Raspberry feature, or
/// `None` if the buff isn't carrying it.
pub fn buff_get_raspberry_params(buff_id: i32) -> Option<(i32, i32)> {
    let parts = super::find_feature_parts(buff_id, "Raspberry")?;
    let act_id: i32 = parts.first()?.trim().parse().ok()?;
    let rate: i32 = parts.get(1)?.trim().parse().ok()?;
    Some((act_id, rate))
}

/// Behavior: RaspberryAddCount - accumulates HP into the Raspberry counter
/// and emits the 109/350/108 triplet per target.
/// MonitorContinueChannel trigger scanning is handled separately in monitor_continue.rs.
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

    let buff_uid = ctx
        .buff_mgr()
        .get(ctx.target_uid())
        .iter()
        .find(|buff| rubuska::buff_is_shadow_cloak_accumulator(buff.buff_id))
        .map(|b| b.uid)
        .unwrap_or(0);

    Ok(vec![
        ActEffectBuilder::current_hp_change(ctx.target_uid(), current_hp),
        ActEffect {
            effect_type: Some(EffectType::BuffActInfoUpdate as i32),
            target_id: Some(ctx.target_uid()),
            reserve_id: Some(buff_uid),
            buff_act_info: Some(BuffActInfo {
                act_id: Some(BUFF_ACT_ID_RASPBERRY),
                param: vec![accum, max_cap],
                ..Default::default()
            }),
            ..Default::default()
        },
        ActEffectBuilder::max_hp_change(ctx.target_uid(), new_max_hp, Some(BUFF_ACT_ID_RASPBERRY)),
    ])
}
