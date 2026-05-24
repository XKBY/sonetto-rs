pub mod behavior;
pub mod behaviour_type;
pub mod condition;
pub mod condition_eval;
pub mod condition_type;
pub mod parser;
pub mod target;

use crate::state::battle::event::Event;
use crate::state::battle::{
    manager::fight_data_mgr::Managers, mechanics::bloodtithe::BloodtitheState,
};
use condition::ConditionOp;
use condition_eval::ConditionEval;
use sonettobuf::Fight;

#[derive(Debug, Clone)]
pub struct EffectSlot {
    pub op: ConditionOp,
    pub conditions: Vec<condition::Condition>,
    pub behaviours: Vec<behavior::Behaviour>,
    pub limit: i32,
    pub round_limit: i32,
    pub use_count: i32,
    pub round_use_count: i32,
}

impl EffectSlot {
    fn matches_hook(&self, hook: condition::Hook) -> bool {
        self.conditions.iter().any(|c| c.hook == hook)
    }

    fn check_conditions(&self, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
        match self.op {
            ConditionOp::And => self.conditions.iter().all(|c| c.check(owner_uid, eval)),
            ConditionOp::Or => self.conditions.iter().any(|c| c.check(owner_uid, eval)),
        }
    }

    fn within_limits(&self) -> bool {
        (self.limit == 0 || self.use_count < self.limit)
            && (self.round_limit == 0 || self.round_use_count < self.round_limit)
    }
}

#[derive(Clone)]
pub struct SkillEffect {
    pub owner_uid: i64,
    pub(crate) slots: Vec<EffectSlot>,
}

impl std::fmt::Debug for SkillEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SkillEffect(owner={}, slots={})",
            self.owner_uid,
            self.slots.len()
        )
    }
}

impl SkillEffect {
    pub fn reset_round_counts(&mut self) {
        for slot in &mut self.slots {
            slot.round_use_count = 0;
        }
    }

    pub(crate) fn fire_hook(
        &mut self,
        hook: condition::Hook,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let eval = ConditionEval {
            fight,
            buff_mgr: &buff_mgr,
            entity_mgr: &entity_mgr,
            bloodtithe: &bloodtithe,
            caster_uid: self.owner_uid,
            target_uid: entity_uid,
            condition_target: 0,
            has_trigger_state: false,
            active_card_cast_uids: None,
        };
        let owner_uid = self.owner_uid;
        let mut matching = Vec::new();
        for (slot_idx, slot) in self.slots.iter_mut().enumerate() {
            if !slot.matches_hook(hook) {
                continue;
            }

            let within_limits = slot.within_limits();
            let cond_eval_pass = slot.check_conditions(owner_uid, eval);

            tracing::info!(
                "effect hooked: hook={:?} owner_uid={} target_uid={} slot_idx={} cond_eval_pass={} within_limits={} behaviours={} use_count={} round_use_count={}",
                hook,
                owner_uid,
                entity_uid,
                slot_idx,
                cond_eval_pass,
                within_limits,
                slot.behaviours.len(),
                slot.use_count,
                slot.round_use_count
            );
            tracing::info!("effect slot detail: slot_idx={} slot={:?}", slot_idx, slot);

            if !within_limits || !cond_eval_pass {
                continue;
            }

            slot.use_count += 1;
            slot.round_use_count += 1;
            matching.extend(slot.behaviours.iter().map(|b| (b.raw.clone(), b.target)));
        }
        matching
            .into_iter()
            .flat_map(|(raw, target)| behavior::execute(fight, managers, owner_uid, &raw, target))
            .collect()
    }
    pub fn fire_all(&mut self, fight: &Fight, managers: &mut Managers, entity_uid: i64) -> Vec<Event> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let eval = ConditionEval {
            fight,
            buff_mgr: &buff_mgr,
            entity_mgr: &entity_mgr,
            bloodtithe: &bloodtithe,
            caster_uid: self.owner_uid,
            target_uid: entity_uid,
            condition_target: 0,
            has_trigger_state: false,
            active_card_cast_uids: None,
        };
        let owner_uid = self.owner_uid;
        let mut matching = Vec::new();
        for slot in self.slots.iter_mut() {
            if !slot.within_limits() || !slot.check_conditions(owner_uid, eval) { continue; }
            slot.use_count += 1;
            slot.round_use_count += 1;
            matching.extend(slot.behaviours.iter().map(|b| (b.raw.clone(), b.target)));
        }
        matching.into_iter()
            .flat_map(|(raw, target)| behavior::execute(fight, managers, owner_uid, &raw, target))
            .collect()
    }

    pub fn on_enter_fight(
        &mut self,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        self.fire_hook(condition::Hook::EnterFight, fight, managers, entity_uid)
    }

    pub fn on_dead(
        &mut self,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        self.fire_hook(condition::Hook::Dead, fight, managers, entity_uid)
    }

    pub fn on_round_start(
        &mut self,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        self.fire_hook(condition::Hook::RoundStart, fight, managers, entity_uid)
    }

    pub fn on_round_end(
        &mut self,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        self.fire_hook(condition::Hook::RoundEnd, fight, managers, entity_uid)
    }

    pub fn on_battle_start(
        &mut self,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        self.fire_hook(condition::Hook::BattleStart, fight, managers, entity_uid)
    }
}
