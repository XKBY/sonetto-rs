use crate::state::battle::event::Event;

pub fn execute(targets: Vec<i64>, delta: i32) -> Vec<Event> {
    targets
        .into_iter()
        .map(|target| Event::ExPointChange { target, delta, emit_step: true })
        .collect()
}
