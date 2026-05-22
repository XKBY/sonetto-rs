use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};

pub fn make_checker(
    target: Target,
    owner_uid: i64,
) -> Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync> {
    Box::new(move |eval| {
        let uids = target.entities(eval.fight, owner_uid);
        uids.contains(&eval.target_uid)
    })
}
