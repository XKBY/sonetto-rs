use sonettobuf::ActEffect;

use crate::state::battle::{
    skill::damage::{calculate_heal, calculate_heal_by_two_attr},
    types::effects::EffectType,
};

use super::EffectContext;
use super::action::{BuffAction, BuffActCtx, BuffStage};
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
pub(super) struct Healing;

impl BuffAction for Healing {
    fn execute(
        &self,
        act_type: &str,
        _parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult> {
        if stage == BuffStage::BeforeBuffAdd {
            return None;
        }
        match act_type {
            "CureUpByLostHp" => Some(cure_up_by_lost_hp(ctx.effect_ctx)),
            "Revive" => Some(revive(ctx.effect_ctx)),
            _ => None,
        }
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

/// Buff feature: CureUpByLostHp — emit CureUpByLostHp(347) notification.
pub fn cure_up_by_lost_hp(ctx: &mut EffectContext) -> ActionResult {
    ActionResult::single(ActEffect {
        effect_type: Some(EffectType::CureUpByLostHp as i32),
        target_id: Some(ctx.target_uid()),
        effect_num: Some(0),
        ..Default::default()
    })
}

/// Buff feature: Revive — emit Cure(4) placeholder.
pub fn revive(ctx: &mut EffectContext) -> ActionResult {
    ActionResult::single(ActEffect {
        effect_type: Some(EffectType::Cure as i32),
        target_id: Some(ctx.target_uid()),
        effect_num: Some(0),
        ..Default::default()
    })
}
