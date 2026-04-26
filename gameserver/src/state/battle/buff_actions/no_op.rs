//! NoOp buff_action — handler for the buff_act types that the
//! dispatcher previously routed to a single
//! `ActionResult::none(target)` arm. None of these emit anything
//! meaningful at apply time; their actual semantics either fire on
//! a different trigger (e.g. `AddPassiveSkills` is consumed by the
//! passive collector during round-open, not on buff apply) or live
//! in untouched parts of the engine that haven't migrated yet.
//!
//! Variants owned:
//! `FixAttrBySubBuffLayer`, `AddPassiveSkills`, `SubBuff`, `Bullet`,
//! `CreateMaxHpAdditionalDamageAndRemove`, `LifeAttackFixRate`,
//! `AddBuffByOtherExSkill`, `ProbabilityAddBuff`, `Poison`.

use super::action::{BuffAction, BuffActCtx, BuffStage};
use super::result::ActionResult;

pub(super) struct NoOp;

impl BuffAction for NoOp {
    fn execute(
        _act_type: &str,
        _parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> ActionResult {
        if stage == BuffStage::BeforeBuffAdd {
            return ActionResult::empty();
        }
        ActionResult::none(ctx.effect_ctx.target)
    }
}
