use sonettobuf::Fight;
use crate::state::battle::{
    effect::condition::Hook,
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    utils::find_entity,
};
use super::BuffActExecutor;

pub struct ShieldAct;

impl BuffActExecutor for ShieldAct {
    const HOOKS: &'static [Hook] = &[Hook::BuffAdd];

    fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, params: &str, _carrier_buff_id: i32) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
        let mut parts = params.split('#');
        let _id = parts.next();
        let use_missing: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let attr_id: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let permille: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        if permille == 0 { return (vec![], vec![]); }

        let entity = find_entity(fight, entity_uid);
        let base = if use_missing != 0 {
            let max_hp = entity.and_then(|e| e.attr.as_ref()).and_then(|a| a.hp).unwrap_or(0);
            let cur_hp = managers.entity_mgr.current_hp.get(&entity_uid).copied().unwrap_or(0);
            (max_hp - cur_hp).max(0)
        } else {
            let attr = entity.and_then(|e| e.attr.as_ref());
            match attr_id {
                100 => managers.entity_mgr.current_hp.get(&entity_uid).copied().unwrap_or(0),
                101 => attr.and_then(|a| a.hp).unwrap_or(0),
                102 => attr.and_then(|a| a.attack).unwrap_or(0),
                103 => attr.and_then(|a| a.defense).unwrap_or(0),
                _ => 0,
            }
        };

        let amount = base * permille / 1000;
        if amount <= 0 { return (vec![], vec![]); }

        managers.entity_mgr.shields.entry(entity_uid).or_default().push(amount);

        let effect = ActEffectBuilder::shield(entity_uid, amount);
        (vec![Event::SerializedActEffect { effect }], vec![])
    }
}
