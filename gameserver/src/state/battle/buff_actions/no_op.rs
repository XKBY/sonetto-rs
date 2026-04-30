//! NoOp — buff_acts that emit a single `effect_none(target)` slot.
//! Their real semantics fire on different triggers (e.g.
//! `AddPassiveSkills` is consumed by the passive collector at
//! round-open) or live in untouched parts of the engine.

use super::action::{BuffActCtx, BuffAction, BuffStage};
use super::result::ActionResult;

pub(super) struct NoOp;

impl BuffAction for NoOp {
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
            "FixAttrBySubBuffLayer"
            | "AddPassiveSkills"
            | "SubBuff"
            | "Bullet"
            | "CreateMaxHpAdditionalDamageAndRemove"
            | "LifeAttackFixRate"
            | "AddBuffByOtherExSkill" => Some(ActionResult::none(ctx.effect_ctx.target)),
            _ => None,
        }
    }
}
