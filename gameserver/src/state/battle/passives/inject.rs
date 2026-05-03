//! Generic walker that injects same-side reactive emissions into an
//! enemy-side subtree of `ActEffect`s.
//!
//! ## Why this exists
//!
//! When an enemy SKILL emission appears anywhere in a fightStep
//! subtree (typically a boss-passive bundle's `act_effect` Vec), some
//! same-side passives need to react to it inline. LIVE materializes
//! these reactive wrappers INSIDE the enemy SKILL's `act_effect`, not
//! as siblings at the parent level — Sentinel's MonitorContinue
//! reactive emerges next to the enemy emission's BuffUpdate packets,
//! and ally `BeAttacked` reactives (Nautika `31200222`, Rubuska
//! `31250144`, etc.) appear there too.
//!
//! ## What this module owns
//!
//! Just the structural walk: depth-first recursion over the subtree,
//! plus the predicate that identifies an enemy SKILL emission slot
//! (`effect_type == FightStep`, `act_type == Skill`, `from_id < 0`).
//! All mechanism-specific decisions — which holder to pick, which
//! buff to consume, where in the matched step's `act_effect` to
//! splice the wrapper — live in the per-injector closure.
//!
//! ## What lives elsewhere
//!
//! * `heroes/sentinel.rs::inject_monitor_continue_reactive_into_enemy_skill`
//!   — Sentinel's MonitorContinue logic (holder lookup, buff
//!   consumption, splice index).
//! * (planned) `trigger/passes/ally_be_attacked.rs` — ally
//!   `BeAttacked` reactive emission for cross-side damage from
//!   nested boss skills.

use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::context::FightContext;
use crate::state::battle::passives::collector::CollectedPassives;
use crate::state::battle::types::effects::EffectType;

/// Recursively walk an enemy-side `ActEffect` subtree. For every
/// nested fightStep that wraps an enemy SKILL emission, invoke
/// `inject(ctx, collected, step)`. The injector mutates
/// `step.act_effect` directly to splice in any reactive wrappers.
///
/// Recursion is depth-first so nested matches resolve before the
/// host step's own injection — this keeps insert indices stable
/// regardless of which level the reactive is added at.
pub fn inject_ally_reactives_into_enemy_subtree<F>(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    subtree: &mut Vec<ActEffect>,
    inject: &F,
) where
    F: Fn(&mut FightContext<'_>, &CollectedPassives, &mut FightStep),
{
    for effect in subtree.iter_mut() {
        let Some(step) = effect.fight_step.as_mut() else {
            continue;
        };

        // Depth-first: handle nested matches before this level.
        inject_ally_reactives_into_enemy_subtree(ctx, collected, &mut step.act_effect, inject);

        if effect.effect_type != Some(EffectType::FightStep as i32)
            || step.act_type != Some(fight_step::ActType::Skill as i32)
            || step.from_id.unwrap_or(0) >= 0
        {
            continue;
        }

        inject(ctx, collected, step);
    }
}
