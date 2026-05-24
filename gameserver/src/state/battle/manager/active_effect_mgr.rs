use std::collections::HashMap;
use crate::state::battle::effect::{SkillEffect, condition::Hook};
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use sonettobuf::Fight;

#[derive(Default, Debug, Clone)]
pub struct ActiveEffectMgr {
    next_idx: usize,
    pub map: HashMap<usize, Vec<SkillEffect>>,
}

impl ActiveEffectMgr {
    pub fn push(&mut self, effects: Vec<SkillEffect>) -> usize {
        let idx = self.next_idx;
        self.next_idx += 1;
        self.map.insert(idx, effects);
        idx
    }

    pub fn void(&mut self, idx: usize) {
        self.map.remove(&idx);
    }
}

impl Managers {
    pub fn fire_active_effects_hook(
        &mut self,
        hook: Hook,
        fight: &Fight,
        entity_uid: i64,
    ) -> Vec<Event> {
        let mut all_effects = std::mem::take(&mut self.active_effect_mgr.map);
        let mut events = Vec::new();
        for effects in all_effects.values_mut() {
            for e in effects.iter_mut() {
                events.extend(e.fire_hook(hook, fight, self, entity_uid));
            }
        }
        self.active_effect_mgr.map = all_effects;
        events
    }
}
