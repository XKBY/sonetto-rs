//! Trait every condition cluster module implements.
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
//! The dispatcher in `condition/parser.rs::parse_single` chains
//! `Cluster::parse` calls with `.or_else(...)`. The dispatcher in
//! `condition/mod.rs::ConditionEval::check` does the same with
//! `Cluster::check`. Both return `None` from the cluster method to
//! mean "I don't own this variant; try the next cluster"; the
//! dispatcher's chain handles fall-through.

use super::ConditionEval;
use super::ConditionType;

pub(super) trait Condition {
    /// Try to parse the parts (split on `'#'`) into a `ConditionType`
    /// variant this cluster owns. The first element is the raw `id`
    /// (some clusters parse it; most rely on `cond_type`).
    fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType>;

    /// Try to evaluate `condition` against the given evaluation
    /// context. Returns `Some(bool)` if this cluster owns the
    /// variant, `None` if foreign.
    fn check(condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool>;
}
