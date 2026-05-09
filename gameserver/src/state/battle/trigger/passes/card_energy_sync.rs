use sonettobuf::{Fight, FightStep};

use crate::state::battle::{
    context::FightContext,
    fight_step::{ActEffectBuilder, FightStepBuilder},
    passives::collector::CollectedPassives,
    trigger::combat::TriggerEvent,
    types::ex_point::ExPointType,
};

use super::TriggerPass;

pub struct CardEnergySyncPass;

pub fn build_belief_gain_step(fight: &Fight, team_type: i32, gain: i32) -> Option<FightStep> {
    if gain <= 0 {
        return None;
    }

    let side = match team_type {
        1 => fight.attacker.as_ref(),
        2 => fight.defender.as_ref(),
        _ => None,
    }?;

    let mut effects = Vec::new();
    for entity in side.entitys.iter().chain(side.sub_entitys.iter()) {
        if entity.current_hp.unwrap_or(0) <= 0 {
            continue;
        }
        let Some(uid) = entity.uid else { continue };
        let is_belief = entity
            .ex_point_type
            .and_then(ExPointType::from_i32)
            .map(|t| t == ExPointType::Belief)
            .unwrap_or(false);
        if !is_belief {
            continue;
        }
        for _ in 0..gain {
            effects.push(ActEffectBuilder::moxie_change(uid, 1));
        }
    }

    if effects.is_empty() {
        return None;
    }

    Some(FightStepBuilder::effect().with_many(effects).build())
}

impl TriggerPass for CardEnergySyncPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        _collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        let mut out = Vec::new();
        for &(team_type, gain) in &event.bloodpool_gain_packets_by_team {
            if let Some(step) = build_belief_gain_step(ctx.fight, team_type, gain) {
                out.push(step);
            }
        }
        out
    }
}
