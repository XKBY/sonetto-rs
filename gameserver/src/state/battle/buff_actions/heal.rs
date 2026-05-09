use sonettobuf::ActEffect;

use crate::state::battle::{
    fight_step::ActEffectBuilder,
    skill::damage::{calculate_heal, calculate_heal_by_two_attr},
};

use super::EffectContext;
use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

/// Healing buff_action — handles the buff_act types that emit a
/// healing-style ActEffect when the buff is applied:
///
/// * `CureUpByLostHp` — emit `CureUpByLostHp(347)` notification
/// * `Revive` — emit `Cure(4)` placeholder
///
/// Bare `Cure` is intentionally unhandled here — the dispatcher
/// falls through to its default no-op for that buff_act type because
/// LIVE doesn't emit a per-feature effect for it (the cure happens
/// at skill emission time via the heal helpers below).
pub(super) struct HealParams {
    pub target_uid: i64,
}

pub(super) struct CureUpByLostHpHandler;

impl BuffActionHandler for CureUpByLostHpHandler {
    type Params = HealParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "CureUpByLostHp" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        HealParams {
            target_uid: ctx.effect_ctx.target_uid(),
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        cure_up_by_lost_hp(params.target_uid)
    }
}

pub(super) struct ReviveHandler;

impl BuffActionHandler for ReviveHandler {
    type Params = HealParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Revive" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        HealParams {
            target_uid: ctx.effect_ctx.target_uid(),
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        revive(params.target_uid)
    }
}

/// Skill behavior: Heal — fixed rate heal from caster ATK.
pub fn heal(ctx: &mut EffectContext, rate: i32) -> Vec<ActEffect> {
    calculate_heal(ctx.fight(), ctx.caster_uid(), ctx.target_uid(), rate, false)
        .map(|e| vec![e])
        .unwrap_or_default()
}

/// Skill behavior: HealByTwoAttr — heals based on target missing HP + caster max HP.
pub fn heal_by_two_attr(
    ctx: &mut EffectContext,
    missing_percent: i32,
    caster_hp_percent: i32,
) -> Vec<ActEffect> {
    calculate_heal_by_two_attr(
        ctx.fight(),
        ctx.caster_uid(),
        ctx.target_uid(),
        missing_percent,
        caster_hp_percent,
    )
}

pub fn cure_up_by_lost_hp(target_uid: i64) -> ActionResult {
    ActionResult::single(ActEffectBuilder::cure_up_by_lost_hp(target_uid))
}

pub fn revive(target_uid: i64) -> ActionResult {
    ActionResult::single(ActEffectBuilder::cure(target_uid))
}
