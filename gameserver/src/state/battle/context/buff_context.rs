use crate::state::battle::manager::buff_mgr::{BuffInstance, BuffMgr};

/// Lifecycle/policy context around buff storage.
pub struct BuffContext<'a> {
    pub store: &'a mut BuffMgr,
}

impl<'a> BuffContext<'a> {
    pub fn new(store: &'a mut BuffMgr) -> Self {
        Self { store }
    }

    pub fn buffs(&self, uid: i64) -> &[BuffInstance] {
        self.store.get(uid)
    }

    pub fn add(&mut self, target_uid: i64, buff_id: i32, from_uid: i64, count: i32, layer: i32) {
        self.store.add(target_uid, buff_id, from_uid, count, layer);
    }

    pub fn add_with_uid(
        &mut self,
        target_uid: i64,
        buff_id: i32,
        from_uid: i64,
        count: i32,
        layer: i32,
        buff_uid: i64,
    ) {
        self.store
            .add_with_uid(target_uid, buff_id, from_uid, count, layer, buff_uid);
    }

    pub fn remove_by_uid(&mut self, uid: i64, buff_uid: i64) {
        self.store.remove_by_uid(uid, buff_uid);
    }
}
