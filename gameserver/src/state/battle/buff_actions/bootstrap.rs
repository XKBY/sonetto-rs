//! Bootstrap — battle-init markers (Raspberry, MonitorContinueChannel).
//! Apply-time emissions are bookkeeping; real state lives in `mechanics/`.

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
