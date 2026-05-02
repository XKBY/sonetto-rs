//! Nautika channel-cast post-emission cleanup.
//!
//! When Nautika is channeling, her channel-host buff (a
//! `NuoDiKaCastChannel`-tagged buff such as `31200193`) wraps a
//! round-end bundle that LIVE-side absorbs the ally team's
//! battle-rule state-cycle broadcast plus the matching enemy-side
//! deletion. Our engine emits the rebroadcasts at the top level by
//! default; this module folds them into the channel-host wrapper
//! and strips the now-redundant orphans so the outgoing FightStep
//! stream matches the official client shape.
//!
//! Three entry points called from `manager::round_mgr` in this order:
//! - `strip_duplicate_change_round_markers`: removes standalone
//!   `EffectType::CardDeckNum` duplicates that some round-end
//!   bundles emit alongside the leading marker.
//! - `consolidate_into_bundle`: walks post-round-end FightSteps,
//!   finds the channel-host bundle, migrates Semmelweis's
//!   `530000151` wrapper into it, and strips the orphan ally
//!   broadcasts plus enemy-side `530000412` deletions whose data is
//!   now redundant.
//! - `strip_redundant_post_round_emissions`: removes top-level
//!   enemy boss-cycle rebroadcasts and the flat post-round
//!   state-marker wrappers that LIVE folds into earlier output.
//!
//! The actual Embrace the Past psychube (`equip_id = 1548`) ships
//! different rules — entry-time Max HP, damage-taken Crit Rate
//! stacks, conditional Crit DMG below 80% HP — and is unimplemented
//! today. None of this file relates to those amplification effects.

use std::collections::HashMap;

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    fight_step::ActEffectBuilder, heroes::nautika::CHANNEL_HOST_SKILL_IDS, step_walker,
    types::effects::EffectType, utils::find_uid_by_hero_id,
};

/// Battle-rule-derived ally-side state-cycle broadcast skill
/// (Semmelweis's owner). Sourced from `rule.json::effect`.
const BOSS_CYCLE_ACT_ID: i32 = 530000151;

/// Enemy-side companion broadcast that the bundle absorbs and the
/// orphan stripper removes from the top level.
const ENEMY_CYCLE_DEL_ACT_ID: i32 = 530000412;

/// Round-state marker effect types that LIVE emits alongside the
/// leading `CardDeckNum` opener but doesn't repeat at the top
/// level after the channel-host bundle is consolidated. Our engine
/// emits them naturally from the round-end pipeline; the cleanup
/// strips the redundant copies. The canonical names come from
/// the `EffectType` enum.
const POST_ROUND_STATE_MARKER_TYPES: [EffectType; 4] = [
    EffectType::DealCard2,
    EffectType::RoundEnd,
    EffectType::ClearUniversalCard,
    EffectType::SmallRoundEnd,
];

/// Hero ids (model_id) used to locate runtime uids of the broadcast
/// owners. The literals here are stable engine identifiers; see
/// memory note `project_battle1_gaps.md` re: `find_uid_by_hero_id`.
pub const SEMMELWEIS_HERO_ID: i32 = 3088;
pub const NAUTIKA_HERO_ID: i32 = 3120;

