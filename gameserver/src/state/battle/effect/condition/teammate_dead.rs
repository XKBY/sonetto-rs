use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};
use super::super::condition::Hook;
use super::dead::Checker;

pub fn resolve(owner_uid: i64) -> (Hook, Checker) {
    (Hook::Dead, Box::new(move |eval| {
        let uids = Target::AllAllyNoSelf.entities(eval.fight, owner_uid);
        let result = uids.contains(&eval.target_uid);
        tracing::info!(owner_uid, target_uid = eval.target_uid, ?uids, result, "TeammateDead condition check");
        result
    }))
}
