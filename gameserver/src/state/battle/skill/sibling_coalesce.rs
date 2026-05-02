//! Sibling-skill-wrapper coalescer. When a parent skill structurally
//! emits the same per-target sub-skill twice (or more) under different
//! conditions — e.g. an `AddBuffRanId` chain that runs once per
//! `CareerCheck#0` ally and once per `CareerCheck#1` ally with the
//! same target buff id — the official client folds the duplicate
//! wrappers into one. The two helpers here detect that pattern from
//! the parent skill's `behavior1..5` shape and merge the duplicates
//! after execution.
//!
//! Currently three skills in the data tables match the pattern
//! (Pickles' `30630151`, plus `8290303` and `110320177`). The
//! detection is data-driven via `skill_effect.behaviorN` so a future
//! parent that ships with the same `AddBuffRanId#<buff>` duplication
//! drops in without code changes.
//!
//! TODO(event-queue): once the executor's behavior pipeline emits
//! per-target ticks INSIDE one wrapper rather than producing
//! per-target wrappers that we then merge, this post-execution pass
//! becomes obsolete. EventQueue Phase 4+5 reach that shape during
//! drain; until then this is the cleanest non-hardcoded approximation.

use std::collections::HashMap;

use sonettobuf::{ActEffect, fight_step};

/// Returns true when `parent_skill_id` has 2+ behaviors of type
/// `AddBuffRanId` (skill_behavior id `20021`) that share a target
/// buff id. The shape encodes "this parent runs the same per-target
/// fanout twice (or more) under different conditions, and the LIVE
/// client merges the outputs into one sub-skill wrapper carrying
/// every tick".
pub fn parent_skill_fans_out_into_mergeable_siblings(parent_skill_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == parent_skill_id) else {
        return false;
    };

    let mut add_buff_ran_targets: Vec<i32> = Vec::new();
    for behavior in [
        skill.behavior1.as_str(),
        skill.behavior2.as_str(),
        skill.behavior3.as_str(),
        skill.behavior4.as_str(),
        skill.behavior5.as_str(),
    ] {
        if let Some(rest) = behavior.strip_prefix("20021#") {
            let head = rest.split('#').next().unwrap_or("");
            if let Ok(target_buff_id) = head.parse::<i32>() {
                if target_buff_id > 0 {
                    add_buff_ran_targets.push(target_buff_id);
                }
            }
        }
    }

    if add_buff_ran_targets.len() < 2 {
        return false;
    }
    let mut sorted = add_buff_ran_targets.clone();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[0] == pair[1])
}

/// Merge sibling SKILL-wrapped emissions with matching `act_id` under
/// `parent_skill_id`'s `effect_steps`: the first occurrence keeps its
/// position, every duplicate's nested `act_effect` content is
/// appended to the first, and the duplicate slots are removed. The
/// merge only runs when `parent_skill_fans_out_into_mergeable_siblings`
/// identifies the parent as one whose behaviors structurally produce
/// duplicated wrappers that the official client folds together.
pub fn coalesce_duplicate_sibling_skill_wrappers(
    parent_skill_id: i32,
    effect_steps: &mut Vec<ActEffect>,
) {
    if !parent_skill_fans_out_into_mergeable_siblings(parent_skill_id) {
        return;
    }

    let mut occurrences: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, effect) in effect_steps.iter().enumerate() {
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        if step.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }
        let Some(act_id) = step.act_id else {
            continue;
        };
        if act_id <= 0 {
            continue;
        }
        occurrences.entry(act_id).or_default().push(idx);
    }

    let mut all_duplicate_indices: Vec<usize> = Vec::new();
    for (_act_id, indices) in occurrences.iter() {
        if indices.len() < 2 {
            continue;
        }
        let first_idx = indices[0];
        let mut merged_nested: Vec<ActEffect> = Vec::new();
        for &dup_idx in &indices[1..] {
            if let Some(step) = effect_steps[dup_idx].fight_step.as_ref() {
                merged_nested.extend(step.act_effect.clone());
            }
            all_duplicate_indices.push(dup_idx);
        }
        if let Some(step) = effect_steps[first_idx].fight_step.as_mut() {
            step.act_effect.extend(merged_nested);
        }
    }

    all_duplicate_indices.sort_unstable_by(|a, b| b.cmp(a));
    for idx in all_duplicate_indices {
        effect_steps.remove(idx);
    }
}
