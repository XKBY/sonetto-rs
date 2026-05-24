mod be_attacked;
mod buff_id_del;
mod dead;
mod enter_fight;
mod has_buff_id;
mod none;
mod teammate_dead;
mod use_ex_skill;

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
    BuffLost,
    EvalActiveSkill,
    EvalBeingAttacked,
    UseExSkill,
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
    pub params: Vec<i32>,
}

impl Condition {
    pub fn check(&self, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
        eval_condition(self.cond_type, self.target, &self.params, owner_uid, eval)
    }
}

fn eval_condition(cond_type: ConditionType, target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    if is_none_condition(cond_type) { return none::check(target, owner_uid, eval); }
    match cond_type {
        ConditionType::_22209BeAttacked => be_attacked::check(target, owner_uid, eval),
        ConditionType::_8Dead => dead::check(target, owner_uid, eval),
        ConditionType::_812Dead => dead::check(target, owner_uid, eval),
        ConditionType::_17TeammateDead => teammate_dead::check(owner_uid, eval),
        ConditionType::_5EnterFight => enter_fight::check(target, owner_uid, eval),
        ConditionType::_25210UseExSkill => use_ex_skill::check(target, owner_uid, eval),
        ConditionType::_49BuffIdDel => buff_id_del::check(target, params, owner_uid, eval),
        | ConditionType::_19201HasBuffId
        | ConditionType::_19208HasBuffId
        | ConditionType::_19202HasBuffId
        | ConditionType::_19209HasBuffId
        | ConditionType::_19203HasBuffId => has_buff_id::check(target, params, owner_uid, eval),
        other => {
            tracing::warn!("unimplemented condition type: {:?}", other);
            false
        }
    }
}

fn hooks_for_type(cond_type: ConditionType, cond_target: i32) -> &'static [Hook] {
    if is_none_condition(cond_type) {
        return &[Hook::RoundStart, Hook::BattleStart];
    }
    match cond_type {
        ConditionType::_22209BeAttacked => &[be_attacked::HOOK],
        ConditionType::_8Dead => &[dead::HOOK],
        ConditionType::_812Dead => &[dead::HOOK],
        ConditionType::_17TeammateDead => &[teammate_dead::HOOK],
        ConditionType::_5EnterFight => &[enter_fight::HOOK],
        ConditionType::_19201HasBuffId | ConditionType::_19208HasBuffId | ConditionType::_19203HasBuffId => &[Hook::EvalActiveSkill],
        ConditionType::_25210UseExSkill => &[Hook::UseExSkill],
        ConditionType::_49BuffIdDel => &[buff_id_del::HOOK],
        ConditionType::_19202HasBuffId | ConditionType::_19209HasBuffId => &[Hook::EvalBeingAttacked],
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
            let mut parts = seg.split('#');
            let id: i32 = parts.next()?.parse().ok()?;
            let params: Vec<i32> = parts
                .filter_map(|p| p.trim_end_matches('!').parse().ok())
                .collect();
            let cond_type = condition_type(id)?;
            Some((cond_type, target, params))
        })
        .flat_map(|(cond_type, target, params)| {
            hooks_for_type(cond_type, cond_target).iter().map(move |&hook| Condition {
                hook,
                cond_type,
                target,
                params: params.clone(),
            }).collect::<Vec<_>>()
        })
        .collect();
    if conditions.is_empty() { return None; }
    Some((conditions, op))
}
