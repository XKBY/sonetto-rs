//! Willow — `In the Fire, In the Shadow` (EX) plus `Hag's Advice`
//! and `Hag's Gratitude` basics. Her mixed-mode passive `31040141`
//! is the canonical example for the trigger filter that gates
//! event-driven vs sweep-driven re-fires; that filter lives in
//! `trigger::combat`.
//!
//! Currently a thin marker — `is_willow` lets callers stop spelling
//! out model id `3104`.

use crate::state::battle::hero::HeroId;

#[allow(dead_code)]
pub fn is_willow(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Willow.model_id())
}
