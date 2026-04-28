//! Pure FightStep / ActEffect structural walkers and predicates.
//!
//! These helpers were originally defined as `&self` methods on
//! `FightRoundMgr` but reference no manager state — they only inspect
//! and mutate the FightStep tree. Hoisting them to a free-function
//! module makes them reusable by `mechanics/` modules without
//! requiring a trait adapter shim, and shrinks `round_mgr.rs`'s god
//! class.
//!
//! Naming follows the project convention: each function is a verb +
//! domain noun phrase that reads as the question it answers
//! (`step_contains_act_id`, `wrapped_skill_from_effect`).

use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    steps::trigger_embed,
    types::effects::EffectType,
};

/// True when `step.act_effect` carries any direct effect with the
/// matching `effect_type`. Does NOT recurse into nested fight_steps.
pub fn step_has_effect_type(step: &FightStep, effect_type: i32) -> bool {
    step.act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(effect_type))
}

/// True when `step` is a single-effect `Effect`-typed wrapper carrying
/// exactly the given marker. Used to identify standalone marker steps
/// that the round-end stripper drops as redundant.
pub fn is_standalone_effect_marker(step: &FightStep, effect_type: i32) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_effect.len() == 1
        && step
            .act_effect
            .first()
            .map(|effect| effect.effect_type == Some(effect_type))
            .unwrap_or(false)
}

/// True when `step` itself or any nested `fight_step` descendant
/// carries the matching `act_id`. Walks the entire subtree.
pub fn step_contains_act_id(step: &FightStep, act_id: i32) -> bool {
    step.act_id == Some(act_id)
        || step.act_effect.iter().any(|effect| {
            effect
                .fight_step
                .as_ref()
                .map(|child| step_contains_act_id(child, act_id))
                .unwrap_or(false)
        })
}

/// True when `step` or any descendant emits a `MagicCircleAdd`
/// effect. Used by the magic-circle inlining pass to detect bundles
/// that need their root wrapper unwrapped.
pub fn step_contains_magic_circle_add(step: &FightStep) -> bool {
    step.act_effect.iter().any(|effect| {
        effect.effect_type == Some(EffectType::MagicCircleAdd as i32)
            || effect
                .fight_step
                .as_ref()
                .map(|child| step_contains_magic_circle_add(child))
                .unwrap_or(false)
    })
}

/// If `host_step` carries a 162-wrapped child whose inner SKILL
/// emits a `MagicCircleAdd`, lift that child's `act_effect` content
/// into the host (replacing the wrapper at the same index). Returns
/// `true` when the lift happened. Matches the LIVE shape where
/// magic-circle root wrappers are unrolled into the host's effect
/// list rather than nested.
pub fn inline_magic_circle_root_wrapper(host_step: &mut FightStep) -> bool {
    let Some(idx) = host_step.act_effect.iter().position(|effect| {
        effect.effect_type == Some(162)
            && effect
                .fight_step
                .as_ref()
                .map(|step| {
                    step.act_type == Some(fight_step::ActType::Skill as i32)
                        && step_contains_magic_circle_add(step)
                })
                .unwrap_or(false)
    }) else {
        return false;
    };

    let Some(inner) = host_step
        .act_effect
        .remove(idx)
        .fight_step
        .filter(|step| step.act_type == Some(fight_step::ActType::Skill as i32))
    else {
        return false;
    };

    host_step.act_effect.splice(idx..idx, inner.act_effect);
    true
}

/// Index inside `host_step.act_effect` where trigger emissions should
/// be spliced. If the host has a `MagicCircleAdd` effect, insert
/// after it; otherwise fall through to the generic
/// `trigger_embed::find_trigger_insert_index` policy.
pub fn host_trigger_insert_index(host_step: &FightStep) -> usize {
    host_step
        .act_effect
        .iter()
        .position(|effect| effect.effect_type == Some(EffectType::MagicCircleAdd as i32))
        .map(|idx| idx + 1)
        .unwrap_or_else(|| trigger_embed::find_trigger_insert_index(&host_step.act_effect))
}

