use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};
use super::super::condition::Hook;
use super::dead::Checker;

pub fn resolve(target: Target, owner_uid: i64) -> (Hook, Checker) {
    (Hook::EnterFight, Box::new(move |eval| {
        let ents = target.entities(eval.fight, owner_uid);
        tracing::info!("EnterFight owner={} targets={:?} entering={}", owner_uid, ents, eval.target_uid);
        ents.contains(&eval.target_uid)
    }))
}
