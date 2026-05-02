//! Kakania — `Id, Ego and Superego` (EX) and the `Empathy` Mental
//! resource shared with the rest of the team. Insight III routes a
//! bonus emission through the heal-trigger hook (one bounce per
//! `MaxHP × 3%` Empathy crossing); the EX consumes the bank for
//! Genesis bonus damage. Generic Empathy state + the heal hook live
//! in `mechanics/empathy.rs`; this module owns the hero-side
//! handles her shared kit calls back into.
//!
//! Currently a thin marker — the `find_uid` helper routes through
//! `HeroId::Kakania` so callers stop spelling out `3080`. More of
//! her kit (Subconscious bonus damage, Solace bounce, 50% redirect)
//! still lives in `mechanics/empathy.rs` and will move here as
//! follow-up work lands.

use sonettobuf::Fight;

use crate::state::battle::{hero::HeroId, utils::find_uid_by_hero_id};

#[allow(dead_code)]
pub fn is_kakania(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Kakania.model_id())
}

/// Locate Kakania anywhere in the fight (either side). Returns
/// `None` when she isn't on the field.
pub fn find_uid(fight: &Fight) -> Option<i64> {
    find_uid_by_hero_id(fight, HeroId::Kakania.model_id())
}
