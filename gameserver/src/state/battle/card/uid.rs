use std::sync::atomic::{AtomicI64, Ordering};

static CARD_UID: AtomicI64 = AtomicI64::new(1);

#[allow(dead_code)]
pub fn next_card_uid() -> i64 {
    CARD_UID.fetch_add(1, Ordering::SeqCst)
}

#[allow(dead_code)]
pub fn reset_card_uid() {
    CARD_UID.store(1, Ordering::SeqCst);
}
