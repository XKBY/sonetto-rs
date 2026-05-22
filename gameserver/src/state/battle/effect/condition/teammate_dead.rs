use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};

pub fn make_checker(
    owner_uid: i64,
) -> Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync> {
    Box::new(move |eval| {
        Target::AllAllyNoSelf.entities(eval.fight, owner_uid).contains(&eval.target_uid)
    })
}
