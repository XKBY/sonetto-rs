use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::context::hook_call::on_buff_add;

pub fn execute(fight: &Fight, managers: &mut Managers, _mechanics: &mut crate::state::battle::mechanics::Mechanics, _executor: &mut crate::state::battle::skill::SkillExecutor, _rng: &mut rand::rngs::StdRng, targets: Vec<i64>, raw: &str, count: i32) -> Vec<Event> {
    let buff_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    targets.iter()
        .flat_map(|&t| {
            let mut events = Vec::new();
            for _ in 0..count {
                events.extend(managers.buff_mgr.add_buff(t, buff_id));
            }
            events.extend(on_buff_add(managers, fight, t));
            events
        })
        .collect()
}

pub fn execute_reversed(managers: &mut Managers, targets: Vec<i64>, raw: &str) -> Vec<Event> {
    let buff_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    for &t in &targets {
        if let Some(buff) = managers.buff_mgr.active_buff.get_mut(&t)
            .and_then(|bs| bs.iter_mut().find(|b| b.buff_id == buff_id))
        {
            buff.stacks = (buff.stacks - 1).max(0);
        }
    }
    vec![]
}
