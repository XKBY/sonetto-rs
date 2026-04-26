//! Bootstrap buff_action — handler for the buff_act types that
//! advertise battle-init mechanics (raspberry pool, monitor channel
//! triggers). They're the same set used by the
//! `is_bootstrap_relevant_buff` filter in `battle_gen/generator.rs`
//! that decides whether to seed initial replay state.
//!
//! At apply-time these emissions are bookkeeping only — the actual
//! mechanic state lives in `mechanics/bloodtithe.rs` and
//! `mechanics/channel.rs` and is mutated through dedicated paths.
//!
//! Variants owned:
//! * `Raspberry` — emits `effect_none(target)` so the dispatcher's
//!   feature loop accounts for the slot. The raspberry accumulator
//!   itself is fed by `RaspberryAddCount` (skill_behavior side).
//! * `RaspberryBigSkill` — empty; the big-skill side of the same
//!   mechanic. No emission needed at apply time.
//! * `MonitorContinueChannel` — empty; the monitor-channel registry
//!   in `mechanics::channel` is built once at battle start, not on
//!   per-buff apply.

use super::action::{BuffActCtx, BuffAction, BuffStage};
use super::result::ActionResult;

pub(super) struct Bootstrap;

impl BuffAction for Bootstrap {
    fn execute(
        &self,
        act_type: &str,
        _parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult> {
        if stage == BuffStage::BeforeBuffAdd {
            return None;
        }
        match act_type {
            "Raspberry" => Some(ActionResult::none(ctx.effect_ctx.target)),
            "RaspberryBigSkill" | "MonitorContinueChannel" => Some(ActionResult::empty()),
            _ => None,
        }
    }
}
