//! Sentinel — Hour of Repentance / Dread Bullet kit. Engine wires
//! her core passive (31260191) outside the `skill_passive_level`
//! table; the helper here mirrors that wiring with ex-level
//! resolution. Sentinel does NOT carry a magic-circle ult — but
//! her channel-state buff (`31260201`) carries an
//! `add_passive_skill` feature for `31260181` (Hour of Repentance
//! self-fire), and the generic magic-circle aura-walking code in
//! `mechanics::magic_circle` would otherwise mis-harvest it as a
//! circle aura grant. `is_hour_of_repentance_self_grant` flags the
//! offending pair so the orchestrator can skip it.

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

/// True when `(skill_id, buff_id)` is Sentinel's Hour of Repentance
/// self-grant pair — buff `31260151` or `31260201` carrying the
/// `865#31260181` feature. The magic-circle aura-walking code uses
/// this to avoid re-firing her ult inside another caster's host
/// step.
pub fn is_hour_of_repentance_self_grant(skill_id: i32, buff_id: i32) -> bool {
    skill_id == 31260181 && matches!(buff_id, 31260151 | 31260201)
}
