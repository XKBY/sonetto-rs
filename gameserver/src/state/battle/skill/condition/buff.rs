use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;
use crate::state::battle::manager::buff_mgr::BuffMgr;
use std::collections::HashSet;

pub(super) struct Buff;

impl Condition for Buff {
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        parse(parts, cond_type)
    }

    fn check(&self, condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        check(condition, ctx.buff_mgr, ctx.target_uid)
    }
}

fn resolve_buff_category(id: i32) -> Option<i32> {
    if id <= 0 {
        return None;
    }

    let cfg = config::configs::get();

    if let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == id) {
        let type_id = buff.type_id;
        return cfg
            .skill_bufftype
            .iter()
            .find(|t| t.id == type_id)
            .map(|t| t.r#type);
    }

    cfg.skill_bufftype
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.r#type)
}

pub fn deleted_matches(deleted_buff_ids: &[i32], wanted_ids: &[i32]) -> bool {
    if wanted_ids.is_empty() || deleted_buff_ids.is_empty() {
        return false;
    }

    if wanted_ids.iter().any(|id| deleted_buff_ids.contains(id)) {
        return true;
    }

    let deleted_categories: HashSet<i32> = deleted_buff_ids
        .iter()
        .filter_map(|id| resolve_buff_category(*id))
        .collect();
    if deleted_categories.is_empty() {
        return false;
    }

    wanted_ids
        .iter()
        .filter_map(|id| resolve_buff_category(*id))
        .any(|wanted_category| deleted_categories.contains(&wanted_category))
}

pub fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
    match cond_type {
        "HasBuffId" => {
            let ids = parts[1..]
                .iter()
                .flat_map(|p| p.split(','))
                .filter_map(|v| v.trim_end_matches('!').trim_end_matches('！').parse().ok())
                .collect();
            Some(ConditionType::HasBuffId { buff_ids: ids })
        }
        "NoBuffId" => {
            let ids: Vec<i32> = if parts.get(1).map(|p| p.contains(',')).unwrap_or(false) {
                parts[1]
                    .split(',')
                    .filter_map(|v| v.trim_end_matches('!').trim_end_matches('！').parse().ok())
                    .collect()
            } else {
                parts[1..]
                    .iter()
                    .filter_map(|v| v.trim_end_matches('!').trim_end_matches('！').parse().ok())
                    .collect()
            };
            Some(ConditionType::NoBuffId { buff_ids: ids })
        }
        "BuffIdDel" => {
            let ids = parts[1..]
                .iter()
                .flat_map(|p| p.split('#'))
                .filter_map(|v| v.trim_end_matches('!').trim_end_matches('！').parse().ok())
                .collect();
            Some(ConditionType::BuffIdDel { buff_ids: ids })
        }
        "HasTypeIdBuffMoreThan" => Some(ConditionType::HasTypeIdBuffMoreThan {
            type_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            min_count: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "TypeIdBuffCountMoreThan" => Some(ConditionType::TypeIdBuffCountMoreThan {
            type_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_count: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "TypeIdBuffCountLessThan" => Some(ConditionType::TypeIdBuffCountLessThan {
            type_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_count: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "HasTypeIdBuffEqual" => Some(ConditionType::HasTypeIdBuffEqual {
            type_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_count: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
        }),
        "PerBuffIdCount" => Some(ConditionType::PerBuffIdCount {
            buff_ids: parts
                .get(1)
                .and_then(|v| v.parse().ok())
                .map(|id| vec![id])
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

pub fn check(condition: &ConditionType, buff_mgr: &BuffMgr, condition_uid: i64) -> Option<bool> {
    match condition {
        ConditionType::HasBuffId { buff_ids } => {
            Some(buff_ids.iter().any(|&bid| {
                buff_mgr.has(condition_uid, bid) || buff_mgr.has_type(condition_uid, bid)
            }))
        }
        ConditionType::NoBuffId { buff_ids } => Some(buff_ids.iter().all(|&bid| {
            !buff_mgr.has(condition_uid, bid) && !buff_mgr.has_type(condition_uid, bid)
        })),
        ConditionType::BuffIdDel { .. } => Some(false),
        ConditionType::HasTypeIdBuffMoreThan { type_id, min_count } => {
            Some(buff_mgr.count_type(condition_uid, *type_id) >= *min_count)
        }
        ConditionType::TypeIdBuffCountMoreThan { type_id, max_count } => {
            Some(buff_mgr.count_type(condition_uid, *type_id) >= *max_count)
        }
        ConditionType::TypeIdBuffCountLessThan { type_id, max_count } => {
            Some(buff_mgr.count_type(condition_uid, *type_id) <= *max_count)
        }
        ConditionType::HasTypeIdBuffEqual { type_id, max_count } => {
            Some(buff_mgr.count_type(condition_uid, *type_id) >= *max_count)
        }
        ConditionType::PerBuffIdCount { buff_ids } => {
            Some(buff_mgr.count_buff_ids(condition_uid, buff_ids) > 0)
        }
        _ => None,
    }
}
