use sonettobuf::ActEffect;

use crate::state::battle::{types::effects::EffectType, utils::attr_update};

use super::EffectContext;
use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

pub(super) struct AttrBeforeParams {
    pub result: ActionResult,
}

pub(super) struct AttrAfterParams {
    pub result: ActionResult,
}

pub(super) struct EachChangeAttrBeforeParams {
    pub result: ActionResult,
}

pub(super) struct EachChangeAttrAfterParams {
    pub target_uid: i64,
}

pub(super) struct AttrFromEntityParams {
    pub result: ActionResult,
}

pub(super) struct AttrBeforeHandler;

impl BuffActionHandler for AttrBeforeHandler {
    type Params = AttrBeforeParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Attr" && stage == BuffStage::BeforeBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        AttrBeforeParams {
            result: attr_before_apply(ctx, parts),
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        params.result
    }
}

pub(super) struct AttrAfterHandler;

impl BuffActionHandler for AttrAfterHandler {
    type Params = AttrAfterParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Attr" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        AttrAfterParams {
            result: ActionResult::empty(),
        }
    }

    fn execute(&self, params: &mut Self::Params, ctx: &mut BuffActCtx<'_, '_>) {
        params.result = on_apply(ctx.effect_ctx);
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        params.result
    }
}

pub(super) struct EachChangeAttrBeforeHandler;

impl BuffActionHandler for EachChangeAttrBeforeHandler {
    type Params = EachChangeAttrBeforeParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "EachChangeAttr" && stage == BuffStage::BeforeBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        EachChangeAttrBeforeParams {
            result: each_change_attr_before(ctx, parts),
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        params.result
    }
}

pub(super) struct EachChangeAttrAfterHandler;

impl BuffActionHandler for EachChangeAttrAfterHandler {
    type Params = EachChangeAttrAfterParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "EachChangeAttr" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        EachChangeAttrAfterParams {
            target_uid: ctx.effect_ctx.target,
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::none(params.target_uid)
    }
}

pub(super) struct AttrFromEntityHandler;

impl BuffActionHandler for AttrFromEntityHandler {
    type Params = AttrFromEntityParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "AttrFromEntity" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        AttrFromEntityParams {
            result: ActionResult::empty(),
        }
    }

    fn execute(&self, params: &mut Self::Params, ctx: &mut BuffActCtx<'_, '_>) {
        params.result = from_entity(ctx.effect_ctx, ctx.buff_id);
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        params.result
    }
}

pub(super) struct AttrOnlyCalDamageHandler;

impl BuffActionHandler for AttrOnlyCalDamageHandler {
    type Params = ();

    fn matches(&self, act_type: &str, _stage: BuffStage) -> bool {
        matches!(
            act_type,
            "AttrOnlyCalDamageReplaceAttr" | "AttrOnlyCalDamageReplaceAttrADCreator"
        )
    }

    fn parse(&self, _parts: &[&str], _ctx: &BuffActCtx<'_, '_>) -> Self::Params {}

    fn steps(&self, _params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::empty()
    }
}

fn parse_part(parts: &[&str], idx: usize) -> i32 {
    parts
        .get(idx)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Pre-stage `Attr` HP broadcast: emit two `MaxHp/CurrentHp` pairs
/// when applying an HP-attr buff during EnterFight / BattleStart.
/// Strict to avoid career/unconditional attr adds emitting extra
/// pairs.
fn attr_before_apply(ctx: &BuffActCtx<'_, '_>, parts: &[&str]) -> ActionResult {
    let char_attr_id = parse_part(parts, 1);
    let rate = parse_part(parts, 2);
    if !(char_attr_id == 101 && (ctx.condition_id == 5 || ctx.condition_id == 5021)) {
        return ActionResult::empty();
    }
    let base_hp = ctx
        .effect_ctx
        .target_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let new_max = base_hp + base_hp * rate / 1000;
    let current_hp = ctx.effect_ctx.target_hp();
    let target_uid = ctx.effect_ctx.target_uid();
    let mut effects = Vec::new();
    for _ in 0..2 {
        effects.push(ActEffect {
            effect_type: Some(EffectType::MaxHpChange as i32),
            target_id: Some(target_uid),
            effect_num: Some(new_max),
            ..Default::default()
        });
        effects.push(ActEffect {
            effect_type: Some(EffectType::CurrentHpChange as i32),
            target_id: Some(target_uid),
            effect_num: Some(current_hp),
            ..Default::default()
        });
    }
    ActionResult::effects(effects)
}

/// Pre-stage `EachChangeAttr` HP broadcast: emit a single
/// `MaxHp/CurrentHp` pair when applying an HP-attr buff. The new
/// max scales by `caster_max_hp * source_rate / 1000`.
fn each_change_attr_before(ctx: &BuffActCtx<'_, '_>, parts: &[&str]) -> ActionResult {
    let char_attr_id = parse_part(parts, 1);
    let source_rate = parse_part(parts, 4);
    if char_attr_id != 101 {
        return ActionResult::empty();
    }
    let caster_max_hp = ctx
        .effect_ctx
        .caster_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let target_max_hp = ctx
        .effect_ctx
        .target_entity()
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let current_hp = ctx.effect_ctx.target_hp();
    let target_uid = ctx.effect_ctx.target_uid();
    let new_max = target_max_hp + caster_max_hp * source_rate / 1000;
    ActionResult::effects(vec![
        ActEffect {
            effect_type: Some(EffectType::MaxHpChange as i32),
            target_id: Some(target_uid),
            effect_num: Some(new_max),
            ..Default::default()
        },
        ActEffect {
            effect_type: Some(EffectType::CurrentHpChange as i32),
            target_id: Some(target_uid),
            effect_num: Some(current_hp),
            ..Default::default()
        },
    ])
}

/// Buff feature: Attr (post stage) — emit Attr(26) marker.
/// HP-specific MaxHp/CurrentHp broadcasts are handled in the before-add stage.
pub fn on_apply(ctx: &mut EffectContext) -> ActionResult {
    ActionResult::single(attr_update(ctx.target))
}

/// Buff feature: AttrFromEntity — replaces ATK with entity stat for one hit, then self-deletes.
pub fn from_entity(ctx: &mut EffectContext, buff_id: i32) -> ActionResult {
    ActionResult {
        effects: vec![ActEffect {
            effect_type: Some(EffectType::Attr as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }],
        buff_dels: vec![(ctx.target, buff_id)],
        ..Default::default()
    }
}
