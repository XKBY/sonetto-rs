use sonettobuf::Fight;
use crate::state::battle::{
    buff::Buff,
    effect::{SkillEffect, condition::Hook},
    event::Event,
    manager::fight_data_mgr::Managers,
};

enum HookPayload {
    Effect(SkillEffect, i64),
    Buff(Buff, i64),
}

struct HookEntry {
    priority: i32, // TODO: derive from effect/condition config
    payload: HookPayload,
}

impl HookEntry {
    fn fire(self, hook: Hook, fight: &Fight, managers: &mut Managers) -> Vec<Event> {
        match self.payload {
            HookPayload::Effect(mut e, owner_uid) => e.fire_hook(hook, fight, managers, owner_uid),
            HookPayload::Buff(b, entity_uid) => b.fire_hook(hook, fight, managers, entity_uid),
        }
    }
}

fn sort_entries(entries: &mut Vec<HookEntry>) {
    // TODO: sort by priority once priority field is populated from config
    let _ = entries;
}

fn collect_passive(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.passive_mgr.get(entity_uid).iter().cloned()
        .map(|e| HookEntry { priority: 0, payload: HookPayload::Effect(e, entity_uid) })
        .collect()
}

fn collect_rule(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.rule_mgr.effects.iter().cloned()
        .map(|e| HookEntry { priority: 0, payload: HookPayload::Effect(e, entity_uid) })
        .collect()
}

fn collect_buff(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default()
        .into_iter()
        .map(|b| HookEntry { priority: 0, payload: HookPayload::Buff(b, entity_uid) })
        .collect()
}

fn collect_active(managers: &Managers) -> Vec<HookEntry> {
    let Some(idx) = managers.active_effect_mgr.active_idx else {
        return Vec::new();
    };
    managers.active_effect_mgr.map.get(&idx)
        .into_iter()
        .flat_map(|effects| effects.iter().cloned().map(|e| {
            let owner = e.owner_uid;
            HookEntry { priority: 0, payload: HookPayload::Effect(e, owner) }
        }))
        .collect()
}

pub fn on_buff_add(managers: &mut Managers, fight: &Fight, target_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::BuffAdd, target_uid)
}

pub fn on_dead(managers: &mut Managers, fight: &Fight, entity_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::Dead, entity_uid)
}

pub fn on_eval_active_skill(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::EvalActiveSkill, caster_uid)
}

pub fn on_use_ex_skill(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::UseExSkill, caster_uid)
}

pub fn on_eval_being_attacked(managers: &mut Managers, fight: &Fight, defender_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::EvalBeingAttacked, defender_uid)
}

pub fn on_after_action(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::AfterAction, caster_uid)
}

pub fn fire_hook(
    managers: &mut Managers,
    fight: &Fight,
    hook: Hook,
    entity_uid: i64,
) -> Vec<Event> {
    let mut entries = collect_buff(managers, entity_uid);
    entries.extend(collect_rule(managers, entity_uid));
    entries.extend(collect_passive(managers, entity_uid));
    entries.extend(collect_active(managers));
    sort_entries(&mut entries);
    let mut events = Vec::new();
    for entry in entries {
        let mut snapshot = managers.clone();
        events.extend(entry.fire(hook, fight, &mut snapshot));
        *managers = snapshot;
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::battle::effect::SkillEffect;
    use crate::state::battle::manager::fight_data_mgr::Managers;

    fn fresh_managers() -> Managers {
        Managers::default()
    }

    #[test]
    fn collect_active_returns_empty_when_active_idx_none() {
        let mut m = fresh_managers();
        let _ = m.active_effect_mgr.push(vec![SkillEffect::empty(1)]);
        // active_idx left as None
        assert!(collect_active(&m).is_empty());
    }

    #[test]
    fn collect_active_returns_only_named_entry() {
        let mut m = fresh_managers();
        let outer = m.active_effect_mgr.push(vec![SkillEffect::empty(100)]);
        let inner = m.active_effect_mgr.push(vec![SkillEffect::empty(200)]);

        m.active_effect_mgr.active_idx = Some(inner);
        let entries = collect_active(&m);
        assert_eq!(entries.len(), 1);
        match &entries[0].payload {
            HookPayload::Effect(e, owner) => {
                assert_eq!(*owner, 200);
                assert_eq!(e.owner_uid, 200);
            }
            HookPayload::Buff(_, _) => panic!("expected Effect payload"),
        }

        m.active_effect_mgr.active_idx = Some(outer);
        let entries = collect_active(&m);
        assert_eq!(entries.len(), 1);
        match &entries[0].payload {
            HookPayload::Effect(_, owner) => assert_eq!(*owner, 100),
            HookPayload::Buff(_, _) => panic!("expected Effect payload"),
        }
    }

    #[test]
    fn collect_active_returns_empty_when_idx_absent_from_map() {
        let mut m = fresh_managers();
        m.active_effect_mgr.active_idx = Some(999);
        assert!(collect_active(&m).is_empty());
    }
}
