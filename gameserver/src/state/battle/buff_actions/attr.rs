use sonettobuf::ActEffect;

use crate::state::battle::{types::effects::EffectType, utils::attr_update};

use super::EffectContext;
use super::action::{BuffAction, BuffActCtx, BuffStage};
use super::result::ActionResult;

/// Attributes buff_action — handles every attr-side buff_act type:
///
/// * `Attr` — both stages. Pre-stage emits `MaxHpChange(108)` /
///   `CurrentHpChange(109)` pairs (×2) when `char_attr_id == 101`
///   (HP) AND the buff was applied during EnterFight / BattleStart
///   (`condition_id == 5 || condition_id == 5021`). Post-stage emits
///   the standard `Attr(26)` marker.
/// * `EachChangeAttr` — both stages. Pre-stage emits a single
///   `MaxHp/CurrentHp` pair when `char_attr_id == 101` (the `new_max`
///   formula uses `caster_max_hp * source_rate / 1000` from
///   `parts[4]`). Post-stage emits a `None(0)` marker.
/// * `AttrFromEntity` — post-stage only; emits `Attr(26)` and
///   queues a self-delete of the buff that triggered it.
/// * `AttrOnlyCalDamageReplaceAttr` /
///   `AttrOnlyCalDamageReplaceAttrADCreator` — both stages no-op
///   (the actual damage-side replacement happens in the damage
///   pipeline; the buff_act presence is a marker only).
pub(super) struct Attributes;

impl BuffAction for Attributes {
    fn execute(
        &self,
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult> {
        match (act_type, stage) {
            ("Attr", BuffStage::BeforeBuffAdd) => Some(attr_before_apply(ctx, parts)),
            ("Attr", BuffStage::AfterBuffAdd) => Some(on_apply(ctx.effect_ctx)),
            ("EachChangeAttr", BuffStage::BeforeBuffAdd) => {
                Some(each_change_attr_before(ctx, parts))
            }
            ("EachChangeAttr", BuffStage::AfterBuffAdd) => {
                Some(ActionResult::none(ctx.effect_ctx.target))
            }
            ("AttrFromEntity", BuffStage::AfterBuffAdd) => {
                Some(from_entity(ctx.effect_ctx, ctx.buff_id))
            }
            ("AttrFromEntity", BuffStage::BeforeBuffAdd) => None,
            ("AttrOnlyCalDamageReplaceAttr", _)
            | ("AttrOnlyCalDamageReplaceAttrADCreator", _) => Some(ActionResult::empty()),
            _ => None,
        }
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
fn attr_before_apply(ctx: &mut BuffActCtx<'_, '_>, parts: &[&str]) -> ActionResult {
    let char_attr_id = parse_part(parts, 1);
    let rate = parse_part(parts, 2);
    if !(char_attr_id == 101
        && (ctx.condition_id == 5 || ctx.condition_id == 5021))
    {
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
fn each_change_attr_before(ctx: &mut BuffActCtx<'_, '_>, parts: &[&str]) -> ActionResult {
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
