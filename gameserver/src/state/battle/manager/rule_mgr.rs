use sonettobuf::Fight;

use super::super::rule::collect::collect_rules;
use super::traits::Manager;
use crate::state::battle::{effect, event::Event};

#[derive(Default)]
pub struct RuleMgr {
    effects: Vec<effect::SkillEffect>,
}

impl std::fmt::Debug for RuleMgr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleMgr").field("effects_count", &self.effects.len()).finish()
    }
}

impl Clone for RuleMgr {
    fn clone(&self) -> Self { Self::default() }
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
            .flat_map(|(_, prefix, effect_id)| {
                uids.iter().filter_map(move |&uid| {
                    let matched = match prefix {
                        // 1: all, 2: player, 3: enemy
                        1 => true,
                        2 => uid >= 0,
                        3 => uid < 0,
                        _ => true,
                    };

                    if !matched {
                        return None;
                    }

                    tracing::info!(
                        prefix,
                        effect_id,
                        uid,
                        "rule_mgr: parsing rule for matched uid"
                    );

                    effect::parser::parse(effect_id, uid)
                })
            })
            .collect();
        Self { effects }
    }
}

impl Manager for RuleMgr {
    fn on_enter_fight(&mut self, fight: &Fight, entity_uid: i64) -> Vec<Event> {
        let events: Vec<Event> = self.effects.iter().flat_map(|e| e.on_enter_fight(fight, entity_uid)).collect();
        for event in &events {
            tracing::info!("rule_mgr entity={} event={:?}", entity_uid, event);
        }
        events
    }

    fn on_dead(&mut self, fight: &Fight, entity_uid: i64) -> Vec<Event> {
        self.effects.iter().flat_map(|e| e.on_dead(fight, entity_uid)).collect()
    }
}