/// If `effect` is a 162-wrapped Skill emission (either directly or
/// through a single-child Effect container), return a borrow of the
/// inner SKILL `FightStep`. Otherwise `None`. Hides the legacy
/// `Effect-wrapping-Skill` shape that some emissions still carry.
pub fn wrapped_skill_from_effect(effect: &ActEffect) -> Option<&FightStep> {
    if effect.effect_type != Some(162) {
        return None;
    }

    let wrapped = effect.fight_step.as_ref()?;
    if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
        return Some(wrapped);
    }

    if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
        || wrapped.act_effect.len() != 1
    {
        return None;
    }

    let nested = wrapped.act_effect.first()?;
    if nested.effect_type != Some(162) {
        return None;
    }

    let skill = nested.fight_step.as_ref()?;
    (skill.act_type == Some(fight_step::ActType::Skill as i32)).then_some(skill)
}

/// Mutable variant of [`wrapped_skill_from_effect`].
pub fn wrapped_skill_from_effect_mut(effect: &mut ActEffect) -> Option<&mut FightStep> {
    if effect.effect_type != Some(162) {
        return None;
    }

    let wrapped = effect.fight_step.as_mut()?;
    if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
        return Some(wrapped);
    }

    if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
        || wrapped.act_effect.len() != 1
    {
        return None;
    }

    let nested = wrapped.act_effect.first_mut()?;
    if nested.effect_type != Some(162) {
        return None;
    }

    let skill = nested.fight_step.as_mut()?;
    (skill.act_type == Some(fight_step::ActType::Skill as i32)).then_some(skill)
}

/// Strip the legacy `Effect-wrapping-Skill` outer container, leaving
/// a flat 162-wrapped Skill `ActEffect`. Returns `None` when `effect`
/// isn't a wrapped skill in either shape. Used by callers that want
/// to relocate a wrapped emission into a different parent without
/// dragging the legacy wrapper along.
pub fn normalize_wrapped_skill_effect(effect: &ActEffect) -> Option<ActEffect> {
    if effect.effect_type != Some(162) {
        return None;
    }

    let wrapped = effect.fight_step.as_ref()?;
    if wrapped.act_type == Some(fight_step::ActType::Skill as i32) {
        return Some(effect.clone());
    }

    if wrapped.act_type != Some(fight_step::ActType::Effect as i32)
        || wrapped.act_effect.len() != 1
    {
        return None;
    }

    let nested = wrapped.act_effect.first()?;
    let skill = nested.fight_step.as_ref()?;
    (nested.effect_type == Some(162) && skill.act_type == Some(fight_step::ActType::Skill as i32))
        .then_some(nested.clone())
}

/// Locate the defender-bootstrap step's nested effect Vec where
/// boss-side passive emissions get appended. Prefers the rightmost
/// step whose subtree contains the boss state-cycle act_id
/// `530000151`; falls back to the rightmost flat Effect-typed step
/// with all-zero ids. Returns the nested effect Vec inside that
/// step's first 162-wrapped Effect-typed child, or `None` when no
/// suitable bootstrap step is present.
pub fn find_bootstrap_nested_effects_mut(
    steps: &mut [FightStep],
) -> Option<&mut Vec<ActEffect>> {
    let preferred_step_idx = steps.iter().rposition(|step| {
        step.act_effect.iter().any(|effect| {
            wrapped_skill_from_effect(effect).map(|s| s.act_id) == Some(Some(530000151))
        })
    });
    let fallback_step_idx = steps.iter().rposition(|step| {
        step.act_type == Some(fight_step::ActType::Effect as i32)
            && step.act_id.unwrap_or(0) == 0
            && step.from_id.unwrap_or(0) == 0
            && step.to_id.unwrap_or(0) == 0
    });
    let step = steps.get_mut(preferred_step_idx.or(fallback_step_idx)?)?;
    let nested = step.act_effect.iter_mut().find(|effect| {
        effect.effect_type == Some(162)
            && effect
                .fight_step
                .as_ref()
                .map(|inner| {
                    inner.act_type == Some(fight_step::ActType::Effect as i32)
                        && inner.act_id.unwrap_or(0) == 0
                })
                .unwrap_or(false)
    })?;
    Some(&mut nested.fight_step.as_mut()?.act_effect)
}
