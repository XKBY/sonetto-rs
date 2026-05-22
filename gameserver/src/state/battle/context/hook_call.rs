use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::{buff_mgr::BuffMgr, fight_data_mgr::Managers, rule_mgr::RuleMgr, traits::Manager};

pub fn on_battle_start(managers: &mut Managers, fight: &mut Fight) {
    managers.cloth_mgr.on_battle_start(fight);
    BuffMgr::on_battle_start(fight, managers);
}

pub fn on_round_end(managers: &mut Managers, fight: &mut Fight) {
    managers.buff_mgr.on_round_end(fight);
    managers.calculate_mgr.on_round_end();
    managers.cloth_mgr.on_round_end(fight);
    BuffMgr::on_round_end_hooks(fight, managers);
}

pub fn on_enter_fight(managers: &mut Managers, fight: &Fight, entity_uid: i64) -> Vec<Event> {
    let mut events = managers.cloth_mgr.on_enter_fight(fight, entity_uid);
    let buffs = managers.buff_mgr.active_buff.remove(&entity_uid).unwrap_or_default();
    events.extend(BuffMgr::on_enter_fight(&buffs, fight, managers, entity_uid));
    managers.buff_mgr.active_buff.insert(entity_uid, buffs);
    // Rules are snapshotted before execution. If a rule adds another rule mid-flight,
    // the new rule won't fire this event — ordering of dynamically-added rules is unresolved.
    let rule_effects = std::mem::take(&mut managers.rule_mgr.effects);
    events.extend(RuleMgr::on_enter_fight(&rule_effects, fight, managers, entity_uid));
    managers.rule_mgr.effects = rule_effects;
    events
}

pub fn on_use_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let mut events = managers.cloth_mgr.on_use_card(fight, events);
    let buffs = managers.buff_mgr.active_buff.remove(&entity_uid).unwrap_or_default();
    events.extend(BuffMgr::on_use_card(&buffs, fight, managers, entity_uid));
    managers.buff_mgr.active_buff.insert(entity_uid, buffs);
    events
}

pub fn on_move_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let mut events = managers.cloth_mgr.on_move_card(fight, events);
    let buffs = managers.buff_mgr.active_buff.remove(&entity_uid).unwrap_or_default();
    events.extend(BuffMgr::on_move_card(&buffs, fight, managers, entity_uid));
    managers.buff_mgr.active_buff.insert(entity_uid, buffs);
    events
}

pub fn on_compose_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let mut events = managers.cloth_mgr.on_compose_card(fight, events);
    let buffs = managers.buff_mgr.active_buff.remove(&entity_uid).unwrap_or_default();
    events.extend(BuffMgr::on_compose_card(&buffs, fight, managers, entity_uid));
    managers.buff_mgr.active_buff.insert(entity_uid, buffs);
    events
}

pub fn on_dead(managers: &mut Managers, fight: &Fight, entity_uid: i64) -> Vec<Event> {
    let mut events = vec![
        Event::Dead { entity_uid },
        Event::RemoveEntityCards { entity_uid },
    ];
    events.extend(managers.cloth_mgr.on_dead(fight, entity_uid));
    let buffs = managers.buff_mgr.active_buff.remove(&entity_uid).unwrap_or_default();
    events.extend(BuffMgr::on_dead(&buffs, fight, managers, entity_uid));
    managers.buff_mgr.active_buff.insert(entity_uid, buffs);
    // Rules are snapshotted before execution. If a rule adds another rule mid-flight,
    // the new rule won't fire this event — ordering of dynamically-added rules is unresolved.
    let rule_effects = std::mem::take(&mut managers.rule_mgr.effects);
    events.extend(RuleMgr::on_dead(&rule_effects, fight, managers, entity_uid));
    managers.rule_mgr.effects = rule_effects;
    events
}

pub fn on_buff_add(managers: &mut Managers, fight: &Fight, target_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.remove(&target_uid).unwrap_or_default();
    let events = BuffMgr::on_buff_add(&buffs, fight, managers, target_uid);
    managers.buff_mgr.active_buff.insert(target_uid, buffs);
    events
}
