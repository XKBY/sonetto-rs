use sonettobuf::{Fight, FightEntityInfo};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct EntityLocation {
    pub is_attacker: bool,
    pub index: usize,
}

#[derive(Default, Debug, Clone)]
pub struct FightEntityDataMgr {
    entity_cache: HashMap<i64, EntityLocation>,
}

impl FightEntityDataMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut mgr = Self::default();
        mgr.rebuild_cache(fight);
        mgr
    }

    pub fn rebuild_cache(&mut self, fight: &Fight) {
        self.entity_cache.clear();

        if let Some(attacker) = &fight.attacker {
            for (idx, entity) in attacker.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(
                        uid,
                        EntityLocation {
                            is_attacker: true,
                            index: idx,
                        },
                    );
                }
            }
            for (idx, entity) in attacker.sub_entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(
                        uid,
                        EntityLocation {
                            is_attacker: true,
                            index: idx,
                        },
                    );
                }
            }
        }

        if let Some(defender) = &fight.defender {
            for (idx, entity) in defender.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(
                        uid,
                        EntityLocation {
                            is_attacker: false,
                            index: idx,
                        },
                    );
                }
            }
        }
    }

    pub fn get_location(&self, entity_id: i64) -> Option<EntityLocation> {
        self.entity_cache.get(&entity_id).copied()
    }

    #[allow(dead_code)]
    pub fn get_team_entities<'a>(
        &self,
        fight: &'a Fight,
        team_type: i32,
    ) -> Vec<&'a FightEntityInfo> {
        let mut entities = Vec::new();

        if let Some(attacker) = &fight.attacker {
            entities.extend(
                attacker
                    .entitys
                    .iter()
                    .filter(|e| e.team_type == Some(team_type)),
            );
        }

        if let Some(defender) = &fight.defender {
            entities.extend(
                defender
                    .entitys
                    .iter()
                    .filter(|e| e.team_type == Some(team_type)),
            );
        }

        entities
    }
}

pub fn get_entity_mut_by_location(
    fight: &mut Fight,
    location: EntityLocation,
) -> Option<&mut FightEntityInfo> {
    if location.is_attacker {
        fight.attacker.as_mut()?.entitys.get_mut(location.index)
    } else {
        fight.defender.as_mut()?.entitys.get_mut(location.index)
    }
}
