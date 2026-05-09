//! Trait + context bundle + registries for the buff_action dispatch
//! pipeline. The dispatcher in `mod.rs` walks
//! [`BUFF_HANDLER_REGISTRY`] (per-stage handlers) first, then falls
//! through to [`BUFF_ACTION_REGISTRY`] (legacy cluster handlers).

use super::super::skill::SkillExecutor;
use super::EffectContext;
use super::result::ActionResult;
use super::{
    add_buff_both, attr, bootstrap, dot, halo, heal, hp, markers, no_op, probability_add_buff,
    shield,
};
use crate::state::battle::manager::buff_mgr::BuffInstance;

/// When a buff feature runs relative to the BuffAdd emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffStage {
    BeforeBuffAdd,
    AfterBuffAdd,
    OnDeath,
    RoundStartCure,
    RoundStartCard,
    BeAttackedDefensive,
    OnCast,
    BeAttackedReactive,
    PostSkill,
    RoundEndDot,
    OverflowHandler,
    RoundEndInjuryBank,
}

impl BuffStage {
    /// Canonical `effectTime` value for staged dispatch.
    pub fn effect_time(self) -> Option<i32> {
        match self {
            Self::BeforeBuffAdd | Self::AfterBuffAdd => Some(0),
            Self::OnDeath => Some(12),
            Self::RoundStartCure => Some(102),
            Self::RoundStartCard => Some(105),
            Self::BeAttackedDefensive => Some(207),
            Self::OnCast => Some(208),
            Self::BeAttackedReactive => Some(209),
            Self::PostSkill => Some(212),
            Self::RoundEndDot => Some(302),
            Self::OverflowHandler => Some(305),
            Self::RoundEndInjuryBank => Some(307),
        }
    }
}

/// Shared mutable context for stage dispatch entry points.
pub struct DispatchCtx<'a, 'ctx> {
    pub effect_ctx: &'a mut EffectContext<'ctx>,
    pub executor: &'a mut SkillExecutor,
    pub condition_id: i32,
    pub has_bloodpool: bool,
    pub is_synthetic: bool,
}

impl<'a, 'ctx> DispatchCtx<'a, 'ctx> {
    pub fn new(effect_ctx: &'a mut EffectContext<'ctx>, executor: &'a mut SkillExecutor) -> Self {
        Self {
            effect_ctx,
            executor,
            condition_id: 0,
            has_bloodpool: false,
            is_synthetic: false,
        }
    }
}

/// Mutable handles + per-feature invocation data threaded into every
/// handler. `condition_id` carries the parent skill's condition
/// (used by Attr handlers to gate EnterFight/BattleStart broadcasts).
#[allow(dead_code)]
pub(super) struct BuffActCtx<'a, 'ctx> {
    pub effect_ctx: &'a mut EffectContext<'ctx>,
    pub executor: &'a mut SkillExecutor,
    pub buff_id: i32,
    pub owner_uid: i64,
    pub carrier: Option<BuffInstance>,
    pub condition_id: i32,
    pub has_bloodpool: bool,
    pub is_synthetic: bool,
}

/// Legacy cluster contract — one impl per cluster claims many
/// (act_type, stage) tuples in its `execute` body. New code should
/// prefer [`BuffActionHandler`] instead.
pub(super) trait BuffAction {
    fn execute(
        &self,
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult>;
}

/// Per-handler contract for ONE (act_type, stage) tuple. Body is
/// `parse → execute → steps`.
///
/// - `parse` extracts typed params from `parts` and snapshots ctx.
/// - `execute` applies state mutations (default no-op) and may fill
///   in `Params` fields that depend on side-effect ordering.
/// - `steps` builds ActEffects from the (possibly mutated) params.
pub(super) trait BuffActionHandler {
    type Params;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool;
    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params;
    fn execute(&self, _params: &mut Self::Params, _ctx: &mut BuffActCtx<'_, '_>) {}
    fn steps(&self, params: Self::Params, ctx: &BuffActCtx<'_, '_>) -> ActionResult;
}

/// Object-safe wrapper. Every [`BuffActionHandler`] gets a blanket
/// impl that runs `parse → execute → steps` in order.
pub(super) trait BuffActionRunner {
    fn run(
        &self,
        act_type: &str,
        stage: BuffStage,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
    ) -> Option<ActionResult>;
}

impl<H: BuffActionHandler> BuffActionRunner for H {
    fn run(
        &self,
        act_type: &str,
        stage: BuffStage,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
    ) -> Option<ActionResult> {
        if !self.matches(act_type, stage) {
            return None;
        }
        let mut params = self.parse(parts, ctx);
        self.execute(&mut params, ctx);
        Some(self.steps(params, ctx))
    }
}

/// Legacy cluster registry. First match wins.
pub(super) const BUFF_ACTION_REGISTRY: &[&dyn BuffAction] = &[];

/// Per-stage handler registry. Walked BEFORE the cluster registry so
/// migrated handlers take precedence.
pub(super) const BUFF_HANDLER_REGISTRY: &[&dyn BuffActionRunner] = &[
    &hp::LostHpCountAddBuffBefore,
    &hp::LostHpCountAddBuffAfter,
    &halo::MasterHaloHandler,
    &halo::SlaveHaloHandler,
    &shield::ShieldHandler,
    &heal::CureUpByLostHpHandler,
    &heal::ReviveHandler,
    &probability_add_buff::ProbabilityAddBuffHandler,
    &bootstrap::RaspberryHandler,
    &bootstrap::RaspberryBigSkillHandler,
    &bootstrap::MonitorContinueChannelHandler,
    &no_op::NoOpHandler,
    &markers::MarkerHandler,
    &attr::AttrBeforeHandler,
    &attr::AttrAfterHandler,
    &attr::EachChangeAttrBeforeHandler,
    &attr::EachChangeAttrAfterHandler,
    &attr::AttrFromEntityHandler,
    &attr::AttrOnlyCalDamageHandler,
    &add_buff_both::AddBuffBothHandler,
    &dot::PoisonHandler,
    &dot::DeadlyPoisonHandler,
    &dot::BurnHandler,
];

pub(super) fn run_registered_handler(
    act_type: &str,
    stage: BuffStage,
    parts: &[&str],
    ctx: &mut BuffActCtx<'_, '_>,
) -> Option<ActionResult> {
    for handler in BUFF_HANDLER_REGISTRY {
        if let Some(result) = handler.run(act_type, stage, parts, ctx) {
            return Some(result);
        }
    }
    for cluster in BUFF_ACTION_REGISTRY {
        if let Some(result) = cluster.execute(act_type, parts, ctx, stage) {
            return Some(result);
        }
    }
    None
}
