pub mod apply;

use sonettobuf::{ActEffect, BeginRoundOper, CardInfo, FightHurtInfo as HurtInfo};
use crate::state::battle::{event_queue::SkillEmitKind, fight_step::ActEffectBuilder};

#[derive(Debug, Clone)]
pub enum Event {
    BuffApply {
        target: i64, buff_id: i32, count: i32, layer: i32,
        from: i64, from_skill_id: i32, config_effect: Option<i32>,
    },
    BuffUpdate { target: i64, buff_uid: i64, new_count: i32, new_layer: i32 },
    BuffRemove { target: i64, buff_uid: i64 },
    BuffSyncAddWithUid {
        target: i64, buff_id: i32, from: i64, from_skill_id: i32,
        count: i32, layer: i32, buff_uid: i64,
    },
    BuffSyncAddWithUidAndEmitUpdate {
        target: i64, buff_id: i32, from: i64, from_skill_id: i32,
        sync_count: i32, sync_layer: i32, buff_uid: i64,
        emit_count: i32, emit_layer: i32,
    },
    Damage {
        target: i64, amount: i32, is_crit: bool,
        hurt_info: HurtInfo, from: i64, skill_id: Option<i32>,
    },
    Heal     { target: i64, amount: i32, from: i64 },
    HealCrit { target: i64, amount: i32, from: i64 },
    ExPointChange { target: i64, delta: i32, emit_step: bool },
    PowerChange   { delta: i32 },
    BloodpoolValueChange { team_type: i32, target: i64, delta: i32 },
    BloodpoolMaxChange   { team_type: i32, max: i32 },
    SerializedActEffect  { effect: ActEffect },
    SkillEmit {
        skill_id: i32, from: i64, to: i64,
        children: Vec<Event>, kind: SkillEmitKind,
    },
    EffectMarker { effect_type: i32 },

    CardPlayed           { card: CardInfo, oper: BeginRoundOper },
    CardMoved            { card: CardInfo },
    CardComposed         { card: CardInfo },
    SimulateDissolveCard { oper: BeginRoundOper },
    EnterBattle          { entity_uid: i64 },
    Dead                 { entity_uid: i64 },
    RemoveEntityCards    { entity_uid: i64 },
}

pub fn events_to_act_effects(events: Vec<Event>) -> Vec<ActEffect> {
    events.into_iter().filter_map(|e| match e {
        Event::Dead { entity_uid } => Some(ActEffectBuilder::dead(entity_uid)),
        Event::RemoveEntityCards { entity_uid } => Some(ActEffectBuilder::remove_entity_cards(entity_uid, Some(1))),
        _ => None,
    }).collect()
}