/// Walk post-round-end FightSteps, fold Semmelweis's `530000151`
/// rebroadcast into the Nautika bundle host, and strip the orphan
/// ally rebroadcasts plus enemy-side `530000412` deletions whose
/// content is now redundant.
pub fn consolidate_into_bundle(fight: &Fight, steps: &mut Vec<FightStep>) {
    #[derive(Clone, Copy)]
    struct WrapperLocation {
        step_idx: usize,
        effect_idx: usize,
        from_id: i64,
    }

    let Some(semmelweis_uid) = find_uid_by_hero_id(fight, SEMMELWEIS_HERO_ID) else {
        return;
    };
    let Some(nautika_uid) = find_uid_by_hero_id(fight, NAUTIKA_HERO_ID) else {
        return;
    };

    if !steps
        .iter()
        .any(|step| any_carrier_host_in_step(step))
    {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(EffectType::AllocateCardEnergy as i32)
    }) else {
        return;
    };

    let Some(nautika_bundle_idx) = steps
        .iter()
        .position(|step| is_bundle_step(step, nautika_uid))
    else {
        return;
    };

    let mut ally_wrappers = Vec::new();
    let mut enemy_del_wrappers = Vec::new();

    for (step_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if step_idx == nautika_bundle_idx
            || step.act_type != Some(fight_step::ActType::Effect as i32)
            || step.act_id.unwrap_or(0) != 0
        {
            continue;
        }

        for (effect_idx, effect) in step.act_effect.iter().enumerate() {
            let Some(skill) = step_walker::wrapped_skill_from_effect(effect) else {
                continue;
            };

            let act_id = skill.act_id.unwrap_or(0);
            let from_id = skill.from_id.unwrap_or(0);
            if act_id == BOSS_CYCLE_ACT_ID && from_id > 0 {
                ally_wrappers.push(WrapperLocation {
                    step_idx,
                    effect_idx,
                    from_id,
                });
            } else if act_id == ENEMY_CYCLE_DEL_ACT_ID && from_id < 0 {
                enemy_del_wrappers.push(WrapperLocation {
                    step_idx,
                    effect_idx,
                    from_id,
                });
            }
        }
    }

    let host_already_has_semm_broadcast = steps
        .get(nautika_bundle_idx)
        .map(|step| {
            step.act_effect.iter().any(|effect| {
                step_walker::wrapped_skill_from_effect(effect)
                    .map(|skill| {
                        skill.act_id == Some(BOSS_CYCLE_ACT_ID)
                            && skill.from_id == Some(semmelweis_uid)
                            && skill.to_id == Some(semmelweis_uid)
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let semm_wrapper = ally_wrappers
        .iter()
        .find(|wrapper| wrapper.from_id == semmelweis_uid)
        .copied();

    if !host_already_has_semm_broadcast
        && let Some(wrapper_loc) = semm_wrapper
        && let Some(source_step) = steps.get(wrapper_loc.step_idx)
        && let Some(source_effect) = source_step.act_effect.get(wrapper_loc.effect_idx)
        && let Some(mut normalized) = step_walker::normalize_wrapped_skill_effect(source_effect)
    {
        ensure_tail_marker(&mut normalized, semmelweis_uid);
        if let Some(host_step) = steps.get_mut(nautika_bundle_idx) {
            host_step.act_effect.push(normalized);
        }
    }

    let mut removals_by_step: HashMap<usize, Vec<usize>> = HashMap::new();
    if host_already_has_semm_broadcast || semm_wrapper.is_some() {
        for wrapper in ally_wrappers {
            removals_by_step
                .entry(wrapper.step_idx)
                .or_default()
                .push(wrapper.effect_idx);
        }
    }
    for wrapper in enemy_del_wrappers {
        removals_by_step
            .entry(wrapper.step_idx)
            .or_default()
            .push(wrapper.effect_idx);
    }
    if removals_by_step.is_empty() {
        return;
    }

    let mut emptied_steps = Vec::new();
    for (step_idx, mut effect_indices) in removals_by_step {
        let Some(step) = steps.get_mut(step_idx) else {
            continue;
        };
        effect_indices.sort_unstable();
        effect_indices.dedup();
        for effect_idx in effect_indices.into_iter().rev() {
            if effect_idx < step.act_effect.len() {
                step.act_effect.remove(effect_idx);
            }
        }
        if step.act_effect.is_empty() {
            emptied_steps.push(step_idx);
        }
    }

    emptied_steps.sort_unstable();
    emptied_steps.dedup();
    for step_idx in emptied_steps.into_iter().rev() {
        steps.remove(step_idx);
    }
}

/// Remove duplicate `EffectType::CardDeckNum` (310) sync markers that
/// sometimes appear standalone after the round-leading marker when
/// the Nautika carrier-host is present. The first marker is kept as
/// the round opener; later standalone duplicates are stripped.
/// Returns silently when the carrier-host isn't on this round
/// (Nautika-only effect).
pub fn strip_duplicate_change_round_markers(steps: &mut Vec<FightStep>) {
    let change_round_sync = EffectType::CardDeckNum as i32;
    let Some(first_step) = steps.first() else {
        return;
    };
    if !step_walker::step_has_effect_type(first_step, change_round_sync) {
        return;
    }
    if !steps
        .iter()
        .any(|step| any_carrier_host_in_step(step))
    {
        return;
    }

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(1) {
        if step_walker::is_standalone_effect_marker(step, change_round_sync) {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// Remove top-level emissions LIVE folds into earlier output: the
/// enemy-side mirror of the active battle-rule cycle (rebroadcast
/// at the top level by our engine but absorbed by LIVE), and the
/// flat post-round state-marker wrappers (`DealCard2`, `RoundEnd`,
/// `ClearUniversalCard`, `SmallRoundEnd`) that LIVE pairs with the
/// leading `CardDeckNum` marker only. Runs after
/// `consolidate_into_bundle` has folded the canonical broadcast
/// into the carrier bundle.
pub fn strip_redundant_post_round_emissions(steps: &mut Vec<FightStep>) {
    if !steps
        .iter()
        .any(|step| any_carrier_host_in_step(step))
    {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(EffectType::AllocateCardEnergy as i32)
    }) else {
        return;
    };

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if is_redundant_enemy_cycle_rebroadcast(step)
            || is_redundant_post_round_state_marker(step)
        {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// True when `step` (or any descendant) carries an `act_id` that
/// belongs to Nautika's channel-cast host buff family.
fn any_carrier_host_in_step(step: &FightStep) -> bool {
    CHANNEL_HOST_SKILL_IDS
        .iter()
        .any(|host| step_walker::step_contains_act_id(step, *host))
}

fn is_bundle_step(step: &FightStep, host_uid: i64) -> bool {
    if step.act_type != Some(fight_step::ActType::Effect as i32) {
        return false;
    }

    let Some(first) = step.act_effect.first() else {
        return false;
    };
    if first.effect_type != Some(EffectType::FightStep as i32) {
        return false;
    }

    first
        .fight_step
        .as_ref()
        .map(|wrapped| {
            wrapped.act_type == Some(fight_step::ActType::Effect as i32)
                && wrapped.act_id.is_some_and(|id| CHANNEL_HOST_SKILL_IDS.contains(&id))
                && wrapped.from_id == Some(host_uid)
                && wrapped.to_id == Some(host_uid)
        })
        .unwrap_or(false)
}

fn ensure_tail_marker(wrapper: &mut ActEffect, semmelweis_uid: i64) {
    let Some(skill) = step_walker::wrapped_skill_from_effect_mut(wrapper) else {
        return;
    };
    if skill.act_id != Some(BOSS_CYCLE_ACT_ID)
        || skill.from_id != Some(semmelweis_uid)
        || skill.to_id != Some(semmelweis_uid)
    {
        return;
    }
    let tail_marker = EffectType::Attr as i32;
    if skill
        .act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(tail_marker))
    {
        return;
    }
    if !skill
        .act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
    {
        return;
    }

    let insert_at = skill
        .act_effect
        .iter()
        .rposition(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
        .map(|idx| idx + 1)
        .unwrap_or(skill.act_effect.len());
    skill.act_effect.insert(
        insert_at,
        ActEffectBuilder::new(tail_marker, semmelweis_uid)
            .effect_num(0)
            .build(),
    );
}

fn is_redundant_enemy_cycle_rebroadcast(step: &FightStep) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id.unwrap_or(0) == 0
        && step.from_id.unwrap_or(0) == 0
        && step.to_id.unwrap_or(0) == 0
        && !step.act_effect.is_empty()
        && step.act_effect.iter().all(|effect| {
            step_walker::wrapped_skill_from_effect(effect)
                .map(|skill| {
                    skill.act_id == Some(BOSS_CYCLE_ACT_ID) && skill.from_id.unwrap_or(0) < 0
                })
                .unwrap_or(false)
        })
}

fn is_redundant_post_round_state_marker(step: &FightStep) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id.unwrap_or(0) == 0
        && step.from_id.unwrap_or(0) == 0
        && step.to_id.unwrap_or(0) == 0
        && !step.act_effect.is_empty()
        && step.act_effect.len() <= 3
        && step.act_effect.iter().all(|effect| {
            effect.fight_step.is_none()
                && POST_ROUND_STATE_MARKER_TYPES
                    .iter()
                    .any(|marker| Some(*marker as i32) == effect.effect_type)
                && effect.target_id.unwrap_or(0) == 0
                && matches!(effect.effect_num.unwrap_or(0), 0 | 1)
        })
}
