//! Nautika psychube-bundle post-turn cleanup.
//!
//! Nautika's psychube carrier (host act_id `31200193`) collects the
//! ally-side battle-rule cycle broadcasts and the enemy-side cycle
//! deletions emitted at round end, folds the Semmelweis broadcast
//! into the bundle, and strips the orphan top-level rebroadcasts plus
//! a small family of post-turn attribute-noise emissions so the
//! outgoing FightStep stream matches the official client shape.
//!
//! Two entry points:
//! - `consolidate_into_bundle`: walks post-round-end FightSteps, finds
//!   the Nautika bundle host, and migrates Semmelweis's `530000151`
//!   wrapper into it. Strips the orphan ally broadcasts and the
//!   enemy-side `530000412` deletions whose data is now redundant.
//! - `strip_post_turn_noise`: removes top-level enemy boss-cycle
//!   rebroadcasts and the flat post-round attr-noise wrappers. Runs
//!   after the consolidation step.
//!
//! In-game text frames Nautika's psychube as keeping the cycle running
//! across the round transition; the engine implements that as a
//! single Nautika-hosted bundle plus removal of the now-redundant
//! standalone wrappers.

use std::collections::HashMap;

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    fight_step::ActEffectBuilder, step_walker, types::effects::EffectType,
    utils::find_uid_by_hero_id,
};

/// Nautika carrier-host wrapper act_id. The post-turn bundle is
/// rooted under an effect_container with this act_id.
pub const CARRIER_HOST_ACT_ID: i32 = 31200193;

/// Battle-rule-derived ally-side state-cycle broadcast skill
/// (Semmelweis's owner). Sourced from `rule.json::effect`.
pub const BOSS_CYCLE_ACT_ID: i32 = 530000151;

/// Enemy-side companion broadcast that the bundle absorbs and the
/// orphan stripper removes from the top level.
pub const ENEMY_CYCLE_DEL_ACT_ID: i32 = 530000412;

/// Post-turn attribute-noise effect types that the stripper removes
/// from flat top-level wrappers after the bundle is consolidated.
pub const POST_ROUND_ATTR_NOISE_TYPES: [i32; 4] = [60, 61, 96, 211];

/// Tail-marker effect appended to the consolidated bundle wrapper to
/// signal cycle completion.
pub const TAIL_MARKER_EFFECT_TYPE: i32 = 26;

/// Hero ids (model_id) used to locate runtime uids of the broadcast
/// owners. The literals here are stable engine identifiers; see
/// memory note `project_battle1_gaps.md` re: `find_uid_by_hero_id`.
pub const SEMMELWEIS_HERO_ID: i32 = 3088;
pub const NAUTIKA_HERO_ID: i32 = 3120;

/// effect_type emitted by the round-transition synchronization step
/// at the head of a round. The post-Nautika cleanup strips standalone
/// duplicates of this marker (only LIVE keeps the leading one).
const CHANGE_ROUND_SYNC_EFFECT_TYPE: i32 = 310;

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
        .any(|step| step_walker::step_contains_act_id(step, CARRIER_HOST_ACT_ID))
    {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(276)
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

/// Remove duplicate `effect_type=310` (ChangeRound) sync markers
/// that sometimes appear standalone after the round-leading marker
/// when the Nautika carrier-host is present. The first ChangeRound
/// marker is kept as the round opener; later standalone duplicates
/// are stripped. Returns silently when the carrier-host isn't on
/// this round (Nautika-only effect).
pub fn strip_duplicate_change_round_markers(steps: &mut Vec<FightStep>) {
    let Some(first_step) = steps.first() else {
        return;
    };
    if !step_walker::step_has_effect_type(first_step, CHANGE_ROUND_SYNC_EFFECT_TYPE) {
        return;
    }
    if !steps
        .iter()
        .any(|step| step_walker::step_contains_act_id(step, CARRIER_HOST_ACT_ID))
    {
        return;
    }

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(1) {
        if step_walker::is_standalone_effect_marker(step, CHANGE_ROUND_SYNC_EFFECT_TYPE) {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// Remove top-level enemy boss-cycle rebroadcasts and flat post-round
/// attr-noise wrappers from the FightStep stream. Runs after
/// `consolidate_into_bundle` has folded the canonical broadcast into
/// the Nautika bundle.
pub fn strip_post_turn_noise(steps: &mut Vec<FightStep>) {
    if !steps
        .iter()
        .any(|step| step_walker::step_contains_act_id(step, CARRIER_HOST_ACT_ID))
    {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(276)
    }) else {
        return;
    };

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if is_top_level_enemy_cycle_noise_step(step) || is_flat_post_round_attr_noise_step(step) {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

fn is_bundle_step(step: &FightStep, host_uid: i64) -> bool {
    if step.act_type != Some(fight_step::ActType::Effect as i32) {
        return false;
    }

    let Some(first) = step.act_effect.first() else {
        return false;
    };
    if first.effect_type != Some(162) {
        return false;
    }

    first
        .fight_step
        .as_ref()
        .map(|wrapped| {
            wrapped.act_type == Some(fight_step::ActType::Effect as i32)
                && wrapped.act_id == Some(CARRIER_HOST_ACT_ID)
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
    if skill
        .act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(TAIL_MARKER_EFFECT_TYPE))
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
        ActEffectBuilder::new(TAIL_MARKER_EFFECT_TYPE, semmelweis_uid)
            .effect_num(0)
            .build(),
    );
}

fn is_top_level_enemy_cycle_noise_step(step: &FightStep) -> bool {
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

fn is_flat_post_round_attr_noise_step(step: &FightStep) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id.unwrap_or(0) == 0
        && step.from_id.unwrap_or(0) == 0
        && step.to_id.unwrap_or(0) == 0
        && !step.act_effect.is_empty()
        && step.act_effect.len() <= 3
        && step.act_effect.iter().all(|effect| {
            effect.fight_step.is_none()
                && POST_ROUND_ATTR_NOISE_TYPES.contains(&effect.effect_type.unwrap_or(0))
                && effect.target_id.unwrap_or(0) == 0
                && matches!(effect.effect_num.unwrap_or(0), 0 | 1)
        })
}
