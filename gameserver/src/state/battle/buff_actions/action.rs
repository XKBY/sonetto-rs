//! Trait + context bundle + registry every buff_action module
//! implements / consults.
//!
//! Each `buff_act.type` string in `data/excel2json/buff_act.json` is
//! routed through the dispatcher in `buff_actions/mod.rs::dispatch_feature`.
//! Per-cluster modules (e.g. `healing.rs`, `attributes.rs`) expose a
//! unit struct that implements [`BuffAction`]. The dispatcher iterates
//! [`BUFF_ACTION_REGISTRY`] and picks the first cluster whose
//! `execute` returns `Some(...)`.
//!
//! Two-stage pipeline:
//! - `BeforeBuffAdd` runs before the BuffAdd ActEffect is emitted (used
//!   only by buff_acts that need to broadcast HP changes synchronously
//!   with the buff add — `Attr`, `EachChangeAttr`, `LostHpCountAddBuff`).
//! - `AfterBuffAdd` runs after the BuffAdd is emitted (the default for
//!   every other buff_act).
//!
//! Return semantics:
//! - `Some(result)` — this cluster owns `act_type` AND has something
//!   to emit in this `stage`.
//! - `None` — either foreign (cluster doesn't own `act_type`) or
//!   owned but no-op for this `stage`. The registry iteration falls
//!   through to the next cluster.

use super::EffectContext;
use super::super::skill::SkillExecutor;
use super::result::ActionResult;
use super::{attr, bootstrap, halo, heal, hp, markers, no_op, shield};

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
/// Each cluster pattern-matches the strings it owns inside `execute`
/// and returns `None` for foreign act_types or stages it has nothing
/// to emit on. The registry's iteration falls through to the next
/// cluster on `None`.
pub(super) trait BuffAction {
    fn execute(
        &self,
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult>;
}

/// Ordered registry of every buff_action cluster the dispatcher
/// consults. First cluster to return `Some(...)` wins.
///
/// Order is informed by the prior dispatcher's match arm sequence —
/// `attr` first because it claims the catch-all `Attr` /
/// `EachChangeAttr` BeforeBuffAdd path; `no_op` last because it
/// catches all the placeholder act_types that every other cluster
/// rejects.
pub(super) const BUFF_ACTION_REGISTRY: &[&dyn BuffAction] = &[
    &attr::Attributes,
    &heal::Healing,
    &shield::Shield,
    &markers::Markers,
    &halo::Halo,
    &hp::Hp,
    &bootstrap::Bootstrap,
    &no_op::NoOp,
];
