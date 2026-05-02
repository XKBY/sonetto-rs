//! Sotheby — Triple the Dose / Concentrated Essence kit, Tier IV
//! Duality Potion (round-start grant + consume-on-attack pattern).
//!
//! Currently exposes only the destiny passive linkage. The Tier IV
//! consume-on-attack semantics live in shared mechanics today (battle3
//! has a known parity gap on this) and will move here as separate
//! work lands.

/// Destiny stone id that activates Sotheby's Tier IV passive bundle.
pub const DESTINY_STONE: i32 = 300901;

/// `(skill_id, tier_required)` pairs the engine injects when a Sotheby
/// entity has at least the matching destiny rank. All three of
/// Sotheby's orphan passives unlock together at Tier IV
/// (Duality Potion mechanic).
const DESTINY_PASSIVES: &[(i32, u8)] = &[
    (30090144, 4),
    (30090145, 4),
    (30090146, 4),
];

/// Returns the orphan destiny passives the engine should inject for
/// a Sotheby entity at `destiny_rank`. Filters by tier so a rank-1
/// Sotheby doesn't get Tier IV passives. Returns an empty slice if
/// the stone isn't Sotheby's.
pub fn destiny_passive_skills(destiny_stone: i32, destiny_rank: i32) -> Vec<i32> {
    if destiny_stone != DESTINY_STONE {
        return Vec::new();
    }
    DESTINY_PASSIVES
        .iter()
        .filter(|(_, tier)| destiny_rank >= *tier as i32)
        .map(|(id, _)| *id)
        .collect()
}
