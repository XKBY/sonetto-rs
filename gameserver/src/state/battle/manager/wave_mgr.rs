//! Wave-spawn manager. Owns wave-state mutation and exposes
//! advance_wave plus replay-mode fast-forward helpers.
//!
//! In replay mode, captured boss steps may reference entity uids
//! from wave N+1 before our state has spawned them (because LIVE
//! kills wave N faster than our damage simulation does). The
//! expected_wave_for_uid + fast_forward_to_wave pair lets
//! card_mgr::execute_ai_turn lazy-spawn future waves on demand.

use anyhow::Result;
use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::{
    buff_actions::{EffectContext, apply_after_buff_add_features},
    context::FightContext,
    fight::defender::Defender,
    fight_step::FightStepBuilder,
    manager::buff_mgr::observe_explicit_buff_uid_for_target,
    skill::SkillExecutor,
    types::effects::EffectType,
    utils::buff_add,
};

#[derive(Debug, Clone, Default)]
pub struct WaveMgr {}

impl WaveMgr {
    pub fn new() -> Self {
        Self::default()
    }

    /// UID assignment formula: uid = -((2 * (wave - 1)) + position),
    /// position in {1, 2}. So abs(uid)={1,2} -> wave 1; {3,4} -> wave 2; etc.
    /// Returns None for non-defender uids (uid >= 0).
    pub fn expected_wave_for_uid(uid: i64) -> Option<i32> {
        if uid >= 0 {
            return None;
        }
        let abs = uid.unsigned_abs() as i32;
        Some((abs + 1) / 2)
    }

    /// Advance to the next wave. Existing behavior, moved from the
    /// free-function advance_wave.
    pub fn advance_wave(
        &mut self,
        ctx: &mut FightContext<'_>,
        executor: &mut SkillExecutor,
    ) -> Result<Vec<FightStep>> {
        let current_wave = ctx.fight.cur_wave.unwrap_or(1);
        let new_wave = current_wave + 1;
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        let new_entities = Defender::build_wave_entities(battle_id, new_wave, 2)?;

        let defender = ctx
            .fight
            .defender
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Fight missing defender team"))?;
        defender.entitys = new_entities;
        defender.sub_entitys.clear();

        ctx.fight.cur_wave = Some(new_wave);
        ctx.fight.is_finish = Some(false);

        let fight = ctx.fight.clone();
        let mut steps = vec![
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::NewChangeWave as i32),
                    effect_num: Some(0),
                    fight: Some(fight.clone()),
                    ..Default::default()
                })
                .build(),
        ];

        if let Some(step) = build_active_circle_enemy_buff_step(&fight, ctx, executor) {
            steps.push(step);
        }

        Ok(steps)
    }

    /// Replay-mode fast-forward: advance waves until current_wave >= target.
    /// Concatenates wave-spawn steps from each advance.
    pub fn fast_forward_to_wave(
        &mut self,
        ctx: &mut FightContext<'_>,
        executor: &mut SkillExecutor,
        target_wave: i32,
    ) -> Result<Vec<FightStep>> {
        let mut steps = Vec::new();
        loop {
            let current = ctx.fight.cur_wave.unwrap_or(1);
            if current >= target_wave {
                break;
            }
            steps.extend(self.advance_wave(ctx, executor)?);
        }
        Ok(steps)
    }
}

fn build_active_circle_enemy_buff_step(
    fight: &Fight,
    ctx: &mut FightContext<'_>,
    executor: &mut SkillExecutor,
) -> Option<FightStep> {
    let circle = fight.magic_circle.as_ref()?;
    let circle_id = circle.magic_circle_id.unwrap_or(0);
    let circle_round = circle.round.unwrap_or(0);
    if circle_id == 0 || circle_round <= 0 {
        return None;
    }

    let circle_cfg = config::configs::get().magic_circle.get(circle_id)?;
    let buff_id = circle_cfg.enemy_buff.trim().parse::<i32>().ok()?.max(0);
    if buff_id == 0 {
        return None;
    }

    let caster_uid = circle.create_uid.unwrap_or(0);
    if caster_uid == 0 {
        return None;
    }

    let target_uids: Vec<i64> = fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.entitys.iter())
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter_map(|entity| entity.uid)
        .filter(|uid| *uid != 0)
        .collect();

    if target_uids.is_empty() {
        return None;
    }

    let mut effects = Vec::new();
    for target_uid in target_uids {
        let mut effect_ctx =
            EffectContext::new(fight, ctx.managers, ctx.mechanics, caster_uid, target_uid);
        let effect = buff_add(target_uid, caster_uid, buff_id, 1);
        if let Some(buff_uid) = effect.buff.as_ref().and_then(|buff| buff.uid) {
            observe_explicit_buff_uid_for_target(target_uid, buff_uid);
            effect_ctx
                .buff_mgr_mut()
                .add_with_uid(target_uid, buff_id, caster_uid, 0, 1, buff_uid);
        }
        effects.push(effect);
        effects.extend(apply_after_buff_add_features(
            &mut effect_ctx,
            executor,
            buff_id,
            false,
        ));
        effects.extend(executor.side_effects.drain(..));
    }

    if effects.is_empty() {
        None
    } else {
        Some(FightStepBuilder::effect().with_many(effects).build())
    }
}
