use sonettobuf::ActEffect;

use crate::state::battle::{types::effects::EffectType, utils::attr_update};

use super::EffectContext;
use super::result::ActionResult;

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
