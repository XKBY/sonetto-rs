//! Markers — buff_acts that emit a single flag ActEffect at apply.
//! Most are presence flags (`effect_num = 0`); `ExPointMaxAdd` carries
//! a payload from `parts[1]`. DOT family + status flags follow the
//! same shape: BuffAdd → matching effect-type marker.

use sonettobuf::ActEffect;

use super::action::{BuffActCtx, BuffAction, BuffStage};
use super::result::ActionResult;
use crate::state::battle::types::effects::EffectType;

pub(super) struct Markers;

impl BuffAction for Markers {
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
        let target = ctx.effect_ctx.target;
        let (effect_type, effect_num) = match act_type {
            "Rebound" => (EffectType::Rebound as i32, 0),
            "AddToTarget" => (EffectType::AddToTarget as i32, 0),
            "MonsterLabel" => (EffectType::MonsterLabelBuff as i32, 0),
            "ExPointOverflowBank" => (EffectType::ExPointOverflowBank as i32, 0),
            "ExPointMaxAdd" => {
                let amount = parts
                    .get(1)
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                (EffectType::ExPointMaxAdd as i32, amount)
            }
            "TeammateInjuryCount" => (EffectType::TeammateInjuryCount as i32, 0),
            "PoisonSettleCanCrit" => (EffectType::PoisonSettleCanCrit as i32, 0),
            "RealHurtFix" => (EffectType::RealHurtFix as i32, 0),
            "RealHarmFix" => (EffectType::RealHarmFix as i32, 0),
            "RealHurtSkillEffectFix" => (EffectType::RealHurtSkillEffectFix as i32, 0),
            "RealHarmSkillEffectFix" => (EffectType::RealHarmSkillEffectFix as i32, 0),
            "Poison" => (EffectType::Poison as i32, 0),
            "LockPoison" => (EffectType::LockDot as i32, 0),
            "DeadlyPoison" => (EffectType::DeadlyPoison as i32, 0),
            // Kakania 30800121's tag for end-of-round damage settlement.
            "InjuryLogback" => (EffectType::InjuryLogBack as i32, 0),
            "Dizzy" => (EffectType::Dizzy as i32, 0),
            "Forbid" => (EffectType::Forbid as i32, 0),
            "ImmunityExpointChange" => (EffectType::ImmunityExPointChange as i32, 0),
            _ => return None,
        };
        Some(ActionResult::single(ActEffect {
            effect_type: Some(effect_type),
            target_id: Some(target),
            effect_num: Some(effect_num),
            ..Default::default()
        }))
    }
}
