//! Tuesday — `Please, Come On In` (EX) plus the
//! `In Mother's Arms` / `The Horror's Delight` basics. Her kit
//! drives Poison / DeadlyPoison DOT ticks through the generic
//! `mechanics::dot` machinery (buff_act 803 / 844) and gates several
//! conditions on `HasBuffGroup` / `NoBuffGroup` (77208 / 78208) per
//! the Tuesday-class debuffs.
//!
//! Currently a thin marker — `is_tuesday` lets callers stop spelling
//! out model id `3098`. Magic-circle wave-respawn re-application
//! (`30980131` LockPoison) and the array `endSkills` marker are
//! tracked in `mechanics::magic_circle` for now and will move here
//! when follow-up extractions pull her circle hooks together.

use crate::state::battle::hero::HeroId;

#[allow(dead_code)]
pub fn is_tuesday(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Tuesday.model_id())
}
