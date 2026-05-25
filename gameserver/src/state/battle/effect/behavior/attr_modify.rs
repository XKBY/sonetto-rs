use sonettobuf::Fight;
use std::collections::HashMap;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;

/// Compute the attr-bonus contribution this behaviour would add for `entity_uid`.
/// Returns a map keyed by `(uid, attr_id) -> amount`. The merge into
/// `entity_mgr.attr_bonus` is the caller's responsibility (Vec<i32> aggregation
/// happens at that layer).
///
/// Supports raw strings from the `_10004AttrFix`, `_10011AttrFixBuff`, and
/// `_60033AttrFixByLoseHp` BehaviourType variants.
pub fn calculate_bonus(
    fight: &Fight,
    _managers: &Managers,
    entity_uid: i64,
    raw: &str,
    count: i32,
) -> HashMap<(i64, i32), i32> {
    let mut parts = raw.split('#');
    let id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = HashMap::new();
    match id {
        // _10004AttrFix         "10004#<attr_id>#<amount>"
        // _10011AttrFixBuff     "10011#<attr_id>#<amount>#<buff_id>" (buff_id ignored at this layer)
        10004 | 10011 => {
            let attr_id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let amount: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let scaled = amount.saturating_mul(count.max(1));
            if attr_id != 0 && scaled != 0 {
                out.insert((entity_uid, attr_id), scaled);
            }
        }
        // _60033AttrFixByLoseHp "60033#<step_permille>#<attr_id>#<bonus_per_stack>#<max_stacks>"
        60033 => {
            let step_permille: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let attr_id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let bonus_per_stack: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let max_stacks: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            if step_permille <= 0 || bonus_per_stack <= 0 || max_stacks <= 0 || attr_id == 0 {
                return out;
            }
            let entity = fight
                .attacker
                .iter()
                .chain(fight.defender.iter())
                .flat_map(|s| s.entitys.iter().chain(s.sub_entitys.iter()))
                .find(|e| e.uid == Some(entity_uid));
            let Some(entity) = entity else { return out; };
            let max_hp = entity.attr.as_ref().and_then(|a| a.hp).unwrap_or(0);
            if max_hp <= 0 {
                return out;
            }
            let cur_hp = entity.current_hp.unwrap_or(0);
            let missing = (max_hp - cur_hp).max(0) as i64;
            let missing_permille = (missing * 1000 / max_hp as i64) as i32;
            let stacks = (missing_permille / step_permille).min(max_stacks);
            if stacks <= 0 {
                return out;
            }
            let bonus = stacks.saturating_mul(bonus_per_stack);
            out.insert((entity_uid, attr_id), bonus);
        }
        _ => {}
    }
    out
}

/// Side-effecting form: compute then merge into `managers.entity_mgr`. Returns
/// no events (the parallel "calculate_bonus" pass is what other code observes;
/// `execute` exists for the rare case where a behaviour fires outside of an
/// eval-hook attr-fix pass and needs to persist immediately).
pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    entity_uid: i64,
    raw: &str,
    count: i32,
) -> Vec<Event> {
    let map = calculate_bonus(fight, managers, entity_uid, raw, count);
    let mut nested: HashMap<(i64, i32), Vec<i32>> = HashMap::new();
    for (k, v) in map {
        nested.entry(k).or_default().push(v);
    }
    managers.entity_mgr.merge_attr_bonus(nested);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonettobuf::models::{Fight, FightEntityInfo, FightSide, EntityAttr};

    fn fight_with_entity(uid: i64, max_hp: i32, cur_hp: i32) -> Fight {
        let entity = FightEntityInfo {
            uid: Some(uid),
            current_hp: Some(cur_hp),
            attr: Some(EntityAttr { hp: Some(max_hp), ..Default::default() }),
            ..Default::default()
        };
        Fight {
            attacker: Some(FightSide { entitys: vec![entity], ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn attrfix_returns_single_entry_map() {
        let fight = fight_with_entity(1, 1000, 1000);
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 1, "10004#102#50", 1);
        assert_eq!(out.get(&(1, 102)).copied(), Some(50));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn attrfix_scales_by_count() {
        let fight = fight_with_entity(1, 1000, 1000);
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 1, "10004#102#50", 3);
        assert_eq!(out.get(&(1, 102)).copied(), Some(150));
    }

    #[test]
    fn attrfix_by_lose_hp_zero_step_returns_empty() {
        let fight = fight_with_entity(1, 1000, 500);
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 1, "60033#0#205#75#8", 1);
        assert!(out.is_empty());
    }

    #[test]
    fn attrfix_by_lose_hp_caps_at_max_stacks() {
        let fight = fight_with_entity(1, 1000, 0); // 100% missing → 1000‰ / 100 = 10 stacks, capped to 8
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 1, "60033#100#205#75#8", 1);
        assert_eq!(out.get(&(1, 205)).copied(), Some(75 * 8));
    }

    #[test]
    fn attrfix_by_lose_hp_zero_missing_returns_empty() {
        let fight = fight_with_entity(1, 1000, 1000);
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 1, "60033#100#205#75#8", 1);
        assert!(out.is_empty());
    }

    #[test]
    fn execute_merges_into_entity_mgr() {
        let fight = fight_with_entity(1, 1000, 1000);
        let mut mgrs = Managers::default();
        let events = execute(&fight, &mut mgrs, 1, "10004#102#50", 1);
        assert!(events.is_empty());
        assert_eq!(mgrs.entity_mgr.sum_attr_bonus(1, 102), 50);
    }
}