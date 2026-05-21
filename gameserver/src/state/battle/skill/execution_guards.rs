use std::cell::RefCell;

pub(super) struct DepthGuard {
    pub depth: *mut usize,
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        // SAFETY: `depth` points to `SkillExecutor::call_depth` for the lifetime of execute_skill.
        unsafe {
            *self.depth = (*self.depth).saturating_sub(1);
        }
    }
}

pub(super) struct SkillContextGuard {
    pub current: *mut Option<(i32, i64)>,
    pub previous: Option<(i32, i64)>,
}

impl Drop for SkillContextGuard {
    fn drop(&mut self) {
        // SAFETY: `current` points to `SkillExecutor::current_skill_context` for the lifetime of `execute_skill`.
        unsafe {
            *self.current = self.previous;
        }
    }
}

thread_local! {
    static EXEC_SKILL_STACK: RefCell<Vec<(i64, i64, i32)>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct ReentryGuard {
    active: bool,
}

impl ReentryGuard {
    pub fn enter(caster_uid: i64, target_uid: i64, skill_id: i32) -> Option<Self> {
        let mut entered = false;
        EXEC_SKILL_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            let key = (caster_uid, target_uid, skill_id);
            if stack.contains(&key) {
                return;
            }
            stack.push(key);
            entered = true;
        });
        if entered { Some(Self { active: true }) } else { None }
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        EXEC_SKILL_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}
