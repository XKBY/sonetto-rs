use sonettobuf::ActEffect;

use crate::state::battle::types::effects::EffectType;

use super::EffectContext;
use super::action::{BuffAction, BuffActCtx, BuffStage};
use super::result::ActionResult;

pub(super) struct Shield;

impl BuffAction for Shield {
    fn execute(
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> ActionResult {
        if stage == BuffStage::BeforeBuffAdd {
            return ActionResult::empty();
        }

        match act_type {
            "Shield" => {
                let permille = parts
                    .get(3)
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                apply(ctx.effect_ctx, permille)
            }
            _ => ActionResult::empty(),
        }
    }
}

/// Buff feature: Shield — grants a shield based on max HP percentage.
pub fn apply(ctx: &mut EffectContext, permille: i32) -> ActionResult {
    let max_hp = ctx
        .target_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let amount = (max_hp as f32 * permille as f32 / 1000.0).ceil() as i32;
    ActionResult::single(ActEffect {
        effect_type: Some(EffectType::Shield as i32),
        target_id: Some(ctx.target_uid()),
        effect_num: Some(amount),
        ..Default::default()
    })
}
