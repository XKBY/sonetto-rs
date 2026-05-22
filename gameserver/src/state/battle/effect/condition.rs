use super::condition_eval::ConditionEval;
use super::condition_type::ConditionType;
use super::target::Target;

mod dead;
mod enter_fight;
mod teammate_dead;

pub enum Hook {
    Dead,
    EnterFight,
}

pub struct Condition {
    pub hook: Hook,
    pub ids: Vec<Vec<i32>>,
    check: Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync>,
}

impl Condition {
    pub fn check(&self, eval: ConditionEval<'_>) -> bool {
        (self.check)(eval)
    }
}

// Assumption: No mixed & and | in any condition string
fn parse_ids(raw: &str) -> (Vec<Vec<i32>>, bool) {
    let is_and = raw.contains('&');
    let sep = if is_and { '&' } else { '|' };
    let ids = raw.split(sep)
        .map(|seg| seg.split('#').filter_map(|v| v.parse().ok()).collect())
        .filter(|v: &Vec<i32>| !v.is_empty())
        .collect();
    (ids, is_and)
}

fn make_checker(
    cond_type: ConditionType,
    params: Vec<i32>,
    target: Target,
    owner_uid: i64,
) -> Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync> {
    match cond_type {
        ConditionType::Dead => dead::make_checker(target, owner_uid),
        ConditionType::TeammateDead => teammate_dead::make_checker(owner_uid),
        ConditionType::EnterFight => enter_fight::make_checker(target, owner_uid),
        _ => {
            let _ = (params, target, owner_uid);
            Box::new(|_| false)
        }
    }
}

pub fn parse(raw: &str, cond_target: i32, owner_uid: i64) -> Option<Condition> {
    let (ids, is_and) = parse_ids(raw);
    if ids.is_empty() { return None; }

    let target = Target::from_id(cond_target);

    let hook = ids.iter().find_map(|seg| {
        let cond_type = super::condition_type::condition_type(*seg.first()?)?;
        match cond_type {
            ConditionType::Dead | ConditionType::TeammateDead => Some(Hook::Dead),
            ConditionType::EnterFight => Some(Hook::EnterFight),
            _ => None,
        }
    })?;

    let checkers: Vec<Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync>> = ids
        .iter()
        .filter_map(|seg| {
            let cond_type = super::condition_type::condition_type(*seg.first()?)?;
            Some(make_checker(cond_type, seg[1..].to_vec(), target, owner_uid))
        })
        .collect();

    let check: Box<dyn Fn(ConditionEval<'_>) -> bool + Send + Sync> = if is_and {
        Box::new(move |eval| checkers.iter().all(|f| f(eval)))
    } else {
        Box::new(move |eval| checkers.iter().any(|f| f(eval)))
    };

    Some(Condition { hook, ids, check })
}
