mod be_attacked;
mod buff_id_del;
mod dead;
mod enter_fight;
mod has_buff_id;
mod none;
mod per_buff_id_count;
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
    AfterAction,
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
    pub fn check(&self, owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
        eval_condition(self.cond_type, self.target, &self.params, owner_uid, eval)
    }
}

fn eval_condition(cond_type: ConditionType, target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
    if is_none_condition(cond_type) { return if none::check(target, owner_uid, eval) { Some(1) } else { None }; }
    match cond_type {
        ConditionType::_22209BeAttacked => if be_attacked::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_8Dead => if dead::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_812Dead => if dead::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_17TeammateDead => if teammate_dead::check(owner_uid, eval) { Some(1) } else { None },
        ConditionType::_5EnterFight | ConditionType::_5021EnterFight => if enter_fight::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_25210UseExSkill => if use_ex_skill::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_49BuffIdDel => if buff_id_del::check(target, params, owner_uid, eval) { Some(1) } else { None },
        | ConditionType::_19201HasBuffId
        | ConditionType::_19208HasBuffId
        | ConditionType::_19202HasBuffId
        | ConditionType::_19209HasBuffId
        | ConditionType::_19210HasBuffId
        | ConditionType::_19203HasBuffId => if has_buff_id::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_61003PerBuffIdCount | ConditionType::_61004PerBuffIdCount |
        ConditionType::_61010PerBuffIdCount | ConditionType::_61012PerBuffIdCount |
        ConditionType::_61100PerBuffIdCount | ConditionType::_61102PerBuffIdCount |
        ConditionType::_61103PerBuffIdCount | ConditionType::_61104PerBuffIdCount |
        ConditionType::_61106PerBuffIdCount | ConditionType::_61201PerBuffIdCount |
        ConditionType::_61202PerBuffIdCount | ConditionType::_61203PerBuffIdCount |
        ConditionType::_61204PerBuffIdCount | ConditionType::_61208PerBuffIdCount |
        ConditionType::_61209PerBuffIdCount | ConditionType::_61210PerBuffIdCount |
        ConditionType::_61212PerBuffIdCount | ConditionType::_61213PerBuffIdCount |
        ConditionType::_61214PerBuffIdCount | ConditionType::_61215PerBuffIdCount |
        ConditionType::_61301PerBuffIdCount | ConditionType::_61302PerBuffIdCount |
        ConditionType::_61303PerBuffIdCount | ConditionType::_61304PerBuffIdCount |
        ConditionType::_61307PerBuffIdCount | ConditionType::_61401PerBuffIdCount |
        ConditionType::_612081PerBuffIdCount => per_buff_id_count::check(target, params, owner_uid, eval),
        other => {
            tracing::warn!("unimplemented condition type: {:?}", other);
            None
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
        ConditionType::_5EnterFight | ConditionType::_5021EnterFight => &[enter_fight::HOOK],
        ConditionType::_19201HasBuffId | ConditionType::_19208HasBuffId | ConditionType::_19203HasBuffId => &[Hook::EvalActiveSkill],
        ConditionType::_25210UseExSkill => &[Hook::UseExSkill],
        ConditionType::_49BuffIdDel => &[buff_id_del::HOOK],
        ConditionType::_19202HasBuffId | ConditionType::_19209HasBuffId => &[Hook::EvalBeingAttacked],
        ConditionType::_19210HasBuffId => &[Hook::AfterAction],

        // Round start triggers:
        ConditionType::_61100PerBuffIdCount | ConditionType::_61102PerBuffIdCount |
        ConditionType::_61103PerBuffIdCount | ConditionType::_61104PerBuffIdCount |
        ConditionType::_61106PerBuffIdCount => &[Hook::RoundStart],

        // Being attacked triggers:
        ConditionType::_61204PerBuffIdCount => &[Hook::EvalBeingAttacked],

        // Skill evaluation triggers:
        ConditionType::_61201PerBuffIdCount | ConditionType::_61202PerBuffIdCount |
        ConditionType::_61203PerBuffIdCount | ConditionType::_61208PerBuffIdCount |
        ConditionType::_61209PerBuffIdCount | ConditionType::_61210PerBuffIdCount |
        ConditionType::_61212PerBuffIdCount | ConditionType::_61213PerBuffIdCount |
        ConditionType::_61214PerBuffIdCount | ConditionType::_61215PerBuffIdCount |
        ConditionType::_612081PerBuffIdCount => &[Hook::EvalActiveSkill],

        // Round end triggers:
        ConditionType::_61301PerBuffIdCount | ConditionType::_61302PerBuffIdCount |
        ConditionType::_61303PerBuffIdCount | ConditionType::_61304PerBuffIdCount |
        ConditionType::_61307PerBuffIdCount | ConditionType::_61401PerBuffIdCount => &[Hook::RoundEnd],

        // Battle start & round end triggers:
        ConditionType::_61003PerBuffIdCount | ConditionType::_61004PerBuffIdCount |
        ConditionType::_61010PerBuffIdCount | ConditionType::_61012PerBuffIdCount => &[Hook::RoundEnd, Hook::BattleStart, Hook::EnterFight],

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
