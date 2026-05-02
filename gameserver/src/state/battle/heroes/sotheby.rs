//! Sotheby — Triple the Dose / Concentrated Essence kit, Tier IV
//! Duality Potion (round-start grant + consume-on-attack pattern).
//!
//! Currently exposes only the destiny passive linkage. The Tier IV
//! consume-on-attack semantics live in shared mechanics today (battle3
//! has a known parity gap on this) and will move here as separate
//! work lands.

/// Destiny stone id that activates Sotheby's Tier IV passive bundle.
pub const DESTINY_STONE: i32 = 300901;

/// Skills the Tier IV destiny passive bundle adds to her passive list.
/// Not present in `skill_passive_level` or destiny `exchangeSkills`,
/// so the engine has to inject them explicitly at fresh-battle build.
/// 30090144/45 grant `820#…` HealingBoost-family buffs at round start;
/// 30090146 grants `30091120` (Duality Potion) which cascades the
/// `850 AddBuffBoth` chain (Poison enhanced + Cure to allies).
pub const DESTINY_PASSIVE_SKILLS: &[i32] = &[30090144, 30090145, 30090146];
