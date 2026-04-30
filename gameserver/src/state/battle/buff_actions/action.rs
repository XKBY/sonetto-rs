//! Trait + context bundle + registries for the buff_action dispatch
//! pipeline. The dispatcher in `mod.rs` walks
//! [`BUFF_HANDLER_REGISTRY`] (per-stage handlers) first, then falls
//! through to [`BUFF_ACTION_REGISTRY`] (legacy cluster handlers).

use super::super::skill::SkillExecutor;
use super::EffectContext;
use super::result::ActionResult;
use super::{
    add_buff_both, attr, bootstrap, halo, heal, hp, markers, no_op, probability_add_buff, shield,
};

/// When a buff feature runs relative to the BuffAdd emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffStage {
    BeforeBuffAdd,
    AfterBuffAdd,
}

/// Mutable handles + per-feature invocation data threaded into every
/// handler. `condition_id` carries the parent skill's condition
/// (used by Attr handlers to gate EnterFight/BattleStart broadcasts).
#[allow(dead_code)]
pub(super) struct BuffActCtx<'a, 'ctx> {
    pub effect_ctx: &'a mut EffectContext<'ctx>,
    pub executor: &'a mut SkillExecutor,
    pub buff_id: i32,
    pub condition_id: i32,
    pub has_bloodpool: bool,
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
pub(super) const BUFF_ACTION_REGISTRY: &[&dyn BuffAction] = &[
    &attr::Attributes,
    &heal::Healing,
    &shield::Shield,
    &add_buff_both::AddBuffBothAction,
    &probability_add_buff::ProbabilityAddBuffAction,
    &markers::Markers,
    &bootstrap::Bootstrap,
    &no_op::NoOp,
];

/// Per-stage handler registry. Walked BEFORE the cluster registry so
/// migrated handlers take precedence.
pub(super) const BUFF_HANDLER_REGISTRY: &[&dyn BuffActionRunner] = &[
    &hp::LostHpCountAddBuffBefore,
    &hp::LostHpCountAddBuffAfter,
    &halo::MasterHaloHandler,
    &halo::SlaveHaloHandler,
];
