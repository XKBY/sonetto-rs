//! Melania — destiny passives (`30620144`, `30620147`) the data
//! tables don't list directly; the orphan linkage flows through
//! `crate::state::battle::destiny` keyed on facets stone `306201`.
//!
//! Currently a thin marker — `is_melania` lets callers stop
//! spelling out model id `3062`.

use crate::state::battle::hero::HeroId;

#[allow(dead_code)]
pub fn is_melania(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Melania.model_id())
}
