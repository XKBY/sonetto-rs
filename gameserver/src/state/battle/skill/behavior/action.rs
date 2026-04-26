//! Trait + context bundle every behavior action module implements.
//!
//! Each `BehaviorType` variant is owned by exactly one action module
//! under `behavior/`. The module exposes a unit struct (e.g.
//! `damage::Damage`, `heal::Heal`) that implements [`BehaviorAction`].
//! The dispatcher in `behavior/mod.rs::dispatch_impl` matches on the
//! enum variant and routes to the corresponding `Action::execute`.
//!
//! This trait gives every action a uniform signature:
//! `(behavior, ctx, condition) -> Result<Vec<ActEffect>>`. New actions
//! get added by (1) creating a module, (2) implementing the trait,
//! (3) adding one routing arm to the dispatcher's match.

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::ActEffect;

use super::super::executor::SkillExecutor;
use crate::state::battle::{
    context::behavior_context::BehaviorContext, manager::fight_data_mgr::Managers,
    mechanics::Mechanics, types::behavior::BehaviorType, types::condition::ConditionType,
};

/// Bundle of mutable handles + per-target invocation data threaded
/// into every action's `execute`. Fields are `pub(super)` so siblings
/// (action modules under `behavior/`) can read them directly without
/// an accessor explosion. The dispatcher constructs this once per
/// target inside `dispatch_impl`.
///
/// Some fields (e.g. `rng`, `condition_id`) are unused by the actions
/// migrated so far — they'll be used by upcoming AddBuff / AddBuffRanId
/// migrations. Suppress the dead-code warning until the surface is
/// fully migrated.
#[allow(dead_code)]
pub(super) struct ActionCtx<'a, 'ctx> {
    pub executor: &'a mut SkillExecutor,
    pub rng: &'a mut StdRng,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    pub behavior_ctx: &'a BehaviorContext<'ctx>,
    pub caster_uid: i64,
    pub target: i64,
    pub skill_id: i32,
    pub condition_id: i32,
}

/// Contract for every behavior action module. The trait is sealed to
/// the `behavior` module tree — each variant of `BehaviorType` has
/// exactly one implementation, and the dispatcher's match arms guard
/// which struct each variant routes to.
///
/// Implementations should pattern-match the variant they own at the
/// top of `execute` and return `Ok(vec![])` for any other variant
/// (defensive, but the dispatcher won't hand them off-pattern input).
pub(super) trait BehaviorAction {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Result<Vec<ActEffect>>;
}
