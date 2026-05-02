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
pub mod rubuska;
pub mod sotheby;
