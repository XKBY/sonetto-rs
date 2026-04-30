//! NoOp — buff_acts that emit a single `effect_none(target)` slot.
//! Their real semantics fire on different triggers (e.g.
//! `AddPassiveSkills` is consumed by the passive collector at
//! round-open) or live in untouched parts of the engine.

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

pub(super) struct NoOpParams {
    pub target_uid: i64,
}

pub(super) struct NoOpHandler;

impl BuffActionHandler for NoOpHandler {
    type Params = NoOpParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        stage == BuffStage::AfterBuffAdd
            && matches!(
                act_type,
                "FixAttrBySubBuffLayer"
                    | "AddPassiveSkills"
                    | "SubBuff"
                    | "Bullet"
                    | "CreateMaxHpAdditionalDamageAndRemove"
                    | "LifeAttackFixRate"
                    | "AddBuffByOtherExSkill"
            )
    }

    fn parse(&self, _parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        NoOpParams {
            target_uid: ctx.effect_ctx.target,
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::none(params.target_uid)
    }
}
