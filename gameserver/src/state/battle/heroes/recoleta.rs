//! Recoleta — `Maze of Visceral Realism` (EX) plus the
//! `Metaphysical Conceit` / `Literary Imagery` basics. Her psychube
//! `The Final Roll` (`equip_id=1544`, skill `434415`) is the
//! standing example of an idle-sweep over-fire — the latent gap
//! is parked behind the EventQueue / psychube-rider migration and
//! tracked in `memory/project_battle1_gaps.md`.
//!
//! Currently a thin marker — `is_recoleta` lets callers stop
//! spelling out model id `3114`.

use crate::state::battle::hero::HeroId;

#[allow(dead_code)]
pub fn is_recoleta(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Recoleta.model_id())
}
