use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};

pub const HOOK: Hook = Hook::None;

pub fn check(_target: Target, _owner_uid: i64, _eval: ConditionEval<'_>) -> bool {
    true
}
