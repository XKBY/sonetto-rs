mod behavior;

pub mod cache;
pub mod classification;
pub mod classification_kind;
pub mod condition;
pub mod euphoria;
pub mod source_kind;

pub mod damage;
mod executor;
mod phase;
pub mod sibling_coalesce;
pub mod targets;

pub(crate) use behavior::buff;
pub(crate) use behavior::precast::{
    collect_precast_skills_for_caster, infer_precast_per_decr_seed_cap,
};
pub use executor::{SkillExecutor, build_skill_act_effect};
pub use phase::{PhaseFilter, TriggerState};
pub use targets::get_entity;
