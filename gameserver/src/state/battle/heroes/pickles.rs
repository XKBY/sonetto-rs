//! Pickles — `The Dog Thinks` Insight passive plus the destiny
//! orphan passives (`30630151` / `30630161` / `30630171`) the data
//! tables don't list directly. The orphan ladder threads through
//! `crate::state::battle::destiny`; the inner-wrapper coalescer for
//! her Hedonism Implement (`30630122`) lives in `skill::executor`
//! today and will move here when the executor coalescer migrates.
//!
//! Currently a thin marker — `is_pickles` lets callers stop spelling
//! out model id `3063`.

use crate::state::battle::hero::HeroId;

#[allow(dead_code)]
pub fn is_pickles(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Pickles.model_id())
}
