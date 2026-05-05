//! Sibling-skill-wrapper coalescer parent-eligibility predicate.
//! When a parent skill structurally emits the same per-target sub-skill
//! twice (or more) under different conditions — e.g. an `AddBuffRanId`
//! chain that runs once per `CareerCheck#0` ally and once per
//! `CareerCheck#1` ally with the same target buff id — the official
//! client folds the duplicate wrappers into one. The drain helper
//! `event_queue::coalesce_sibling_skills` performs the merge; this
//! module supplies the data-driven gate predicate that decides when to
//! invoke it.
//!
//! Currently three skills in the data tables match the pattern
//! (Pickles' `30630151`, plus `8290303` and `110320177`). The
//! detection is data-driven via `skill_effect.behaviorN` so a future
//! parent that ships with the same `AddBuffRanId#<buff>` duplication
//! drops in without code changes.

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

