//! Trait every condition cluster module implements + the static
//! registry that drives dispatch.
//!
//! Each cluster module under `skill/condition/` exposes a unit struct
//! (e.g. `buff::Buff`, `combat::Combat`) that implements
//! [`Condition`]. The two methods are the cluster's two
//! responsibilities:
//!
//! * `parse` — given the parts of a raw condition string (split on
//!   `'#'`) and the resolved `cond_type`, return `Some(ConditionType)`
//!   if this cluster owns the variant, else `None`.
//! * `check` — given a parsed [`ConditionType`] and a
//!   [`super::ConditionEval`] context, return `Some(true|false)` if
//!   this cluster owns the variant, else `None`.
//!
//! Phase 4 dispatch: the parser and evaluator iterate
//! [`CONDITION_REGISTRY`] in declaration order and pick the first
//! cluster whose method returns `Some(...)`. Adding a new cluster is
//! one struct + impl + one entry in the registry — no dispatcher
//! match to maintain.

use super::ConditionEval;
use super::ConditionType;
use super::{bloodtithe, buff, career, combat, enter_fight, ex_point, life, misc};

pub(super) trait Condition {
    /// Try to parse the parts (split on `'#'`) into a `ConditionType`
    /// variant this cluster owns. The first element is the raw `id`
    /// (some clusters parse it; most rely on `cond_type`).
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType>;

    /// Try to evaluate `condition` against the given evaluation
    /// context. Returns `Some(bool)` if this cluster owns the
    /// variant, `None` if foreign.
    fn check(&self, condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool>;
}

/// Ordered registry of every cluster the dispatcher consults.
///
/// Order matters when two clusters could conceivably handle the same
/// `cond_type`/`ConditionType` — the first cluster that returns
/// `Some(...)` wins. `EnterFight` is intentionally first because it
/// claims the catch-all `EnterFight`/`None`/`CombatNone` variants
/// that other clusters defer to it on.
pub(super) const CONDITION_REGISTRY: &[&dyn Condition] = &[
    &enter_fight::EnterFight,
    &buff::Buff,
    &career::Career,
    &life::Life,
    &ex_point::ExPoint,
    &combat::Combat,
    &misc::Misc,
    &bloodtithe::Bloodtithe,
];
