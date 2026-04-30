//! Bootstrap — battle-init markers (Raspberry, MonitorContinueChannel).
//! Apply-time emissions are bookkeeping; real state lives in `mechanics/`.

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

pub(super) struct BootstrapParams {
    pub target_uid: i64,
}

pub(super) struct RaspberryHandler;

impl BuffActionHandler for RaspberryHandler {
    type Params = BootstrapParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Raspberry" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        BootstrapParams {
            target_uid: ctx.effect_ctx.target,
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::none(params.target_uid)
    }
}

pub(super) struct RaspberryBigSkillHandler;

impl BuffActionHandler for RaspberryBigSkillHandler {
    type Params = ();

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "RaspberryBigSkill" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {}

    fn steps(&self, _params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::empty()
    }
}

pub(super) struct MonitorContinueChannelHandler;

impl BuffActionHandler for MonitorContinueChannelHandler {
    type Params = ();

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "MonitorContinueChannel" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {}

    fn steps(&self, _params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::empty()
    }
}
