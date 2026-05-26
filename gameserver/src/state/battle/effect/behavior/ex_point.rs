use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;

pub fn execute(managers: &mut Managers, _mechanics: &mut crate::state::battle::mechanics::Mechanics, _executor: &mut crate::state::battle::skill::SkillExecutor, _rng: &mut rand::rngs::StdRng, targets: Vec<i64>, raw: &str, count: i32) -> Vec<Event> {
    let delta: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let total_delta = delta * count;
    targets
        .into_iter()
        .map(|target| {
            managers.entity_mgr.add_ex_point(target, total_delta);
            Event::ExPointChange { target, delta: total_delta, emit_step: true }
        })
        .collect()
}
