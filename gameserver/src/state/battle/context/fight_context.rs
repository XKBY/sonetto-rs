use crate::state::battle::event::Event;
use crate::state::battle::manager::{fight_data_mgr::Managers, traits::Manager};
use crate::state::battle::mechanics::Mechanics;
use super::hook_call;
use crate::state::battle::skill::{PhaseFilter, TriggerState};

use rand::rngs::StdRng;
use sonettobuf::Fight;
use std::{collections::HashSet, ptr::NonNull};

#[derive(Copy, Clone)]
struct RngPtr(NonNull<StdRng>);

// SAFETY: this is a non-owning pointer to the round-local RNG. Access still requires
// `&mut FightContext`, so callers maintain exclusivity when dereferencing it.
unsafe impl Send for RngPtr {}

pub struct FightContext<'a> {
    pub fight: &'a mut Fight,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    pub active_card_cast_uids: HashSet<i64>,
    rng: Option<RngPtr>,
}

impl<'a> FightContext<'a> {
    pub fn new(
        fight: &'a mut Fight,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
    ) -> Self {
        Self {
            fight,
            managers,
            mechanics,
            active_card_cast_uids: HashSet::new(),
            rng: None,
        }
    }

    pub fn with_rng(mut self, rng: &mut StdRng) -> Self {
        self.rng = Some(RngPtr(NonNull::from(rng)));
        self
    }

    pub fn rng_ptr(&self) -> Option<NonNull<StdRng>> {
        self.rng.map(|ptr| ptr.0)
    }

    pub fn sync(&mut self) {
        self.managers.entity_mgr.rebuild_cache(self.fight);
        self.managers.calculate_mgr.update_cache(self.fight);
    }

    pub fn clear_round_active_card_casts(&mut self) {
        self.active_card_cast_uids.clear();
    }

    pub fn mark_round_active_card_cast(&mut self, uid: i64) {
        if uid != 0 {
            self.active_card_cast_uids.insert(uid);
        }
    }

    pub fn set_round_active_card_cast_uids(&mut self, uids: &HashSet<i64>) {
        self.active_card_cast_uids = uids.clone();
    }

    pub fn combat_trigger_state(&self) -> TriggerState {
        TriggerState::default().with_round_active_card_cast_uids(&self.active_card_cast_uids)
    }

    pub fn combat_phase(&self) -> PhaseFilter {
        PhaseFilter::combat_with(self.combat_trigger_state())
    }

    pub fn active_use_trigger_state(&self, skill_id: i32) -> TriggerState {
        TriggerState::on_active_use_skill(skill_id)
            .with_round_active_card_cast_uids(&self.active_card_cast_uids)
    }

    pub fn on_round_end(&mut self) {
        self.managers.entity_mgr.on_round_end(self.fight);
        self.clear_round_active_card_casts();
        self.sync();
        hook_call::on_round_end(self.managers, self.fight);
    }

    pub fn on_use_card(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardPlayed { card, .. } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let events = vec![Event::ExPointChange { target: uid, delta: 1, emit_step: true }];
        hook_call::on_use_card(self.managers, self.fight, events)
    }

    pub fn on_move_card(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardMoved { card } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let events = vec![Event::ExPointChange { target: uid, delta: 1, emit_step: true }];
        hook_call::on_move_card(self.managers, self.fight, events)
    }

    pub fn on_compose_card(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardComposed { card } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let events = vec![Event::ExPointChange { target: uid, delta: 1, emit_step: true }];
        hook_call::on_compose_card(self.managers, self.fight, events)
    }

    pub fn on_enter_fight(&mut self, entity_uid: i64) -> Vec<Event> {
        hook_call::on_enter_fight(self.managers, self.fight, entity_uid)
    }

    pub fn on_dead(&mut self, entity_uid: i64) -> Vec<Event> {
        hook_call::on_dead(self.managers, self.fight, entity_uid)
    }
}
