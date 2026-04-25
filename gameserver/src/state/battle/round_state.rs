use std::sync::atomic::{AtomicI32, Ordering};

static SIMULATED_ROUND: AtomicI32 = AtomicI32::new(0);

pub fn set_simulated_round(round: i32) {
    SIMULATED_ROUND.store(round, Ordering::Relaxed);
}

pub fn simulated_round() -> i32 {
    SIMULATED_ROUND.load(Ordering::Relaxed)
}
