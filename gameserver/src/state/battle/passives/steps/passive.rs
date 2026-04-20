use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::{
    context::FightContext, fight_step::FightStepBuilder, passives::collector::CollectedPassives,
    skill::PhaseFilter,
};

use super::skill::execute_skill;

pub fn build_passive_step(
    ctx: &mut FightContext<'_>,
    uids: &[i64],
    collected: &CollectedPassives,
    phase: &PhaseFilter,
) -> Vec<FightStep> {
    let mut steps: Vec<FightStep> = Vec::new();

    for &uid in uids {
        let skill_ids = collected.merged_for(uid);
        tracing::info!("  uid={} passives={:?} phase={:?}", uid, skill_ids, phase);

        let per_entity_effects = execute_skills_for_entity(ctx, uid, skill_ids, phase);

        if !per_entity_effects.is_empty() {
            tracing::info!(
                "  uid={} phase={:?} per_entity_effects count={}",
                uid,
                phase,
                per_entity_effects.len()
            );
            steps.push(
                FightStepBuilder::effect()
                    .with_many(per_entity_effects)
                    .build(),
            );
        }
    }

    steps
}

pub fn build_battle_rule_step(
    ctx: &mut FightContext<'_>,
    uids: &[i64],
    battle_skill_ids: &[i32],
    phase: &PhaseFilter,
) -> Vec<FightStep> {
    let mut steps: Vec<FightStep> = Vec::new();

    for &uid in uids {
        let per_entity_effects = execute_skills_for_entity(ctx, uid, battle_skill_ids.to_vec(), phase);
        if !per_entity_effects.is_empty() {
            steps.push(
                FightStepBuilder::effect()
                    .with_many(per_entity_effects)
                    .build(),
            );
        }
    }

    steps
}

fn execute_skills_for_entity(
    ctx: &mut FightContext<'_>,
    uid: i64,
    skill_ids: Vec<i32>,
    phase: &PhaseFilter,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();

    for skill_id in skill_ids {
        eprintln!(
            "[DBG][PASSIVE-EXEC] uid={} phase={:?} skill={} enter",
            uid, phase, skill_id
        );
        match execute_skill(ctx, uid, uid, skill_id, phase) {
            Ok(skill_effects) => {
                eprintln!(
                    "[DBG][PASSIVE-EXEC] uid={} phase={:?} skill={} exit effects={}",
                    uid,
                    phase,
                    skill_id,
                    skill_effects.len()
                );
                effects.extend(skill_effects)
            }
            Err(e) => tracing::warn!("passive {} uid {} phase {:?}: {}", skill_id, uid, phase, e),
        }
    }

    effects
}
