//! `LostHpCountAddBuff` — emits HP-broadcast pairs pre-stage and a
//! None marker post-stage. Migrated to the `BuffActionHandler` trait
//! (one handler per stage). Child buffs whose primary `Attr` targets
//! 203 or 211 skip the broadcast pairs.

use sonettobuf::ActEffect;

use crate::state::battle::types::effects::EffectType;

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

const NO_BROADCAST_ATTRS: &[i32] = &[203, 211];

pub(super) struct LostHpCountAddBuffParams {
    pub target_uid: i64,
    pub max_hp: i32,
    pub current_hp: i32,
    pub skip_broadcast: bool,
}

fn parse_child_buff_id(parts: &[&str]) -> i32 {
    parts
        .get(1)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn child_buff_skips_broadcast(child_buff_id: i32) -> bool {
    if child_buff_id <= 0 {
        return false;
    }
    let cfg = config::configs::get();
    let Some(child) = cfg.skill_buff.iter().find(|b| b.id == child_buff_id) else {
        return false;
    };
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
}

fn snapshot_params(parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> LostHpCountAddBuffParams {
    let child_buff_id = parse_child_buff_id(parts);
    LostHpCountAddBuffParams {
        target_uid: ctx.effect_ctx.target_uid(),
        max_hp: ctx
            .effect_ctx
            .target_entity()
            .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
            .unwrap_or(0),
        current_hp: ctx.effect_ctx.target_hp(),
        skip_broadcast: child_buff_skips_broadcast(child_buff_id),
    }
}

pub(super) struct LostHpCountAddBuffBefore;

impl BuffActionHandler for LostHpCountAddBuffBefore {
    type Params = LostHpCountAddBuffParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "LostHpCountAddBuff" && stage == BuffStage::BeforeBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        snapshot_params(parts, ctx)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        if params.skip_broadcast {
            return ActionResult::empty();
        }
        let mut effects = Vec::with_capacity(4);
        for _ in 0..2 {
            effects.push(ActEffect {
                effect_type: Some(EffectType::MaxHpChange as i32),
                target_id: Some(params.target_uid),
                effect_num: Some(params.max_hp),
                ..Default::default()
            });
            effects.push(ActEffect {
                effect_type: Some(EffectType::CurrentHpChange as i32),
                target_id: Some(params.target_uid),
                effect_num: Some(params.current_hp),
                ..Default::default()
            });
        }
        ActionResult::effects(effects)
    }
}

pub(super) struct LostHpCountAddBuffAfter;

impl BuffActionHandler for LostHpCountAddBuffAfter {
    type Params = LostHpCountAddBuffParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "LostHpCountAddBuff" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        snapshot_params(parts, ctx)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::single(crate::state::battle::fight_step::ActEffectBuilder::effect_none(params.target_uid))
    }
}


