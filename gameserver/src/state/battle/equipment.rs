//! Equipment (psychube) identification helpers.
//!
//! Psychubes are equipment-sourced skill emissions. The mapping
//! lives in `equip_skill.json`: each row's `id` is an equip id
//! (cross-referenced with `equip.json`), `skillLv` is the
//! enhancement level, and `skill` is the actual emitted skill id
//! at that level. For example, equip 1544 ("The Final Roll") has
//! five rows mapping levels 1..5 to skills 434411..434415.
//!
//! At runtime, a hero's `passive_skill` list contains the level-
//! appropriate `skill` value; the engine emits that skill_id as a
//! depth-1 inline child of host card casts (LIVE Pattern A from
//! `_live_emission_contract.md`), with the carrier hero as `from`.
//!
//! This module is for **diagnostic classification** (used by
//! `EmissionTimeline` to tag psychube emissions as a separate
//! phase from generic passive sweeps) and as the foundation for
//! drain-time shape rules when EventQueue Phase 4 adds
//! `SkillEmitKind::EquipmentEmbedded`.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Set of skill ids that are emitted directly by an equipment
/// row in `equip_skill`. Computed once from config; stable.
fn psychube_skill_set() -> &'static HashSet<i32> {
    static CACHE: OnceLock<HashSet<i32>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let cfg = config::configs::get();
        cfg.equip_skill
            .iter()
            .filter(|row| row.skill > 0)
            .map(|row| row.skill)
            .collect()
    })
}

/// Whether `skill_id` is the `skill` field of any `equip_skill`
/// row — i.e. it is the carrier passive emitted directly by a
/// psychube at some level.
///
/// Note: this is the strict reading. Skills that the psychube's
/// carrier passive *applies* (e.g. stat-buff downstream emissions
/// like `434425` triggered by the level-5 carrier `434415`) are
/// NOT recognized here — those are downstream effects, not
/// equipment skills themselves.
pub fn is_psychube_skill(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    psychube_skill_set().contains(&skill_id)
}

// Tests need full config init (`config::configs::init()` must be called
// before `get()`). Validation happens via fixture replay — running with
// `SONETTO_EMISSION_TIMELINE=1` will show `PsychubeAttached` records for
// emissions that pass `is_psychube_skill`.
