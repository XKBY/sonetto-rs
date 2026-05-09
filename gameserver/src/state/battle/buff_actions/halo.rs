//! `MasterHalo` / `SlaveHalo` — applies a slave buff to every ally.
//! Master fans the slave buff out (state mutation in `execute`) and
//! emits a master_halo marker plus a side-effect FightStep wrapping
//! all the per-ally BuffAdd / slave_halo / feature emissions.
//! `SlaveHalo` itself is bookkeeping (the master side does the work).

use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    fight_step::ActEffectBuilder, manager::buff_mgr::observe_explicit_buff_uid_for_target,
};

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;
use super::{apply_after_buff_add_features, apply_before_buff_add_features};

pub(super) struct MasterHaloParams {
    pub slave_buff_id: i32,
    pub caster_uid: i64,
    pub target_uid: i64,
    pub ally_uids: Vec<i64>,
    /// Filled in by `execute`. Holds every per-ally emission collected
    /// during the slave buff application loop.
    pub slave_effects: Vec<ActEffect>,
}

pub(super) struct MasterHaloHandler;

impl BuffActionHandler for MasterHaloHandler {
    type Params = MasterHaloParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "MasterHalo" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        let slave_buff_id = parts
            .get(2)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let caster_uid = ctx.effect_ctx.caster_uid();
        let target_uid = ctx.effect_ctx.target_uid();
        let ally_uids = ctx
            .executor
            .get_ally_uids(ctx.effect_ctx.fight(), caster_uid);
        Self::Params {
            slave_buff_id,
            caster_uid,
            target_uid,
            ally_uids: ally_uids.clone(),
            slave_effects: Vec::with_capacity(ally_uids.len() * 4),
        }
    }

    fn execute(&self, params: &mut Self::Params, ctx: &mut BuffActCtx<'_, '_>) {
        let original_target = params.target_uid;
        for &ally_uid in &params.ally_uids {
            ctx.effect_ctx.target = ally_uid;

            // Pre-stage features (e.g. HP snapshots for Attr children).
            let before = apply_before_buff_add_features(
                ctx.effect_ctx,
                ctx.executor,
                params.slave_buff_id,
                0,
            );
            params.slave_effects.extend(before);

            let buff_effect = crate::state::battle::fight_step::ActEffectBuilder::buff_add_slave(
                ally_uid,
                params.caster_uid,
                params.slave_buff_id,
                0,
            );
            if let Some(buff_uid) = buff_effect.buff.as_ref().and_then(|b| b.uid) {
                observe_explicit_buff_uid_for_target(ally_uid, buff_uid);
                ctx.effect_ctx.buff_mgr_mut().add_with_uid(
                    ally_uid,
                    params.slave_buff_id,
                    params.caster_uid,
                    0,
                    1,
                    buff_uid,
                );
            }
            params.slave_effects.push(buff_effect);
            params
                .slave_effects
                .push(ActEffectBuilder::slave_halo(ally_uid));

            let after = apply_after_buff_add_features(
                ctx.effect_ctx,
                ctx.executor,
                params.slave_buff_id,
                false,
            );
            params.slave_effects.extend(after);
        }
        ctx.effect_ctx.target = original_target;
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        let slave_step = ActEffectBuilder::skill_wrapper_without_num(FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            from_id: Some(params.caster_uid),
            to_id: Some(0),
            act_id: Some(0),
            act_effect: params.slave_effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        });
        ActionResult {
            effects: vec![ActEffectBuilder::master_halo(params.target_uid)],
            side_effects: vec![slave_step],
            ..Default::default()
        }
    }
}

pub(super) struct SlaveHaloHandler;

impl BuffActionHandler for SlaveHaloHandler {
    type Params = ();

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "SlaveHalo" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {}

    fn steps(&self, _params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::empty()
    }
}
