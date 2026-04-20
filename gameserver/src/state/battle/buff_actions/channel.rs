use crate::state::battle::skill::SkillExecutor;

use super::EffectContext;

/// Queues MonitorContinueChannel trigger skills for any ally that has one.
/// The trigger list is pre-built at battle start in ChannelState::init()
/// so this is just a vec extend — no scanning at runtime.
pub fn queue_monitor_triggers(ctx: &EffectContext, executor: &mut SkillExecutor) {
    for &(uid, skill_id) in &ctx.mechanics.channel.monitor_triggers {
        executor.pending_monitor_triggers.push((uid, skill_id));
    }
}
