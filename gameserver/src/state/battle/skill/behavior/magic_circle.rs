//! MagicCircle action — handler for the two behavior variants that
//! interact with the magic-circle system:
//!
//! * `AddMagicCircle { circle_id }` — summon a magic circle. Emits a
//!   `MagicCircleAdd` ActEffect carrying the circle config plus,
//!   optionally, a `BuffAdd` for the circle's `self_buff` (and a
//!   per-ally `CureUpByLostHp` pair when the self_buff features a
//!   CureUpByLostHp act). Delegates to `misc::add_magic_circle`.
//! * `MagicCircleAttr { .. }` — currently a no-op placeholder; the
//!   real attr-modification side of magic circles is handled by the
//!   buff_actions side. Delegates to `misc::magic_circle_attr`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::misc;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

pub(super) struct MagicCircle;

impl BehaviorAction for MagicCircle {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        match behavior {
            BehaviorType::AddMagicCircle { circle_id } => {
                misc::add_magic_circle(ctx.behavior_ctx.fight, ctx.caster_uid, *circle_id)
            }
            BehaviorType::MagicCircleAttr { .. } => misc::magic_circle_attr(),
            _ => Ok(vec![]),
        }
    }
}
