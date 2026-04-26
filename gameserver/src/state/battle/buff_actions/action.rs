//! Trait + context bundle every buff_action module implements.
//!
//! Each `buff_act.type` string in `data/excel2json/buff_act.json` is
//! routed through the dispatcher in `buff_actions/mod.rs::dispatch_feature`.
//! Per-cluster modules (e.g. `healing.rs`, `attributes.rs`) expose a unit
//! struct that implements [`BuffAction`]. The dispatcher's match arms call
//! `Cluster::execute(act_type, parts, ctx, stage)`.
//!
//! Two-stage pipeline:
//! - `BeforeBuffAdd` runs before the BuffAdd ActEffect is emitted (used
//!   only by buff_acts that need to broadcast HP changes synchronously
//!   with the buff add — `Attr`, `EachChangeAttr`, `LostHpCountAddBuff`).
//! - `AfterBuffAdd` runs after the BuffAdd is emitted (the default for
//!   every other buff_act).
//!
//! The same struct's `execute` is called for both stages; the impl
//! filters by `stage` if the action behaves differently. Most clusters
//! return `ActionResult::empty()` for the BeforeBuffAdd stage.

use super::EffectContext;
use super::result::ActionResult;
use super::super::skill::SkillExecutor;

/// When a buff feature runs relative to the BuffAdd emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffStage {
    BeforeBuffAdd,
    AfterBuffAdd,
}

/// Bundle of mutable handles + per-feature invocation data threaded
/// into every buff_action's `execute`. Built once per feature inside
/// `dispatch_feature`. Fields are `pub(super)` so sibling cluster
/// modules can read them directly.
///
/// `condition_id` carries the skill's `condition_id` from the parent
/// skill emission; `BeforeBuffAdd` `Attr` handlers use it to gate
/// EnterFight/BattleStart HP broadcasts (see `mod.rs`).
#[allow(dead_code)]
pub(super) struct BuffActCtx<'a, 'ctx> {
    pub effect_ctx: &'a mut EffectContext<'ctx>,
    pub executor: &'a mut SkillExecutor,
    pub buff_id: i32,
    pub condition_id: i32,
    pub has_bloodpool: bool,
}

/// Contract for every buff_action cluster module.
///
/// The dispatcher's match arms route `act_type` strings to the owning
/// cluster's `execute`. Each cluster pattern-matches the strings it
/// owns at the top of `execute` and falls through to `ActionResult`
/// defaults for off-pattern strings (defensive — the dispatcher
/// won't actually feed off-pattern input).
pub(super) trait BuffAction {
    fn execute(
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> ActionResult;
}
