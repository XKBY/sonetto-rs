//! Markers buff_action — handler for the buff_act types that emit a
//! single self-contained marker ActEffect on the target with no
//! supporting math beyond the effect-type/value mapping.
//!
//! Most of these are LIVE protocol "flag" emissions that the engine
//! reads back later to gate downstream behavior (e.g. `RealHurtFix`
//! tells the damage pipeline to skip the rate fix; `MonsterLabelBuff`
//! flags the target as a monster for AOE filters). A few carry a
//! payload value (`ExPointMaxAdd` reads `parts[1]`); the rest emit
//! `effect_num = 0` as a presence flag.
//!
//! Variants owned:
//! * `Rebound`, `AddToTarget`, `MonsterLabel`, `ExPointOverflowBank`,
//!   `ExPointMaxAdd`, `TeammateInjuryCount`, `PoisonSettleCanCrit`,
//!   `RealHurtFix`, `RealHarmFix`, `RealHurtSkillEffectFix`,
//!   `RealHarmSkillEffectFix`.
//! * **DOT family** — `Poison`, `LockPoison`, `DeadlyPoison`. Each
//!   emits the matching effect-type marker (`Poison(213)`,
//!   `LockDot(216)`, `DeadlyPoison(255)`) alongside its `BuffAdd`.
//!   The actual DOT damage tick fires later via the buff settlement
//!   phase; at apply time the marker is the only emission. Verified
//!   from LIVE battle3 r2-r10: every BuffAdd of a Poison-family buff
//!   (e.g. 31040005, 30980111, 30091129) is paired with one of these
//!   effect-type markers in the same actEffect array.

use sonettobuf::ActEffect;

use super::action::{BuffAction, BuffActCtx, BuffStage};
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
