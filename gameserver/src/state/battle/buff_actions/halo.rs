use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    manager::buff_mgr::observe_explicit_buff_uid_for_target,
    skill::SkillExecutor,
    types::effects::EffectType,
    utils::{buff_add_slave, master_halo, slave_halo},
};

use super::EffectContext;
use super::action::{BuffAction, BuffActCtx, BuffStage};
use super::result::ActionResult;

/// Halo buff_action — handles `MasterHalo` (active) and `SlaveHalo`
/// (passive bookkeeping; the engine emits the actual halo effects
/// from the master side).
///
/// The master side calls into `master(ctx, executor, slave_buff_id)`
/// where `slave_buff_id` is taken from `parts[2]`.
pub(super) struct Halo;

impl BuffAction for Halo {
    fn execute(
        &self,
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult> {
        if stage == BuffStage::BeforeBuffAdd {
            return None;
        }
        match act_type {
            "MasterHalo" => {
                let slave_buff_id = parts
                    .get(2)
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                Some(master(ctx.effect_ctx, ctx.executor, slave_buff_id))
            }
            "SlaveHalo" => Some(ActionResult::empty()),
            _ => None,
        }
    }
}

/// Buff feature: MasterHalo — applies a slave buff to all allies as a side effect.
/// Needs executor to queue the slave halo step.
pub fn master(
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    slave_buff_id: i32,
) -> ActionResult {
    let ally_uids = executor.get_ally_uids(ctx.fight(), ctx.caster_uid());
    let original_target = ctx.target_uid();
    let mut slave_effects: Vec<ActEffect> = Vec::new();

    for ally_uid in ally_uids {
        ctx.target = ally_uid;
        let caster_uid = ctx.caster_uid();

        // Emit "before BUFFADD" side effects for this slave buff on this ally
        // (e.g. HP snapshots for Attr/EachChangeAttr style features).
        let before_add_effects =
            super::apply_before_buff_add_features(ctx, executor, slave_buff_id, 0);
        slave_effects.extend(before_add_effects);

        let effect = buff_add_slave(ally_uid, ctx.caster_uid(), slave_buff_id, 0);
        if let Some(buff_uid) = effect.buff.as_ref().and_then(|b| b.uid) {
            observe_explicit_buff_uid_for_target(ally_uid, buff_uid);
            ctx.buff_mgr_mut()
                .add_with_uid(ally_uid, slave_buff_id, caster_uid, 0, 0, buff_uid);
        }
        slave_effects.push(effect);
        slave_effects.push(slave_halo(ally_uid));

        let after_add_effects =
            super::apply_after_buff_add_features(ctx, executor, slave_buff_id, false);
        slave_effects.extend(after_add_effects);
    }

    ctx.target = original_target;

    let slave_step = ActEffect {
        effect_type: Some(EffectType::FightStep as i32),
        target_id: Some(0),
        fight_step: Some(FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            from_id: Some(ctx.caster_uid()),
            to_id: Some(0),
            act_id: Some(0),
            act_effect: slave_effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }),
        ..Default::default()
    };

    ActionResult {
        effects: vec![master_halo(ctx.target_uid())],
        side_effects: vec![slave_step],
        ..Default::default()
    }
}
