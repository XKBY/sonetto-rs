use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};

pub fn make_checker(
    target: Target,
    owner_uid: i64,
) -> Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync> {
    Box::new(move |eval| {
        let ents = target.entities(eval.fight, owner_uid);
        tracing::info!("EnterFight owner={} targets={:?} entering={}", owner_uid, ents, eval.target_uid);
        ents.contains(&eval.target_uid)
    })
}
