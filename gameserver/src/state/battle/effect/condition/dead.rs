use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};
use super::super::{condition::Hook};

pub type Checker = Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync>;

pub fn resolve(target: Target, owner_uid: i64) -> (Hook, Checker) {
    (Hook::Dead, Box::new(move |eval| {
        let uids = target.entities(eval.fight, owner_uid);
        let result = uids.contains(&eval.target_uid);
        tracing::info!(owner_uid, target_uid = eval.target_uid, ?uids, result, "Dead condition check");
        result
    }))
}
