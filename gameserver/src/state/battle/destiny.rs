//! Destiny linkage the game's data tables don't expose. Reverse-
//! engineered and verified against LIVE replay; both the SkillSource
//! classifier and the fresh-battle passive injection read from here.
//!
//! These passives exist in `skill_effect.json` but are absent from
//! `skill_passive_level` AND from `character_destiny_facets.exchangeSkills`.
//! The mapping `(destiny_stone, [(skill_id, tier_required)])` is engine
//! knowledge — when a new hero's Tier IV introduces orphan passives,
//! add a match arm here.

#[derive(Debug, Clone, Copy)]
pub struct OrphanPassive {
    pub skill_id: i32,
    pub tier: u8,
}

/// `(skill_id, tier_required)` orphan passives unlocked by the given
/// destiny stone. Empty if `destiny_stone` isn't a known stone.
pub fn passives_for(destiny_stone: i32) -> &'static [OrphanPassive] {
    match destiny_stone {
        300901 => &[
            // Sotheby Tier IV — Duality Potion mechanic
            OrphanPassive {
                skill_id: 30090144,
                tier: 4,
            },
            OrphanPassive {
                skill_id: 30090145,
                tier: 4,
            },
            OrphanPassive {
                skill_id: 30090146,
                tier: 4,
            },
        ],
        306201 => &[
            // Melania
            OrphanPassive {
                skill_id: 30620144,
                tier: 1,
            },
            OrphanPassive {
                skill_id: 30620147,
                tier: 1,
            },
        ],
        306301 => &[
            // Pickles
            OrphanPassive {
                skill_id: 30630151,
                tier: 1,
            },
            OrphanPassive {
                skill_id: 30630161,
                tier: 2,
            },
            OrphanPassive {
                skill_id: 30630171,
                tier: 3,
            },
        ],
        308801 => &[
            // Semmelweis
            OrphanPassive {
                skill_id: 308801911,
                tier: 1,
            },
            OrphanPassive {
                skill_id: 308801921,
                tier: 2,
            },
            OrphanPassive {
                skill_id: 308802111,
                tier: 4,
            },
        ],
        _ => &[],
    }
}

/// Returns the orphan passive skill ids the engine should inject for
/// an entity with the given `destiny_stone` and `destiny_rank`.
/// Filters by tier so a low-rank hero doesn't get higher-tier passives.
pub fn passives_to_inject(destiny_stone: i32, destiny_rank: i32) -> Vec<i32> {
    passives_for(destiny_stone)
        .iter()
        .filter(|p| destiny_rank >= p.tier as i32)
        .map(|p| p.skill_id)
        .collect()
}

/// `destiny_stone` follows `<hero_id><facet_index>` packed as a single
/// int (e.g. `300901` = hero 3009 facet 01). Strips the facet to recover
/// the hero id.
pub fn hero_id_for_stone(destiny_stone: i32) -> i32 {
    destiny_stone / 100
}

/// Stones the engine has reverse-engineered orphan passives for.
/// Used by the SkillSource classifier so each consumer iterates one
/// list instead of duplicating stone enumeration.
pub const KNOWN_STONES: &[i32] = &[300901, 306201, 306301, 308801];
