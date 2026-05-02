//! Per-hero kit modules. Each hero with non-default behavior owns
//! one file containing their skills, passives, insights, and
//! euphoria rules. Phase dispatchers route into these modules via
//! `match HeroId { ... }` against the entity's resolved hero id.
//!
//! Shared mechanics (bloodtithe, channel, empathy, dot,
//! magic_circle) and shared execution (emit pipeline, classification,
//! trigger routing) live elsewhere — hero files call into them, never
//! the reverse.
//!
//! Psychube rules live in `psychubes/` keyed by equip_id, not here.
//! A hero file references psychube IDs only when its kit is balanced
//! around a specific psychube (and even then the rules are owned by
//! the psychube file).

// Hero modules will be added one-per-commit as they migrate from the
// scattered locations they currently live in. Until then this module
// is an intentional placeholder — the HeroId enum is the contract;
// files appear as work lands.
pub mod nautika;
pub mod rubuska;
pub mod sentinel;
pub mod sotheby;

use crate::state::battle::hero::HeroId;
use std::collections::HashMap;

/// Hero-specific orphan passives the engine injects at hero load
/// outside the `skill_passive_level` table. The per-hero modules own
/// the rules; this dispatcher routes by `HeroId` so callers don't
/// hardcode model ids.
pub fn passive_injections(hero_id: i32, ex_map: &HashMap<i32, i32>) -> Vec<i32> {
    match HeroId::from_model_id(hero_id) {
        Some(HeroId::Sentinel) => sentinel::passive_injections(ex_map),
        _ => Vec::new(),
    }
}
