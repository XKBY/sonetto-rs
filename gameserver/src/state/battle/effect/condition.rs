mod dead;
mod enter_fight;
mod none;
mod teammate_dead;

use super::condition_eval::ConditionEval;
use super::condition_type::{ConditionType, condition_type, is_none_condition};
use super::target::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hook {
    None,
    Dead,
    EnterFight,
    RoundStart,
    RoundEnd,
    BattleStart,
    UseCard,
    MoveCard,
    ComposeCard,
    BuffAdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOp {
    And,
    Or,
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub hook: Hook,
    pub cond_type: ConditionType,
    pub target: Target,
}

impl Condition {
    pub fn check(&self, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
        eval_condition(self.cond_type, self.target, owner_uid, eval)
    }
}

fn eval_condition(cond_type: ConditionType, target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    if is_none_condition(cond_type) { return none::check(target, owner_uid, eval); }
    match cond_type {
        ConditionType::_8Dead => dead::check(target, owner_uid, eval),
        ConditionType::_17TeammateDead => teammate_dead::check(owner_uid, eval),
        ConditionType::_5EnterFight => enter_fight::check(target, owner_uid, eval),
        other => {
            tracing::warn!("unimplemented condition type: {:?}", other);
            false
        }
    }
}

fn hooks_for_type(cond_type: ConditionType) -> &'static [Hook] {
    if is_none_condition(cond_type) {
        return &[Hook::RoundStart, Hook::BattleStart];
    }
    match cond_type {
        ConditionType::_8Dead => &[dead::HOOK],
        ConditionType::_17TeammateDead => &[teammate_dead::HOOK],
        ConditionType::_5EnterFight => &[enter_fight::HOOK],
        _ => &[],
    }
}

/// Parses a condition raw string into individual `Condition`s and the combining op.
/// Returns `None` if no known condition types are found.
pub fn parse(raw: &str, cond_target: i32, _owner_uid: i64) -> Option<(Vec<Condition>, ConditionOp)> {
    if raw.is_empty() { return None; }
    let op = if raw.contains('&') { ConditionOp::And } else { ConditionOp::Or };
    let sep = if op == ConditionOp::And { '&' } else { '|' };
    let target = Target::from_id(cond_target);
    let conditions: Vec<Condition> = raw.split(sep)
        .filter_map(|seg| {
            let id: i32 = seg.split('#').next()?.parse().ok()?;
            let cond_type = condition_type(id)?;
            Some((cond_type, target))
        })
        .flat_map(|(cond_type, target)| {
            hooks_for_type(cond_type).iter().map(move |&hook| Condition { hook, cond_type, target })
        })
        .collect();
    if conditions.is_empty() { return None; }
    Some((conditions, op))
}
