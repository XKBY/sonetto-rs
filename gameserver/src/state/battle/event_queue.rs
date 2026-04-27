#![allow(dead_code)]

use sonettobuf::{ActEffect, Fight, FightHurtInfo as HurtInfo, effect_type_enum::EffectType};

use crate::state::battle::{
    fight_step::ActEffectBuilder,
    manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr},
};

/// A typed event recording a state mutation or visual emission
/// that should land in the FightStep stream.
#[derive(Debug, Clone)]
pub enum BattleEvent {
    BuffApply {
        target: i64,
        buff_id: i32,
        count: i32,
        layer: i32,
        from: i64,
        config_effect: Option<i32>,
    },
    BuffUpdate {
        target: i64,
        buff_uid: i32,
        new_count: i32,
        new_layer: i32,
    },
    BuffRemove {
        target: i64,
        buff_uid: i32,
    },
    Damage {
        target: i64,
        amount: i32,
        hurt_info: HurtInfo,
        from: i64,
        skill_id: Option<i32>,
    },
    Heal {
        target: i64,
        amount: i32,
        from: i64,
    },
    ExPointChange {
        target: i64,
        delta: i32,
    },
    PowerChange {
        delta: i32,
    },
    BloodpoolValueChange {
        team_type: i32,
        target: i64,
        delta: i32,
    },
    BloodpoolMaxChange {
        team_type: i32,
        max: i32,
    },
    SkillEmit {
        skill_id: i32,
        from: i64,
        to: i64,
        children: Vec<BattleEvent>,
        kind: SkillEmitKind,
    },
    EffectMarker {
        effect_type: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEmitKind {
    Active,
    Passive,
    TriggerReactive,
    PsychubeRider,
    BossWrapper,
}

#[derive(Debug, Default)]
pub struct EventQueue {
    events: Vec<BattleEvent>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn push(&mut self, event: BattleEvent) {
        self.events.push(event);
    }

    pub fn drain(&mut self) -> Vec<BattleEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Owns mut-borrows for the drain step.
pub struct EventContext<'a> {
    pub fight: &'a mut Fight,
    pub buff_mgr: &'a mut BuffMgr,
    pub ex_point_mgr: &'a mut ExPointMgr,
}

/// Serialize a queue to FightStep ActEffects. Phase 1 only: this
/// is a stub that returns an empty Vec. Phase 2 migrations will
/// fill it in incrementally as each migrated leaf adds its own
/// case.
pub fn drain_to_fight_steps(
    events: Vec<BattleEvent>,
    _ctx: &mut EventContext<'_>,
) -> Vec<ActEffect> {
    let mut out = Vec::with_capacity(events.len());

    for event in events {
        match event {
            BattleEvent::PowerChange { delta } => out.push(ActEffect {
                effect_type: Some(EffectType::Powerchange as i32),
                effect_num: Some(delta),
                ..Default::default()
            }),
            BattleEvent::BloodpoolValueChange {
                team_type,
                target,
                delta,
            } => out.push(ActEffectBuilder::bloodpool_value_change(
                target, team_type, delta,
            )),
            BattleEvent::BloodpoolMaxChange { team_type, max } => {
                out.push(ActEffectBuilder::bloodpool_max_change(team_type, max));
            }
            _ => {}
        }
    }

    out
}

pub fn serialize_leaf_event(event: BattleEvent) -> ActEffect {
    let mut queue = EventQueue::new();
    queue.push(event);

    let mut fight = Fight::default();
    let mut buff_mgr = BuffMgr::new();
    let mut ex_point_mgr = ExPointMgr::new();
    let mut ctx = EventContext {
        fight: &mut fight,
        buff_mgr: &mut buff_mgr,
        ex_point_mgr: &mut ex_point_mgr,
    };

    drain_to_fight_steps(queue.drain(), &mut ctx)
        .into_iter()
        .next()
        .expect("leaf event should serialize to a single ActEffect")
}
