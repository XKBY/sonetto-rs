use sonettobuf::Fight;

use super::super::rule::collect::collect_rules;
use super::traits::Manager;
use super::fight_data_mgr::Managers;
use crate::state::battle::{effect, event::Event};

#[derive(Default, Clone)]
pub struct RuleMgr {
    pub effects: Vec<effect::SkillEffect>,
}

impl std::fmt::Debug for RuleMgr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleMgr").field("effects_count", &self.effects.len()).finish()
    }
}

fn all_entity_uids(fight: &Fight) -> Vec<i64> {
    let attacker = fight.attacker.as_ref().into_iter()
        .flat_map(|a| a.entitys.iter().chain(a.sub_entitys.iter()));
    let defender = fight.defender.as_ref().into_iter()
        .flat_map(|d| d.entitys.iter().chain(d.sub_entitys.iter()));
    attacker.chain(defender).filter_map(|e| e.uid).collect()
}

impl RuleMgr {
    pub fn new(fight: &Fight) -> Self {
        let uids = all_entity_uids(fight);
        let effects = collect_rules(fight)
            .into_iter()
            .flat_map(|(prefix, effect_id)| {
                uids.iter().filter_map(move |&uid| {
                    let matched = match prefix {
                        // 1: player, 2: enemy, 3: all
                        1 => uid >= 0,
                        2 => uid < 0,
                        _ => true,
                    };
                    if !matched { return None; }
                    tracing::info!(prefix, effect_id, uid, "rule_mgr: parsing rule for matched uid");
                    effect::parser::parse(effect_id, uid)
                })
            })
            .collect();
        Self { effects }
    }

    pub fn on_enter_fight(fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let effects = managers.rule_mgr.effects.clone();
        effects.into_iter().flat_map(|mut e| {
            let mut evs = e.on_enter_fight(fight, managers, entity_uid);
            for ev in &evs { tracing::info!("rule_mgr entity={} event={:?}", entity_uid, ev); }
            evs
        }).collect()
    }

    pub fn on_dead(fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let effects = managers.rule_mgr.effects.clone();
        effects.into_iter().flat_map(|mut e| e.on_dead(fight, managers, entity_uid)).collect()
    }

    pub fn on_round_start(fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let effects = managers.rule_mgr.effects.clone();
        effects.into_iter().flat_map(|mut e| e.on_round_start(fight, managers, entity_uid)).collect()
    }

    pub fn on_round_end(fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let effects = managers.rule_mgr.effects.clone();
        effects.into_iter().flat_map(|mut e| e.on_round_end(fight, managers, entity_uid)).collect()
    }

    pub fn on_battle_start(fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let effects = managers.rule_mgr.effects.clone();
        effects.into_iter().flat_map(|mut e| e.on_battle_start(fight, managers, entity_uid)).collect()
    }
}

impl Manager for RuleMgr {}
