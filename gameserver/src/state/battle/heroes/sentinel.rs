//! Sentinel — Hour of Repentance / Dread Bullet kit. Engine wires
//! her core passive (31260191) outside the `skill_passive_level`
//! table; the helper here mirrors that wiring with ex-level
//! resolution. The richer Dread Bullet / Insight III moxie-gain
//! mechanics still live in shared modules and will move here as
//! they migrate one piece at a time.

use std::collections::HashMap;

/// Sentinel's core passive. Not present in `skill_passive_level` for
/// hero 3126 — the engine injects it at hero load. The `ex_map` is
/// the cumulative ex-skill upgrade map the entity loader builds; if
/// the hero has reached a level that upgrades 31260191, the upgraded
/// variant is returned instead.
pub const ORPHAN_PASSIVE: i32 = 31260191;

pub fn passive_injections(ex_map: &HashMap<i32, i32>) -> Vec<i32> {
    vec![*ex_map.get(&ORPHAN_PASSIVE).unwrap_or(&ORPHAN_PASSIVE)]
}
