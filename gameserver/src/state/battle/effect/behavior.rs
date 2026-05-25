use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use super::behaviour_type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;
pub mod attr_modify;

#[derive(Debug, Clone)]
pub struct Behaviour {
    pub raw: String,
    pub target: i32,
}

pub fn is_attr_fix(raw: &str) -> bool {
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    matches!(
        behaviour_type(id),
        Some(BehaviourType::_10004AttrFix)
            | Some(BehaviourType::_10011AttrFixBuff)
            | Some(BehaviourType::_60033AttrFixByLoseHp)
    )
}

pub fn parse(raw: &str, beh_target: i32) -> Vec<Behaviour> {
    raw.split('|')
        .filter(|s| !s.is_empty())
        .map(|seg| Behaviour { raw: seg.to_string(), target: beh_target })
        .collect()
}

pub fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32, count: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_20002AddExPoint) => ex_point::execute(managers, targets, raw, count),
        Some(BehaviourType::_1AddBuff) => add_buff::execute(fight, managers, targets, raw, count),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute(managers, targets, raw, count),
        Some(BehaviourType::_10004AttrFix)
        | Some(BehaviourType::_10011AttrFixBuff)
        | Some(BehaviourType::_60033AttrFixByLoseHp) => {
            // `targets` is already resolved from `beh_target` above, so each
            // resolved target gets its own attr-bonus entry merged into
            // entity_mgr. AttrFix can target self/ally/enemy depending on
            // the slot's behavior_target — this loop honors that.
            targets
                .iter()
                .flat_map(|&t| attr_modify::execute(fight, managers, t, raw, count))
                .collect()
    #[test]
    fn calculate_bonus_dispatches_attr_fix_for_self() {
        use sonettobuf::{Fight, FightEntityInfo, FightSide, EntityAttr};
        let fight = Fight {
            attacker: Some(FightSide {
                entitys: vec![FightEntityInfo {
                    uid: Some(7),
                    current_hp: Some(1000),
                    attr: Some(EntityAttr { hp: Some(1000), ..Default::default() }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mgrs = Managers::default();
        // beh_target=103 is Target::Self_
        let out = calculate_bonus(&fight, &mgrs, 7, "10004#102#50", 103, 1);
        assert_eq!(out.get(&(7, 102)).copied(), Some(50));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn calculate_bonus_resolves_per_target_for_all_ally() {
        use sonettobuf::{Fight, FightEntityInfo, FightSide, EntityAttr};
        let fight = Fight {
            attacker: Some(FightSide {
                entitys: vec![
                    FightEntityInfo { uid: Some(1), current_hp: Some(1000),
                        attr: Some(EntityAttr { hp: Some(1000), ..Default::default() }),
                        ..Default::default() },
                    FightEntityInfo { uid: Some(2), current_hp: Some(1000),
                        attr: Some(EntityAttr { hp: Some(1000), ..Default::default() }),
                        ..Default::default() },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mgrs = Managers::default();
        // beh_target=101 is Target::AllAlly — slot owner is uid 1, both allies should get +50
        let out = calculate_bonus(&fight, &mgrs, 1, "10004#102#50", 101, 1);
        assert_eq!(out.get(&(1, 102)).copied(), Some(50));
        assert_eq!(out.get(&(2, 102)).copied(), Some(50));
    }

    #[test]
    fn calculate_bonus_returns_empty_for_non_attr_fix() {
        use sonettobuf::Fight;
        let fight = Fight::default();
        let mgrs = Managers::default();
        let out = calculate_bonus(&fight, &mgrs, 7, "1#100", 103, 1);
        assert!(out.is_empty());
    }
}

        Some(other) => { tracing::warn!("unimplemented behaviour type: {:?}", other); vec![] }
        None => vec![],
    }
}

pub fn execute_reversed(fight: &Fight, managers: &mut Managers, entity_uid: i64, raw: &str, beh_target: i32) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(BehaviourType::_1AddBuff) => add_buff::execute_reversed(managers, targets, raw),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute_reversed(managers, targets, raw),
            _ => vec![],
        }
    }

    pub fn calculate_bonus(
        fight: &Fight,
        managers: &Managers,
        entity_uid: i64,
        raw: &str,
        beh_target: i32,
        count: i32,
    ) -> std::collections::HashMap<(i64, i32), i32> {
        if !is_attr_fix(raw) {
            return std::collections::HashMap::new();
        }
        let targets = Target::from_id(beh_target).entities(fight, entity_uid);
        let mut out: std::collections::HashMap<(i64, i32), i32> = std::collections::HashMap::new();
        for t in targets {
            let m = super::behavior::attr_modify::calculate_bonus(fight, managers, t, raw, count);
            for (k, v) in m {
                let entry = out.entry(k).or_insert(0);
                *entry = entry.saturating_add(v);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_attr_fix_recognizes_attr_fix_family() {
        assert!(is_attr_fix("10004#102#50"));         // _10004AttrFix
        assert!(is_attr_fix("10011#102#50#1"));        // _10011AttrFixBuff
        assert!(is_attr_fix("60033#100#205#75#8"));    // _60033AttrFixByLoseHp
    }

    #[test]
    fn is_attr_fix_rejects_non_attr_fix() {
        assert!(!is_attr_fix("1#100"));        // _1AddBuff
        assert!(!is_attr_fix("20002#10"));     // _20002AddExPoint
        assert!(!is_attr_fix("10006#1000"));   // _10006Damage
    }

    #[test]
    fn is_attr_fix_handles_malformed_input() {
        assert!(!is_attr_fix(""));
        assert!(!is_attr_fix("abc"));
        assert!(!is_attr_fix("999999999"));
    }
}

