use sonettobuf::ActEffect;

use crate::state::battle::{types::effects::EffectType, utils::effect_none};

use super::EffectContext;
use super::action::{BuffAction, BuffActCtx, BuffStage};
use super::result::ActionResult;

/// HP buff_action — handles `LostHpCountAddBuff`, the only buff_act
/// type with HP-broadcast logic at apply time.
///
/// Two-stage emission: the BeforeBuffAdd pass emits the
/// `MaxHpChange(108)` / `CurrentHpChange(109)` pairs (so the UI sees
/// the new max before the BuffAdd lands), and the AfterBuffAdd pass
/// emits only the trailing `None(0)` marker for the dispatcher's
/// feature loop. Both stages call into `lost_hp_count_add_buff`
/// for the underlying skip-broadcast / max-hp computation; the
/// stage just decides which subset of the resulting effects to keep.
pub(super) struct Hp;

impl BuffAction for Hp {
    fn execute(
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> ActionResult {
        if act_type != "LostHpCountAddBuff" {
            return ActionResult::empty();
        }
        let child_buff_id = parts
            .get(1)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let mut result = lost_hp_count_add_buff(ctx.effect_ctx, child_buff_id);
        match stage {
            BuffStage::BeforeBuffAdd => {
                result
                    .effects
                    .retain(|e| matches!(e.effect_type, Some(108) | Some(109)));
            }
            BuffStage::AfterBuffAdd => {
                result
                    .effects
                    .retain(|e| e.effect_type == Some(EffectType::None as i32));
            }
        }
        result
    }
}

/// Buff feature: LostHpCountAddBuff — broadcasts HP state on buff application.
/// If child buff's attr is in the exclusion list, emits None instead.
pub fn lost_hp_count_add_buff(ctx: &mut EffectContext, _child_buff_id: i32) -> ActionResult {
    // Match legacy/live behavior: certain child-buff Attr targets should NOT trigger
    // MaxHp/CurrentHp broadcast pairs (only emit trailing None).
    const NO_BROADCAST_ATTRS: &[i32] = &[203, 211];

    if _child_buff_id > 0 {
        let cfg = config::configs::get();
        let skip_broadcast = cfg
            .skill_buff
            .iter()
            .find(|b| b.id == _child_buff_id)
            .map(|child| {
                child.features.split('|').any(|entry| {
                    let parts: Vec<&str> = entry.split('#').collect();
                    let act_id: i32 = parts
                        .first()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let is_attr = cfg
                        .buff_act
                        .iter()
                        .find(|a| a.id == act_id)
                        .map(|a| a.r#type == "Attr")
                        .unwrap_or(false);
                    if !is_attr {
                        return false;
                    }
                    let char_attr_id: i32 = parts
                        .get(1)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    NO_BROADCAST_ATTRS.contains(&char_attr_id)
                })
            })
            .unwrap_or(false);

        if skip_broadcast {
            return ActionResult::single(effect_none(ctx.target_uid()));
        }
    }

    let max_hp = ctx
        .target_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let current_hp = ctx.target_hp();

    let mut effects = vec![effect_none(ctx.target_uid())];
    for _ in 0..2 {
        effects.push(ActEffect {
            effect_type: Some(EffectType::MaxHpChange as i32),
            target_id: Some(ctx.target_uid()),
            effect_num: Some(max_hp),
            ..Default::default()
        });
        effects.push(ActEffect {
            effect_type: Some(EffectType::CurrentHpChange as i32),
            target_id: Some(ctx.target_uid()),
            effect_num: Some(current_hp),
            ..Default::default()
        });
    }
    ActionResult::effects(effects)
}
