use std::collections::HashMap;

use sonettobuf::Fight;

use crate::state::battle::{
    effect::{self, SkillEffect},
    event::Event,
    manager::fight_data_mgr::Managers,
};

#[derive(Default, Debug, Clone)]
pub struct PassiveMgr {
    map: HashMap<i64, Vec<SkillEffect>>,
}

impl PassiveMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut map: HashMap<i64, Vec<SkillEffect>> = HashMap::new();

        let sides = [fight.attacker.as_ref(), fight.defender.as_ref()];
        for side in sides.into_iter().flatten() {
            for entity in side.entitys.iter().chain(side.sub_entitys.iter()) {
                let Some(uid) = entity.uid else { continue };
                if entity.position.unwrap_or(-1) <= 0 { continue }
                if entity.passive_skill.is_empty() { continue }

                let effects: Vec<SkillEffect> = entity
                    .passive_skill
                    .iter()
                    .filter_map(|&sid| {
                        tracing::info!(uid, sid, "passive_mgr: parsing passive for entity");
                        effect::parser::parse(sid, uid)
                    })
                    .collect();

                if !effects.is_empty() {
                    tracing::info!(uid, count = effects.len(), "passive_mgr: loaded passives for entity");
                    map.insert(uid, effects);
                }
            }
        }

        Self { map }
    }

    pub fn get(&self, uid: i64) -> &[SkillEffect] {
        self.map.get(&uid).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn on_enter_fight(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.get(entity_uid).iter().flat_map(|e| {
            let evs = e.on_enter_fight(fight, managers, entity_uid);
            for ev in &evs { tracing::info!("passive_mgr entity={} event={:?}", entity_uid, ev); }
            evs
        }).collect()
    }

    pub fn on_dead(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        self.get(entity_uid).iter().flat_map(|e| e.on_dead(fight, managers, entity_uid)).collect()
    }
}
