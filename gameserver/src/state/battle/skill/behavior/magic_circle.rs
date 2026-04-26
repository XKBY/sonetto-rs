//! MagicCircle action — handler for the two behavior variants that
//! interact with the magic-circle / array-skill system:
//!
//! * `AddMagicCircle { circle_id }` — summon a magic circle. Emits a
//!   `MagicCircleAdd` ActEffect carrying the circle config plus,
//!   optionally, a `BuffAdd` for the circle's `self_buff` (and a
//!   per-ally `CureUpByLostHp` pair when the self_buff features a
//!   CureUpByLostHp act). Delegates to
//!   `mechanics::magic_circle::add_magic_circle`.
//! * `MagicCircleAttr { modifiers }` — array-skill attr aura. The
//!   variant's `modifiers` field carries `(side, attr_id, permille)`
//!   tuples parsed from `60076#side#attr#permille[#side2#attr2#permille2]`.
//!   The handler is **currently no-op pending fixture data**. Empirical
//!   testing against battle1 r1 with a per-target `Attr(26)` emission
//!   produced six extra effects vs LIVE — meaning LIVE does NOT emit
//!   per-target Attr(26) markers for this behavior. The correct
//!   emission shape is unknown until a fixture with active magic-circle
//!   /array-skill psychubes is captured. For now the handler preserves
//!   the parsed `modifiers` data structure so the future implementation
//!   can pick it up without re-deriving from raw parts.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::mechanics::magic_circle as magic_circle_mechanic;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

pub(super) struct MagicCircle;

impl BehaviorAction for MagicCircle {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::AddMagicCircle { circle_id } => {
                Some(magic_circle_mechanic::add_magic_circle(
                    ctx.behavior_ctx.fight,
                    ctx.caster_uid,
                    *circle_id,
                ))
            }
            // TODO: implement when a fixture with active magic-circle
            // psychubes is available to validate against. The
            // `modifiers` field is parsed but currently unused.
            BehaviorType::MagicCircleAttr { modifiers: _ } => Some(Ok(vec![])),
            _ => None,
        }
    }
}
