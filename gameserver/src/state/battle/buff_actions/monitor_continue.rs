//! Handler for buff_act 1024 MonitorContinueChannel — Sentinel Dread Bullet trigger chain (post-enemy-action Expiation firing).

use crate::state::battle::skill::SkillExecutor;

use super::EffectContext;

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 1024;

pub fn buff_get_monitor_continue_channel_params(buff_id: i32) -> Option<(i32, i32, i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };
        if act_type != "MonitorContinueChannel" {
            continue;
        }
        let prerequisite_buff_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let monitor_buff_id = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let emit_effect_id = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let emit_skill_id = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        if emit_effect_id > 0 && emit_skill_id > 0 {
            return Some((
                prerequisite_buff_id,
                monitor_buff_id,
                emit_effect_id,
                emit_skill_id,
            ));
        }
    }
    None
}

/// Queues MonitorContinueChannel trigger skills for any ally that has one.
/// The trigger list is pre-built at battle start in ChannelState::init()
/// so this is just a vec extend — no scanning at runtime.
pub fn queue_monitor_triggers(ctx: &EffectContext, executor: &mut SkillExecutor) {
    for &(uid, skill_id) in &ctx.mechanics.channel.monitor_triggers {
        executor.pending_monitor_triggers.push((uid, skill_id));
    }
}
