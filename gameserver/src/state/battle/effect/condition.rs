use super::condition_eval::ConditionEval;
use super::condition_type::ConditionType;
use super::target::Target;

mod dead;
mod enter_fight;
mod teammate_dead;

pub use dead::Checker;

#[derive(Debug)]
pub enum Hook {
    Dead,
    EnterFight,
}

pub struct Condition {
    pub hook: Hook,
    pub ids: Vec<Vec<i32>>,
    check: Checker,
}

impl Condition {
    pub fn check(&self, eval: ConditionEval<'_>) -> bool {
        (self.check)(eval)
    }
}

fn parse_ids(raw: &str) -> (Vec<Vec<i32>>, bool) {
    let is_and = raw.contains('&');
    let sep = if is_and { '&' } else { '|' };
    let ids = raw.split(sep)
        .map(|seg| seg.split('#').filter_map(|v| v.parse().ok()).collect())
        .filter(|v: &Vec<i32>| !v.is_empty())
        .collect();
    (ids, is_and)
}

fn resolve(cond_type: ConditionType, target: Target, owner_uid: i64) -> Option<(Hook, Checker)> {
    match cond_type {
        ConditionType::_8Dead => Some(dead::resolve(target, owner_uid)),
        ConditionType::_17TeammateDead => Some(teammate_dead::resolve(owner_uid)),
        ConditionType::_5EnterFight => Some(enter_fight::resolve(target, owner_uid)),
        other => {
            tracing::warn!("unimplemented condition type: {:?}", other);
            None
        }
    }
}

pub fn parse(raw: &str, cond_target: i32, owner_uid: i64) -> Option<Condition> {
    let (ids, is_and) = parse_ids(raw);
    if ids.is_empty() { return None; }

    let target = Target::from_id(cond_target);

    let resolved: Vec<(Hook, Checker)> = ids
        .iter()
        .filter_map(|seg| {
            let cond_type = super::condition_type::condition_type(*seg.first()?)?;
            resolve(cond_type, target, owner_uid)
        })
        .collect();

    if resolved.is_empty() { return None; }

    let hook = resolved.into_iter().fold(
        (None::<Hook>, Vec::<Checker>::new()),
        |(hook, mut checkers), (h, c)| {
            checkers.push(c);
            (hook.or(Some(h)), checkers)
        },
    );
    let (hook, checkers) = (hook.0?, hook.1);

    let check: Checker = if is_and {
        Box::new(move |eval| checkers.iter().all(|f| f(eval)))
    } else {
        Box::new(move |eval| checkers.iter().any(|f| f(eval)))
    };

    Some(Condition { hook, ids, check })
}
