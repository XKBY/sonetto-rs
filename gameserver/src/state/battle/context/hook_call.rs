use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::{buff_mgr::BuffMgr, fight_data_mgr::Managers, rule_mgr::RuleMgr, passive_mgr::PassiveMgr, traits::Manager};

fn with_snapshot<F>(managers: &mut Managers, f: F) -> Vec<Event>
where
    F: FnOnce(&mut Managers) -> Vec<Event>,
{
    let mut snapshot = managers.clone();
    let events = f(&mut snapshot);
    *managers = snapshot;
    events
}

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
    managers.entity_mgr.set_action_point(entity_uid, 1);
    let buffs = managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default();
    let mut events = managers.cloth_mgr.on_enter_fight(fight, entity_uid);
    events.extend(with_snapshot(managers, |s| BuffMgr::on_enter_fight(&buffs, fight, s, entity_uid)));
    events.extend(with_snapshot(managers, |s| RuleMgr::on_enter_fight(fight, s, entity_uid)));
    events.extend(with_snapshot(managers, |s| PassiveMgr::on_enter_fight(fight, s, entity_uid)));
    events
}

pub fn on_use_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default();
    let mut events = managers.cloth_mgr.on_use_card(fight, events);
    events.extend(with_snapshot(managers, |s| BuffMgr::on_use_card(&buffs, fight, s, entity_uid)));
    events
}

pub fn on_move_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default();
    let mut events = managers.cloth_mgr.on_move_card(fight, events);
    events.extend(with_snapshot(managers, |s| BuffMgr::on_move_card(&buffs, fight, s, entity_uid)));
    events
}

pub fn on_compose_card(managers: &mut Managers, fight: &Fight, events: Vec<Event>, entity_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default();
    let mut events = managers.cloth_mgr.on_compose_card(fight, events);
    events.extend(with_snapshot(managers, |s| BuffMgr::on_compose_card(&buffs, fight, s, entity_uid)));
    events
}

pub fn on_dead(managers: &mut Managers, fight: &Fight, entity_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default();
    let mut events = vec![
        Event::Dead { entity_uid },
        Event::RemoveEntityCards { entity_uid },
    ];
    events.extend(managers.cloth_mgr.on_dead(fight, entity_uid));
    events.extend(with_snapshot(managers, |s| BuffMgr::on_dead(&buffs, fight, s, entity_uid)));
    events.extend(with_snapshot(managers, |s| RuleMgr::on_dead(fight, s, entity_uid)));
    events.extend(with_snapshot(managers, |s| PassiveMgr::on_dead(fight, s, entity_uid)));
    managers.entity_mgr.action_points.remove(&entity_uid);
    events
}

pub fn on_buff_add(managers: &mut Managers, fight: &Fight, target_uid: i64) -> Vec<Event> {
    let buffs = managers.buff_mgr.active_buff.get(&target_uid).cloned().unwrap_or_default();
    with_snapshot(managers, |s| BuffMgr::on_buff_add(&buffs, fight, s, target_uid))
}
