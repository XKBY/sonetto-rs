//! Heal action — handles the two heal-shaped skill_behavior variants:
//! `BehaviorType::Heal { rate }` (covers `Heal` and `HealCantCrit` in
//! the parser) and `BehaviorType::HealByTwoAttr { missing_percent,
//! caster_hp_percent }` (the `HealByTwoAttr` variant which scales by
//! the caster's max HP and the target's missing HP).
//!
//! Both delegate to one-shot helpers in `buff_actions::heal`. The
//! action module owns the dispatch wiring and the effect-context
//! construction; the per-formula math lives in the buff_actions
//! handler so it can also be invoked from non-skill paths (e.g. cure
//! features on buff apply).

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::buff_actions::{EffectContext, heal as heal_handler};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Heal action — routes from `BehaviorType::Heal` and
/// `BehaviorType::HealByTwoAttr`.
pub(super) struct Heal;

impl BehaviorAction for Heal {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        let mut effect_ctx = EffectContext::new(
            ctx.behavior_ctx.fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.target,
        );
        match behavior {
            BehaviorType::Heal { rate } => Ok(heal_handler::heal(&mut effect_ctx, *rate)),
            BehaviorType::HealByTwoAttr {
                missing_percent,
                caster_hp_percent,
            } => Ok(heal_handler::heal_by_two_attr(
                &mut effect_ctx,
                *missing_percent,
                *caster_hp_percent,
            )),
            _ => Ok(vec![]),
        }
    }
}
