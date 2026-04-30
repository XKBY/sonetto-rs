//! Markers — buff_acts that emit a single flag ActEffect at apply.
//! Most are presence flags (`effect_num = 0`); `ExPointMaxAdd` carries
//! a payload from `parts[1]`. DOT family + status flags follow the
//! same shape: BuffAdd → matching effect-type marker.

use sonettobuf::ActEffect;

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;
use crate::state::battle::types::effects::EffectType;

pub(super) struct MarkerParams {
    pub target_uid: i64,
    pub effect_type: i32,
    pub effect_num: i32,
}

pub(super) struct MarkerHandler;

fn marker_effect_type(act_type: &str) -> Option<i32> {
    Some(match act_type {
        "Rebound" => EffectType::Rebound as i32,
        "AddToTarget" => EffectType::AddToTarget as i32,
        "MonsterLabel" => EffectType::MonsterLabelBuff as i32,
        "ExPointOverflowBank" => EffectType::ExPointOverflowBank as i32,
        "ExPointMaxAdd" => EffectType::ExPointMaxAdd as i32,
        "TeammateInjuryCount" => EffectType::TeammateInjuryCount as i32,
        "PoisonSettleCanCrit" => EffectType::PoisonSettleCanCrit as i32,
        "RealHurtFix" => EffectType::RealHurtFix as i32,
        "RealHarmFix" => EffectType::RealHarmFix as i32,
        "RealHurtSkillEffectFix" => EffectType::RealHurtSkillEffectFix as i32,
        "RealHarmSkillEffectFix" => EffectType::RealHarmSkillEffectFix as i32,
        "Poison" => EffectType::Poison as i32,
        "LockPoison" => EffectType::LockDot as i32,
        "DeadlyPoison" => EffectType::DeadlyPoison as i32,
        "InjuryLogback" => EffectType::InjuryLogBack as i32,
        "Dizzy" => EffectType::Dizzy as i32,
        "Forbid" => EffectType::Forbid as i32,
        "ImmunityExpointChange" => EffectType::ImmunityExPointChange as i32,
        _ => return None,
    })
}

fn parse_part(parts: &[&str], idx: usize) -> i32 {
    parts
        .get(idx)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn marker_params(parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> MarkerParams {
    let act_id = parse_part(parts, 0);
    let act_type = config::configs::get()
        .buff_act
        .iter()
        .find(|row| row.id == act_id)
        .map(|row| row.r#type.as_str())
        .unwrap_or("");
    let effect_num = if act_type == "ExPointMaxAdd" {
        parse_part(parts, 1)
    } else {
        0
    };
    MarkerParams {
        target_uid: ctx.effect_ctx.target,
        effect_type: marker_effect_type(act_type).unwrap_or(0),
        effect_num,
    }
}

impl BuffActionHandler for MarkerHandler {
    type Params = MarkerParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        stage == BuffStage::AfterBuffAdd && marker_effect_type(act_type).is_some()
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        marker_params(parts, ctx)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::single(ActEffect {
            effect_type: Some(params.effect_type),
            target_id: Some(params.target_uid),
            effect_num: Some(params.effect_num),
            ..Default::default()
        })
    }
}
