use super::super::super::ConditionType;
use super::{bloodtithe, buff, career, combat, enter_fight, ex_point, life, misc};
use config::configs;

pub fn parse_condition(raw: &str) -> (ConditionType, bool) {
    if raw.is_empty() {
        return (ConditionType::None, false);
    }
    // Leading ! or ！ = negated condition
    let (mut negated, raw) = if raw.starts_with('!') || raw.starts_with('！') {
        (true, raw.trim_start_matches('!').trim_start_matches('！'))
    } else {
        (false, raw)
    };

    // Trailing ! or ！ on the raw string also means negated (e.g. "19203#31260131！")
    if !negated && (raw.ends_with('!') || raw.ends_with('！')) {
        negated = true;
    }
    if raw.contains('&') {
        let parts: Vec<ConditionType> = raw.split('&').map(parse_single).collect();
        return (ConditionType::EnterFightAnd(parts), negated);
    }
    if raw.contains('|') {
        let parts: Vec<ConditionType> = raw.split('|').map(parse_single).collect();
        return (ConditionType::EnterFightOr(parts), negated);
    }
    (parse_single(raw), negated)
}

pub fn parse_single(raw: &str) -> ConditionType {
    let cfg = configs::get();
    if raw.is_empty() {
        return ConditionType::None;
    }

    let parts: Vec<&str> = raw.split('#').collect();
    let id: i32 = parts[0].parse().unwrap_or(0);

    let cond_type = cfg
        .skill_behavior_condition
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.r#type.as_str())
        .unwrap_or("");

    enter_fight::parse(id, cond_type)
        .or_else(|| buff::parse(&parts, cond_type))
        .or_else(|| career::parse(&parts, cond_type))
        .or_else(|| life::parse(&parts, cond_type))
        .or_else(|| ex_point::parse(&parts, cond_type))
        .or_else(|| combat::parse(&parts, cond_type))
        .or_else(|| misc::parse(&parts, cond_type))
        .or_else(|| bloodtithe::parse(&parts, cond_type))
        .unwrap_or_else(|| {
            if !cond_type.is_empty() {
                tracing::warn!("Unknown condition type: {} (id={})", cond_type, id);
            }
            ConditionType::Unknown {
                raw: raw.to_string(),
            }
        })
}
